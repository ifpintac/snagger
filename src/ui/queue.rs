//! The queue on the Downloads tab: filters, multi-select with bulk actions,
//! "when done", and one row per download.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, Align, Id, Key, Label, Layout, Sense, Ui, UiBuilder, vec2};

use super::downloads::Outcome;
use super::icons::{self, Icon};
use super::theme::{Tokens, tk};
use super::thumbs::Thumbs;
use super::widgets::{self, Button, Tone, chip, icon_button, menu_item};
use crate::hub::{Hub, WhenDone};
use crate::model::{Job, Stage, Status};
use crate::platform;
use crate::settings::{self, fmt_minute};
use crate::util;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Active,
    Done,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Act {
    Pause,
    Resume,
    Cancel,
    Redo,
    Remove,
    Top,
    Bottom,
    Open,
    Reveal,
    CopyLink,
    OpenPage,
    Log,
    UpdateRetry,
}

#[derive(Default)]
pub struct QueueUi {
    filter: Filter,
    selected: HashSet<String>,
    anchor: Option<String>,
    /// Rows waiting for "Remove N downloads?" to be answered, and the frame
    /// the question first showed.
    confirm_remove: Option<(Vec<String>, usize, u64)>,
}

const ROW_H: f32 = 72.0;

/// What the row displays, worked out once per frame.
struct RowCtx {
    queue_paused: bool,
    waiting_for: Option<u16>,
    now: u64,
}

/// Removing it throws away a partial download.
fn has_partial_data(j: &Job) -> bool {
    match j.status {
        Status::Running | Status::Paused => true,
        Status::Queued | Status::Failed => j.progress > 0.0,
        Status::Done | Status::Cancelled => false,
    }
}

impl QueueUi {
    pub fn show(&mut self, ui: &mut Ui, hub: &Hub, thumbs: &mut Thumbs) -> Option<Outcome> {
        let t = tk(ui);
        let mut acts: Vec<(String, Act)> = Vec::new();
        let mut header: Option<&'static str> = None;
        let mut parallel_change: Option<u32> = None;
        let mut when_done_change: Option<WhenDone> = None;
        let mut bulk: Option<Act> = None;
        let mut clicks: Vec<(usize, egui::Modifiers)> = Vec::new();
        let order: Vec<String>;
        {
            let c = hub.lock();
            let count = |f: &dyn Fn(Status) -> bool| c.jobs.iter().filter(|j| f(j.status)).count();
            let running = count(&|s| s == Status::Running);
            let waiting = count(&|s| s == Status::Queued);
            let paused = count(&|s| s == Status::Paused);
            let done = count(&|s| s == Status::Done || s == Status::Cancelled);
            let failed = count(&|s| s == Status::Failed);
            let partial = c.jobs.iter().filter(|j| j.status == Status::Done && j.items_failed > 0).count();

            ui.horizontal(|ui| {
                widgets::heading(ui, "Queue");
                let mut parts = Vec::new();
                for (n, word) in [
                    (running, "downloading"),
                    (waiting, "waiting"),
                    (paused, "paused"),
                    (done, "finished"),
                    (failed, "failed"),
                ] {
                    if n > 0 {
                        parts.push(format!("{n} {word}"));
                    }
                }
                ui.label(widgets::muted(ui, parts.join(" · ")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.filter == Filter::Failed {
                        if failed > 0 && Button::secondary("Clear failed").icon(Icon::Erase).show(ui).clicked() {
                            header = Some("clear-failed");
                        }
                    } else if done > 0 && Button::secondary("Clear finished").icon(Icon::Erase).show(ui).clicked() {
                        header = Some("clear");
                    }
                    if failed + partial > 0
                        && Button::secondary("Retry failed")
                            .icon(Icon::Retry)
                            .tip("Try every failed download again")
                            .show(ui)
                            .clicked()
                    {
                        header = Some("retry");
                    }
                    if c.queue_paused || paused > 0 {
                        if Button::secondary("Resume all").icon(Icon::Play).show(ui).clicked() {
                            header = Some("resume");
                        }
                    } else if running + waiting > 0
                        && Button::secondary("Pause all").icon(Icon::Pause).show(ui).clicked()
                    {
                        header = Some("pause");
                    }
                });
            });
            ui.add_space(t.s(2));

            let mut idx: Vec<usize> = (0..c.jobs.len())
                .filter(|&i| {
                    let s = c.jobs[i].status;
                    match self.filter {
                        Filter::All => true,
                        Filter::Active => s.is_active() || s == Status::Paused,
                        Filter::Done => s == Status::Done || s == Status::Cancelled,
                        Filter::Failed => s == Status::Failed || (s == Status::Done && c.jobs[i].items_failed > 0),
                    }
                })
                .collect();
            idx.sort_by_key(|&i| {
                let j = &c.jobs[i];
                let rank = match j.status {
                    Status::Running => 0,
                    Status::Queued => 1,
                    Status::Paused => 2,
                    Status::Failed => 3,
                    Status::Done | Status::Cancelled => 4,
                };
                (rank, if rank >= 3 { u64::MAX - j.finished } else { i as u64 })
            });
            order = idx.iter().map(|&i| c.jobs[i].id.clone()).collect();
            self.selected.retain(|id| order.contains(id));

            // The second row: filters, or (while rows are selected) what to do
            // with them. Same height either way, so the list never jumps.
            let row_h = t.ctrl + t.s(1);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), row_h), Layout::left_to_right(Align::Center), |ui| {
                if !self.selected.is_empty() {
                    let compact = ui.available_width() < 980.0;
                    ui.label(widgets::strong(ui, format!("{} selected", self.selected.len())).color(t.accent_text));
                    ui.add_space(t.s(2));
                    for (ic, label, a) in [
                        (Icon::Play, "Resume", Act::Resume),
                        (Icon::Pause, "Pause", Act::Pause),
                        (Icon::Retry, "Retry", Act::Redo),
                        (Icon::Up, "Download next", Act::Top),
                        (Icon::Trash, "Remove", Act::Remove),
                    ] {
                        let mut b = Button::secondary(label);
                        if !compact {
                            b = b.icon(ic);
                        }
                        if b.show(ui).clicked() {
                            bulk = Some(a);
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, Icon::Close, "Clear the selection (Esc)").clicked() {
                            self.selected.clear();
                        }
                        if Button::quiet("Select all").tip("Ctrl+A").show(ui).clicked() {
                            self.selected = order.iter().cloned().collect();
                        }
                        if icon_button(ui, Icon::Copy, "Copy their links").clicked() {
                            bulk = Some(Act::CopyLink);
                        }
                    });
                } else {
                    for (f, label, n) in [
                        (Filter::All, "All", c.jobs.len()),
                        (Filter::Active, "In progress", running + waiting + paused),
                        (Filter::Done, "Finished", done),
                        (Filter::Failed, "Failed", failed),
                    ] {
                        let note = if n > 0 { n.to_string() } else { String::new() };
                        if chip(ui, self.filter == f, label, &note).clicked() {
                            self.filter = f;
                            self.selected.clear();
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut p = c.settings.parallel;
                        ui.spacing_mut().item_spacing.x = t.s(1);
                        if ui.add_enabled_ui(p < 10, |ui| icon_button(ui, Icon::Plus, "More at once")).inner.clicked() {
                            p += 1;
                        }
                        ui.label(widgets::strong(ui, p.to_string()));
                        if ui.add_enabled_ui(p > 1, |ui| icon_button(ui, Icon::Minus, "Fewer at once")).inner.clicked()
                        {
                            p -= 1;
                        }
                        ui.label(widgets::field_label(ui, "Downloads at once"));
                        if p != c.settings.parallel {
                            parallel_change = Some(p);
                        }
                        ui.add_space(t.s(4));
                        ui.spacing_mut().item_spacing.x = t.s(2);
                        let mut wd = c.when_done;
                        egui::ComboBox::from_id_salt("when-done")
                            .selected_text(wd.label())
                            .width(170.0)
                            .show_ui(ui, |ui| {
                                for w in WhenDone::ALL {
                                    ui.selectable_value(&mut wd, w, w.label());
                                }
                            })
                            .response
                            .on_hover_text("Once everything in the queue has finished. Applies once.");
                        ui.label(widgets::field_label(ui, "When done"));
                        if wd != c.when_done {
                            when_done_change = Some(wd);
                        }
                    });
                }
            });
            ui.add_space(t.s(2));

            if idx.is_empty() {
                empty_state(ui, c.jobs.is_empty());
            } else {
                let minute = settings::local_minute();
                let rc = RowCtx {
                    queue_paused: c.queue_paused,
                    waiting_for: (!c.settings.in_window(minute) && !c.window_override)
                        .then_some(c.settings.window_start),
                    now: util::now(),
                };
                egui::ScrollArea::vertical().id_salt("queue").auto_shrink([false, false]).show_rows(
                    ui,
                    ROW_H,
                    idx.len(),
                    |ui, range| {
                        for vi in range {
                            let j = &c.jobs[idx[vi]];
                            let sel = self.selected.contains(&j.id);
                            let (a, row_clicked) = job_row(ui, j, thumbs, &rc, sel);
                            if let Some(a) = a {
                                acts.push((j.id.clone(), a));
                            }
                            if row_clicked {
                                clicks.push((vi, widgets::click_modifiers(ui)));
                            }
                        }
                    },
                );
            }
        }

        // Selection: click, Ctrl+click to add, Shift+click for a range.
        for (vi, m) in clicks {
            let id = order[vi].clone();
            if m.command || m.ctrl {
                if !self.selected.remove(&id) {
                    self.selected.insert(id.clone());
                }
            } else if m.shift {
                let a = self.anchor.as_ref().and_then(|a| order.iter().position(|x| x == a)).unwrap_or(vi);
                let (lo, hi) = if a <= vi { (a, vi) } else { (vi, a) };
                self.selected.extend(order[lo..=hi].iter().cloned());
            } else if self.selected.len() == 1 && self.selected.contains(&id) {
                self.selected.clear();
            } else {
                self.selected = HashSet::from([id.clone()]);
            }
            self.anchor = Some(id);
        }
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing && self.confirm_remove.is_none() {
            let (all, esc, del) = ui.input(|i| {
                (i.modifiers.command && i.key_pressed(Key::A), i.key_pressed(Key::Escape), i.key_pressed(Key::Delete))
            });
            if all && !order.is_empty() {
                self.selected = order.iter().cloned().collect();
            }
            if esc {
                self.selected.clear();
            }
            if del && !self.selected.is_empty() {
                bulk = Some(Act::Remove);
            }
        }

        let mut outcome = None;
        {
            let mut c = hub.lock();
            if let Some(p) = parallel_change {
                c.settings.parallel = p;
                c.dirty.settings = true;
            }
            if let Some(wd) = when_done_change {
                c.when_done = wd;
                if wd.is_power() {
                    c.toast(format!(
                        "When the queue finishes, Snagger will {} after a 1-minute warning.",
                        match wd {
                            WhenDone::Sleep => "put the PC to sleep",
                            _ => "shut down the PC",
                        }
                    ));
                }
            }
            if let Some(h) = header {
                match h {
                    "clear" => c.clear_finished(),
                    "clear-failed" => c.clear_failed(),
                    "retry" => {
                        let n = c.retry_failed();
                        outcome =
                            Some(Outcome::Toast(format!("Retrying {n} download{}", if n == 1 { "" } else { "s" })));
                    }
                    "resume" => c.resume_all(),
                    "pause" => c.pause_all(),
                    _ => {}
                }
            }
            if let Some(a) = bulk {
                let ids: Vec<String> = order.iter().filter(|id| self.selected.contains(*id)).cloned().collect();
                match a {
                    Act::CopyLink => {
                        let links: Vec<String> = ids.iter().filter_map(|id| c.job(id).map(|j| j.url.clone())).collect();
                        let text = links.join("\n");
                        c.clip_ignore = text.clone();
                        ui.ctx().copy_text(text);
                        outcome = Some(Outcome::Toast(format!("Copied {} links", links.len())));
                    }
                    Act::Remove => {
                        // Unfinished downloads lose their partial data: ask first.
                        let partial = ids.iter().filter(|id| c.job(id).is_some_and(has_partial_data)).count();
                        if partial > 0 {
                            let frame = ui.ctx().cumulative_frame_nr();
                            self.confirm_remove = Some((ids, partial, frame));
                        } else {
                            for id in &ids {
                                c.remove(id);
                            }
                            self.selected.clear();
                            outcome = Some(Outcome::Toast(format!("Removed {} from the list", ids.len())));
                        }
                    }
                    Act::Top => {
                        for id in ids.iter().rev() {
                            c.move_to_top(id);
                        }
                    }
                    Act::Redo => {
                        // Retry what didn't finish; finished downloads aren't
                        // fetched again (that's "Download again" in the menu).
                        let mut retried = 0;
                        let mut skipped = 0;
                        for id in &ids {
                            let Some(j) = c.job(id) else { continue };
                            let retry = match j.status {
                                Status::Failed | Status::Cancelled | Status::Paused => true,
                                Status::Done => j.items_failed > 0,
                                Status::Queued | Status::Running => false,
                            };
                            if retry {
                                c.resume(id);
                                retried += 1;
                            } else {
                                skipped += 1;
                            }
                        }
                        let plural = |n: usize| if n == 1 { "" } else { "s" };
                        outcome = Some(Outcome::Toast(match (retried, skipped) {
                            (0, _) => "Nothing to retry: those finished or are still running".to_string(),
                            (r, 0) => format!("Retrying {r} download{}", plural(r)),
                            (r, s) => {
                                format!("Retrying {r} download{}; skipped {s} that finished or are running", plural(r))
                            }
                        }));
                    }
                    Act::Pause => ids.iter().for_each(|id| c.pause(id)),
                    Act::Resume => ids.iter().for_each(|id| c.resume(id)),
                    _ => {}
                }
            }
        }

        if let Some(msg) = self.confirm_dialog(ui.ctx(), hub) {
            outcome = Some(Outcome::Toast(msg));
        }

        for (id, act) in acts {
            let mut c = hub.lock();
            match act {
                Act::Pause => c.pause(&id),
                Act::Resume => c.resume(&id),
                Act::Cancel => c.cancel(&id),
                Act::Redo => c.redo(&id),
                Act::Remove => c.remove(&id),
                Act::Top => c.move_to_top(&id),
                Act::Bottom => c.move_to_bottom(&id),
                Act::Log => outcome = Some(Outcome::ShowLog(id.clone())),
                Act::UpdateRetry => {
                    c.resume(&id);
                    drop(c);
                    // Downloads wait while the engine updates, then this one runs on the new yt-dlp.
                    crate::engine::check_updates(hub);
                    outcome = Some(Outcome::Toast("Updating yt-dlp, then trying again".into()));
                }
                Act::CopyLink | Act::OpenPage | Act::Open | Act::Reveal => {
                    let Some(j) = c.job(&id) else { continue };
                    let url = j.url.clone();
                    let files = j.files.clone();
                    let folder = j.folder.clone();
                    match act {
                        Act::CopyLink => {
                            c.clip_ignore = url.clone();
                            ui.ctx().copy_text(url);
                            outcome = Some(Outcome::Toast("Link copied".into()));
                        }
                        Act::OpenPage => {
                            drop(c);
                            platform::open_url(&url);
                        }
                        _ => {
                            drop(c);
                            if let Some(msg) = open_result(act, &files, folder.as_deref()) {
                                outcome = Some(Outcome::Toast(msg));
                            }
                        }
                    }
                }
            }
        }
        hub.wake();
        outcome
    }

    /// "Remove N downloads?" Returns a toast once removed.
    fn confirm_dialog(&mut self, ctx: &egui::Context, hub: &Hub) -> Option<String> {
        let (ids, partial, since) = self.confirm_remove.as_ref()?;
        let first_frame = *since == ctx.cumulative_frame_nr();
        let n = ids.len();
        let mut decided: Option<bool> = None;
        let modal = widgets::modal(ctx, Id::new("confirm-remove"), |ui| {
            let t = tk(ui);
            ui.set_width(400.0);
            widgets::heading(
                ui,
                &if n == 1 { "Remove this download?".to_string() } else { format!("Remove {n} downloads?") },
            );
            ui.add_space(t.s(2));
            let what = if *partial == n {
                if n == 1 { "It hasn't finished, so".to_string() } else { "None of them have finished, so".to_string() }
            } else if *partial == 1 {
                "One of them hasn't finished, so".to_string()
            } else {
                format!("{partial} of them haven't finished, so")
            };
            widgets::para(
                ui,
                format!("{what} the part downloaded so far is deleted. Finished files stay where they are."),
            );
            ui.add_space(t.s(6));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if Button::danger("Remove").icon(Icon::Trash).show(ui).clicked() {
                    decided = Some(true);
                }
                if widgets::button(ui, "Keep them").clicked() {
                    decided = Some(false);
                }
            });
        });
        if modal.should_close() && decided.is_none() && !first_frame {
            decided = Some(false);
        }
        let yes = decided?;
        let (ids, _, _) = self.confirm_remove.take()?;
        if !yes {
            return None;
        }
        let mut c = hub.lock();
        for id in &ids {
            c.remove(id);
        }
        self.selected.clear();
        Some(format!("Removed {} from the list", ids.len()))
    }
}

/// Opens or reveals a finished download. Returns a message when it can't.
fn open_result(act: Act, files: &[PathBuf], folder: Option<&std::path::Path>) -> Option<String> {
    let existing: Vec<&PathBuf> = files.iter().filter(|f| f.exists()).collect();
    if existing.is_empty() && folder.is_none_or(|f| !f.exists()) {
        return Some("That file has been moved or deleted.".into());
    }
    match act {
        Act::Open if existing.len() == 1 && files.len() == 1 => {
            platform::open_path(existing[0]);
        }
        Act::Reveal if files.len() == 1 && !existing.is_empty() => platform::reveal(existing[0]),
        _ => match folder.filter(|f| f.exists()) {
            Some(f) => {
                platform::open_path(f);
            }
            None => platform::reveal(existing[0]),
        },
    }
    None
}

fn empty_state(ui: &mut Ui, nothing_at_all: bool) {
    let t = tk(ui);
    ui.add_space(t.s(10));
    ui.vertical_centered(|ui| {
        icons::show(ui, Icon::Download, t.icon_size * 2.0, t.text2);
        ui.add_space(t.s(2));
        if nothing_at_all {
            widgets::heading(ui, "Nothing downloading yet");
            ui.add_space(t.s(1));
            for line in [
                "Paste a link above, or type words to search YouTube.",
                "Copy a link in your browser and Snagger offers to grab it.",
                "Drop a text file of links on this window, or use Add many.",
            ] {
                ui.label(widgets::muted(ui, line));
            }
        } else {
            ui.label(widgets::muted(ui, "Nothing in this list."));
        }
    });
}

/// One line in the queue. Returns what the user clicked, and whether the
/// row itself (not one of its buttons) was clicked.
fn job_row(ui: &mut Ui, j: &Job, thumbs: &mut Thumbs, rc: &RowCtx, selected: bool) -> (Option<Act>, bool) {
    let t = tk(ui);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    let inner = widgets::list_row(ui, rect, &resp, selected);
    let sub = widgets::sub_color(t, selected);
    let mut act = None;
    let is_sub = !j.opts.sub_id.is_empty();
    let audio = j.opts.preset.quality.is_audio();

    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let tex = thumbs.get(&j.thumb);
    let placeholder = if j.opts.direct {
        Icon::Page
    } else if is_sub || j.is_playlist() {
        Icon::Playlist
    } else if audio {
        Icon::Music
    } else {
        Icon::Video
    };
    widgets::thumb(&mut row, tex.as_ref(), vec2(96.0, 54.0), placeholder);
    row.add_space(t.s(3));

    // Buttons for this state.
    let mut buttons: Vec<(Icon, &str, Act)> = Vec::new();
    match j.status {
        Status::Running => {
            buttons.push((Icon::Pause, "Pause", Act::Pause));
            buttons.push((Icon::Close, "Cancel", Act::Cancel));
        }
        Status::Queued => {
            buttons.push((Icon::Up, "Download next", Act::Top));
            buttons.push((Icon::Pause, "Pause", Act::Pause));
            buttons.push((Icon::Close, "Remove", Act::Remove));
        }
        Status::Paused => {
            buttons.push((Icon::Play, "Resume", Act::Resume));
            buttons.push((Icon::Close, "Cancel", Act::Cancel));
        }
        Status::Done => {
            if j.items_failed > 0 {
                buttons.push((Icon::Retry, "Retry the ones that failed", Act::Resume));
            }
            if !j.files.is_empty() {
                buttons.push((Icon::Open, if j.files.len() == 1 { "Open" } else { "Open folder" }, Act::Open));
                buttons.push((Icon::Folder, "Show in folder", Act::Reveal));
            }
        }
        Status::Failed => {
            if !j.opts.direct && util::engine_problem(&j.error) {
                buttons.push((
                    Icon::Refresh,
                    "The site may have changed: update yt-dlp and try again",
                    Act::UpdateRetry,
                ));
            }
            buttons.push((Icon::Retry, "Try again", Act::Resume));
            buttons.push((Icon::Terminal, "Show log", Act::Log));
        }
        Status::Cancelled => {
            buttons.push((Icon::Retry, "Start over", Act::Resume));
            buttons.push((Icon::Close, "Remove", Act::Remove));
        }
    }
    let buttons_w = (buttons.len() as f32 + 1.0) * (t.ctrl + t.s(1));
    let text_w = (row.available_width() - buttons_w - t.s(3)).max(60.0);

    let (status_text, status_color, frac, tone, busy) = describe(j, t, rc);
    let status_color = if status_color == t.text2 { sub } else { status_color };
    row.allocate_ui_with_layout(vec2(text_w, inner.height()), Layout::top_down(Align::Min), |ui| {
        ui.set_width(text_w);
        ui.spacing_mut().item_spacing.y = 2.0;
        let title = j.display_title();
        ui.add(Label::new(widgets::strong(ui, &title)).truncate()).on_hover_text(&title);
        let kind = if j.opts.direct { "File".to_string() } else { j.opts.preset.label() };
        let mut meta = vec![j.site.clone(), kind];
        if is_sub {
            meta.push("Subscription".into());
        } else if j.n_items > 1 {
            meta.push(format!("{} videos", j.n_items));
        }
        if let Some((a, b)) = j.opts.clip {
            let end = if b > 0.0 { util::fmt_timestamp(b) } else { "end".into() };
            meta.push(format!("Clip {}–{end}", util::fmt_timestamp(a)));
        }
        meta.retain(|m| !m.is_empty());
        ui.add(Label::new(widgets::caption(ui, meta.join("  ·  ")).color(sub)).truncate());
        ui.add_space(2.0);
        widgets::progress(ui, text_w, frac, tone, busy);
        ui.add_space(2.0);
        let s = ui.add(Label::new(widgets::mono(ui, &status_text).color(status_color)).truncate());
        if j.status == Status::Failed || j.items_failed > 0 {
            s.on_hover_text(&j.error);
        } else if j.retry_pending(rc.now) {
            s.on_hover_text(&j.note);
        }
    });

    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = t.s(1);
        widgets::more_menu(ui, |ui| row_menu(ui, j, &mut act));
        for (ic, tip, a) in buttons.iter().rev() {
            if icon_button(ui, *ic, tip).clicked() {
                act = Some(*a);
            }
        }
    });

    if resp.double_clicked() && j.status == Status::Done {
        act = Some(Act::Open);
    }
    widgets::context_menu(&resp, |ui| row_menu(ui, j, &mut act));
    (act, resp.clicked() && !resp.double_clicked())
}

fn row_menu(ui: &mut Ui, j: &Job, act: &mut Option<Act>) {
    let mut item = |ui: &mut Ui, icon: Icon, text: &str, a: Act| {
        if menu_item(ui, Some(icon), text).clicked() {
            *act = Some(a);
        }
    };
    if j.status == Status::Queued {
        item(ui, Icon::Up, "Download next", Act::Top);
        item(ui, Icon::Down, "Move to the end", Act::Bottom);
    }
    if j.status.is_finished() {
        item(ui, Icon::Refresh, "Download again", Act::Redo);
    }
    item(ui, Icon::Copy, "Copy link", Act::CopyLink);
    item(ui, Icon::Globe, "Open the page", Act::OpenPage);
    item(ui, Icon::Terminal, "Show log", Act::Log);
    ui.separator();
    let remove = if j.status == Status::Running { "Cancel and remove" } else { "Remove from list" };
    item(ui, Icon::Trash, remove, Act::Remove);
}

/// Status line, its colour, how full the bar is, the bar's tone, and
/// whether progress is unknown.
fn describe(j: &Job, t: &Tokens, rc: &RowCtx) -> (String, egui::Color32, f32, Tone, bool) {
    let pct = |p: f32| format!("{}%", (p * 100.0).floor() as u32);
    let is_sub = !j.opts.sub_id.is_empty();
    match j.status {
        Status::Running => {
            let l = &j.live;
            let mut parts = Vec::new();
            let unknown_size = j.opts.direct && l.total.is_none();
            match l.stage {
                Stage::Starting => parts.push("Starting…".to_string()),
                Stage::Processing => {
                    parts.push(if l.phase.is_empty() { "Finishing up…".into() } else { format!("{}…", l.phase) });
                }
                Stage::Downloading => {
                    if is_sub {
                        parts.push(format!("Checking for new videos · {} downloaded", j.items_done));
                    } else if unknown_size {
                        parts.push(format!("{} so far", util::fmt_bytes(l.downloaded as f64)));
                    } else {
                        parts.push(pct(j.progress));
                        if j.n_items > 1 && l.item > 0 {
                            parts.push(format!("video {} of {}", l.item, j.n_items));
                        }
                    }
                    if !l.phase.is_empty() {
                        parts.push(l.phase.clone());
                    }
                    if let Some(s) = l.speed.filter(|s| *s > 0.0) {
                        parts.push(format!("{}/s", util::fmt_bytes(s)));
                    }
                    if let Some(e) = l.eta.filter(|_| !is_sub) {
                        parts.push(util::fmt_eta(e));
                    }
                    if let Some(total) = l.total.filter(|x| *x > 0) {
                        parts.push(format!(
                            "{} of {}",
                            util::fmt_bytes(l.downloaded as f64),
                            util::fmt_bytes(total as f64)
                        ));
                    }
                }
            }
            let busy = l.stage == Stage::Starting || l.stage == Stage::Processing || is_sub || unknown_size;
            (parts.join("  ·  "), t.text2, j.progress, Tone::Busy, busy)
        }
        Status::Queued if j.retry_pending(rc.now) => {
            let left = j.retry_at.saturating_sub(rc.now);
            let when = if left >= 120 { format!("{} min", left.div_ceil(60)) } else { format!("{left} s") };
            (format!("Trying again in {when} · {}", j.note), t.warning_text, j.progress, Tone::Paused, false)
        }
        Status::Queued => {
            let text = if rc.queue_paused {
                "Waiting · queue paused".to_string()
            } else if let Some(m) = rc.waiting_for {
                format!("Waiting for {} (download hours)", fmt_minute(m))
            } else {
                "Waiting".to_string()
            };
            (text, t.text2, j.progress, Tone::Idle, false)
        }
        Status::Paused => {
            let mut text = if j.progress > 0.0 { format!("Paused at {}", pct(j.progress)) } else { "Paused".into() };
            if !j.note.is_empty() {
                text = format!("{text} · {}", j.note);
            }
            (text, t.warning_text, j.progress, Tone::Paused, false)
        }
        Status::Done => {
            let text = if !j.note.is_empty() {
                j.note.clone()
            } else if is_sub {
                format!(
                    "{} new video{} · {}",
                    j.items_done,
                    if j.items_done == 1 { "" } else { "s" },
                    util::fmt_bytes(j.bytes as f64)
                )
            } else if j.files.len() == 1 {
                let name = j.files[0].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                format!("Saved · {} · {name}", util::fmt_bytes(j.bytes as f64))
            } else {
                let mut s = format!("Saved {} videos · {}", j.files.len(), util::fmt_bytes(j.bytes as f64));
                if j.items_failed > 0 {
                    s.push_str(&format!(" · {} couldn't be downloaded", j.items_failed));
                }
                s
            };
            let color = if j.items_failed > 0 { t.warning_text } else { t.success_text };
            (text, color, 1.0, Tone::Done, false)
        }
        Status::Failed => (j.error.clone(), t.danger_text, j.progress.max(0.03), Tone::Failed, false),
        Status::Cancelled => ("Cancelled".into(), t.text2, 0.0, Tone::Idle, false),
    }
}
