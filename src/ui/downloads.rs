//! The Downloads tab: the link box, the preview card, "Add many" and the queue.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui::{self, Align, Event, FontId, Id, Key, Label, Layout, RichText, Sense, Ui, UiBuilder, vec2};
use egui_phosphor::regular as icon;

use super::theme::{Pal, bold};
use super::thumbs::Thumbs;
use super::widgets::{self, chip, icon_button, small_muted};
use crate::hub::Hub;
use crate::model::{
    AudioFormat, Control, Job, JobOptions, PlaylistMode, Preset, Quality, STOP_CANCEL, Stage, Status, VideoFormat,
};
use crate::platform;
use crate::util;
use crate::ytdlp::{self, Probe};

pub enum Outcome {
    ShowLog(String),
    Toast(String),
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Filter {
    #[default]
    All,
    Active,
    Done,
    Failed,
}

#[derive(Default)]
pub struct DownloadsUi {
    link: String,
    focus_link: bool,
    look: Option<Look>,
    batch: Option<Batch>,
    filter: Filter,
}

struct Look {
    url: String,
    ctl: Arc<Control>,
    rx: Option<Receiver<Result<Probe, String>>>,
    state: LookState,
}

enum LookState {
    Loading,
    Failed(String),
    Ready(Box<Preview>),
}

struct Preview {
    p: Probe,
    /// What the preview picked on its own; only changes from it are remembered.
    initial: Preset,
    quality: Quality,
    video: VideoFormat,
    audio: AudioFormat,
    clip: bool,
    clip_from: String,
    clip_to: String,
    picked: Vec<bool>,
    already: Option<(u64, Option<PathBuf>)>,
}

struct Batch {
    text: String,
    preset: Preset,
    whole: bool,
    links: Vec<String>,
    parsed: String,
    /// The click that opens the dialog must not also close it.
    opened: Option<u64>,
}

#[derive(Clone, Copy)]
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
}

const ROW_H: f32 = 78.0;

impl DownloadsUi {
    // -- getting links in -----------------------------------------------------

    pub fn receive_links(&mut self, hub: &Hub, links: Vec<String>) {
        // Already being handled, so the clipboard bar needn't offer them too.
        {
            let mut c = hub.lock();
            c.clip_offer.retain(|l| !links.contains(l));
            c.handled.extend(links.iter().cloned());
            let excess = c.handled.len().saturating_sub(200);
            c.handled.drain(..excess);
        }
        match links.len() {
            0 => {}
            1 => {
                let url = links.into_iter().next().unwrap_or_default();
                self.link = url.clone();
                self.start_look(hub, url, PlaylistMode::Auto);
            }
            _ => self.open_batch(hub, links.join("\n")),
        }
    }

    fn receive_text(&mut self, hub: &Hub, text: &str) {
        let links = util::extract_links(text);
        if !links.is_empty() {
            self.receive_links(hub, links);
        } else if let Some(one) = util::parse_single_link(text) {
            self.receive_links(hub, vec![one]);
        } else if !text.trim().is_empty() {
            self.fail_look(
                text.trim(),
                "That doesn't look like a link. Copy the address of the video page and paste it here.",
            );
        }
    }

    fn fail_look(&mut self, url: &str, msg: &str) {
        self.cancel_look();
        self.look =
            Some(Look { url: url.into(), ctl: Hub::new_control(), rx: None, state: LookState::Failed(msg.into()) });
    }

    fn cancel_look(&mut self) {
        if let Some(old) = self.look.take() {
            old.ctl.request(STOP_CANCEL);
        }
    }

    fn start_look(&mut self, hub: &Hub, url: String, mode: PlaylistMode) {
        self.cancel_look();
        let (paths, settings) = {
            let c = hub.lock();
            (c.engine.paths(), c.settings.clone())
        };
        let Some(paths) = paths else {
            self.fail_look(&url, "Snag is still setting up its download engine. Try again in a moment.");
            return;
        };
        let ctl = Hub::new_control();
        let (tx, rx) = mpsc::channel();
        let (u, c2, h) = (url.clone(), ctl.clone(), hub.clone());
        thread::spawn(move || {
            let r = ytdlp::probe(&paths, &settings, &u, mode, &c2);
            let _ = tx.send(r);
            h.repaint();
        });
        self.look = Some(Look { url, ctl, rx: Some(rx), state: LookState::Loading });
    }

    pub fn poll(&mut self, hub: &Hub) -> Option<String> {
        let look = self.look.as_mut()?;
        let rx = look.rx.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(p)) => {
                look.rx = None;
                let c = hub.lock();
                let want = c.settings.preset;
                let already = c.already_downloaded(&p.vid, &p.url).map(|h| (h.finished, h.target()));
                let quality = pick_quality(&p, want.quality);
                look.state = LookState::Ready(Box::new(Preview {
                    picked: vec![true; p.entries.len()],
                    initial: Preset { quality, ..want },
                    quality,
                    video: want.video,
                    audio: want.audio,
                    clip: false,
                    clip_from: String::new(),
                    clip_to: String::new(),
                    already,
                    p,
                }));
            }
            Ok(Err(e)) => {
                look.rx = None;
                if e.is_empty() {
                    self.look = None;
                } else {
                    look.state = LookState::Failed(e);
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                look.rx = None;
                look.state = LookState::Failed("Something went wrong while reading that link.".into());
            }
        }
        None
    }

    /// Titles currently on screen in the preview (for loading script fonts).
    pub fn preview_texts(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match self.look.as_ref().map(|l| &l.state) {
            Some(LookState::Ready(pv)) => Box::new(
                std::iter::once(pv.p.title.as_str())
                    .chain(std::iter::once(pv.p.uploader.as_str()))
                    .chain(pv.p.entries.iter().take(500).map(|e| e.title.as_str())),
            ),
            _ => Box::new(std::iter::empty()),
        }
    }

    pub fn batch_open(&self) -> bool {
        self.batch.is_some()
    }

    fn open_batch(&mut self, hub: &Hub, text: String) {
        let c = hub.lock();
        self.batch = Some(Batch {
            text,
            preset: c.settings.preset,
            whole: c.settings.batch_playlists,
            links: Vec::new(),
            parsed: "\u{0}".into(),
            opened: None,
        });
    }

    fn import_file(&mut self, hub: &Hub) {
        let picked = rfd::FileDialog::new()
            .set_title("Add links from a file")
            .add_filter("Lists and pages", &["txt", "csv", "md", "html", "htm", "url", "json", "m3u", "m3u8"])
            .add_filter("All files", &["*"])
            .pick_file();
        let Some(path) = picked else { return };
        let Ok(bytes) = std::fs::read(&path) else { return };
        let text = String::from_utf8_lossy(&bytes);
        let links = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("url")) {
            util::links_from_url_file(&text)
        } else {
            util::extract_links(&text)
        };
        if links.is_empty() {
            self.fail_look(&path.display().to_string(), "No links found in that file.");
        } else {
            self.open_batch(hub, links.join("\n"));
        }
    }

    // -- drawing ----------------------------------------------------------------

    pub fn show(&mut self, ui: &mut Ui, hub: &Hub, thumbs: &mut Thumbs) -> Option<Outcome> {
        self.link_bar(ui, hub);
        ui.add_space(10.0);
        let warnings: Vec<&str> = {
            let c = hub.lock();
            if c.engine.ready && c.engine.busy.is_none() { c.engine.warnings() } else { Vec::new() }
        };
        if !warnings.is_empty() {
            let pal = Pal::of(ui);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icon::WARNING).color(pal.amber));
                ui.label(RichText::new(warnings.join(" ")).color(pal.amber).size(13.0));
                if ui.link("Install what's missing").clicked() {
                    crate::engine::install_missing(hub);
                }
            });
            ui.add_space(6.0);
        }
        let a = self.look_card(ui, hub, thumbs);
        let b = self.queue(ui, hub, thumbs);
        a.or(b)
    }

    fn link_bar(&mut self, ui: &mut Ui, hub: &Hub) {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 316.0).max(220.0);
            let edit = egui::TextEdit::singleline(&mut self.link)
                .hint_text(format!("{}   Paste a link to a video, playlist or channel", icon::LINK))
                .desired_width(w)
                .margin(vec2(10.0, 8.0))
                .font(FontId::proportional(15.0));
            let resp = ui.add(edit);
            if std::mem::take(&mut self.focus_link) {
                resp.request_focus();
            }
            let pasted = resp.changed() && ui.input(|i| i.events.iter().any(|e| matches!(e, Event::Paste(_))));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if pasted || enter {
                let text = std::mem::take(&mut self.link);
                self.receive_text(hub, &text);
            }
            let big = vec2(0.0, 34.0);
            if ui
                .add(egui::Button::new(format!("{}  Paste", icon::CLIPBOARD)).min_size(big))
                .on_hover_text("Paste a link from the clipboard (or press Ctrl+V anywhere)")
                .clicked()
            {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                self.receive_text(hub, &text);
            }
            if ui
                .add(egui::Button::new(format!("{}  Add many", icon::LIST_PLUS)).min_size(big))
                .on_hover_text("Paste a whole list of links at once")
                .clicked()
            {
                self.open_batch(hub, String::new());
            }
            if ui
                .add(egui::Button::new(format!("{}  Import", icon::FILE_TEXT)).min_size(big))
                .on_hover_text("Add every link in a text file, bookmark export or web page")
                .clicked()
            {
                self.import_file(hub);
            }
        });
    }

    fn look_card(&mut self, ui: &mut Ui, hub: &Hub, thumbs: &mut Thumbs) -> Option<Outcome> {
        let look = self.look.as_mut()?;
        let mut close = false;
        let mut retry: Option<(String, PlaylistMode)> = None;
        let mut download = false;
        let download_dir = hub.lock().settings.download_dir.clone();
        widgets::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let pal = Pal::of(ui);
            match &mut look.state {
                LookState::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading the link…");
                        ui.add(Label::new(small_muted(ui, util::short_url(&look.url))).truncate());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
                }
                LookState::Failed(msg) => {
                    ui.horizontal_top(|ui| {
                        ui.label(RichText::new(icon::WARNING_CIRCLE).size(20.0).color(pal.red));
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - 40.0);
                            ui.add(Label::new(msg.as_str()).wrap());
                            ui.add(Label::new(small_muted(ui, util::short_url(&look.url))).truncate());
                            ui.horizontal(|ui| {
                                if look.url.starts_with("http") && ui.button("Try again").clicked() {
                                    retry = Some((look.url.clone(), PlaylistMode::Auto));
                                }
                                if util::video_in_playlist(&look.url) && ui.button("Try the whole playlist").clicked() {
                                    let pl = util::playlist_url_of(&look.url).unwrap_or(look.url.clone());
                                    retry = Some((pl, PlaylistMode::Whole));
                                }
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if icon_button(ui, icon::X, "Close").clicked() {
                                close = true;
                            }
                        });
                    });
                }
                LookState::Ready(pv) => {
                    preview_body(ui, pv, &look.url, thumbs, &download_dir, &mut close, &mut download, &mut retry);
                }
            }
        });
        ui.add_space(12.0);
        if close {
            self.cancel_look();
            self.link.clear();
        }
        if let Some((url, mode)) = retry {
            self.link = url.clone();
            self.start_look(hub, url, mode);
        }
        if download {
            return self.download_preview(hub);
        }
        None
    }

    fn download_preview(&mut self, hub: &Hub) -> Option<Outcome> {
        let look = self.look.take()?;
        let LookState::Ready(pv) = look.state else { return None };
        let p = &pv.p;
        let picked: Vec<u32> = p.entries.iter().zip(&pv.picked).filter(|(_, on)| **on).map(|(e, _)| e.index).collect();
        let items = if p.is_playlist && picked.len() < p.entries.len() { ranges(&picked) } else { String::new() };
        let clip =
            if pv.clip && !p.is_playlist { clip_range(&pv.clip_from, &pv.clip_to, p.duration).ok() } else { None };
        let preset = Preset { quality: pv.quality, video: pv.video, audio: pv.audio };
        let opts = JobOptions {
            preset,
            playlist: if p.is_playlist { PlaylistMode::Whole } else { PlaylistMode::Single },
            items,
            clip,
            ..Default::default()
        };
        let mut job = Job::new(&p.url, opts);
        job.title = p.title.clone();
        job.thumb = p.thumb.clone();
        job.site = p.site.clone();
        job.uploader = p.uploader.clone();
        job.duration = p.duration;
        job.vid = p.vid.clone();
        job.n_items = if p.is_playlist { picked.len() as u32 } else { 1 };
        let title = job.display_title();
        {
            let mut c = hub.lock();
            let id = c.add_job(job);
            c.move_to_top(&id);
            // Remember what the user changed, like the original Snag did. A
            // fallback the preview picked by itself (say "Best" for a 480p
            // video) isn't a choice and mustn't replace the default.
            let before = c.settings.preset;
            let s = &mut c.settings.preset;
            if preset.quality != pv.initial.quality {
                s.quality = preset.quality;
            }
            if preset.video != pv.initial.video {
                s.video = preset.video;
            }
            if preset.audio != pv.initial.audio {
                s.audio = preset.audio;
            }
            if c.settings.preset != before {
                c.dirty.settings = true;
            }
        }
        hub.wake();
        self.link.clear();
        self.focus_link = true;
        Some(Outcome::Toast(format!("Downloading “{}”", util::ellipsize(&title, 60))))
    }

    // -- add many -----------------------------------------------------------------

    pub fn batch_dialog(&mut self, ctx: &egui::Context, hub: &Hub) {
        let Some(b) = self.batch.as_mut() else { return };
        let frame = ctx.cumulative_frame_nr();
        let first_frame = *b.opened.get_or_insert(frame) == frame;
        if b.parsed != b.text {
            b.links = util::extract_links(&b.text);
            b.parsed = b.text.clone();
        }
        let queued = {
            let c = hub.lock();
            b.links.iter().filter(|l| c.is_queued(l)).count()
        };
        let mut add = false;
        let mut cancel = false;
        let mut import = false;
        let modal = egui::Modal::new(Id::new("batch")).show(ctx, |ui| {
            ui.set_width(640.0);
            widgets::heading(ui, &format!("{}  Add many links", icon::LIST_PLUS));
            ui.label(small_muted(
                ui,
                "Paste links below: one per line, or any text that contains them (a list, a chat, a web page). Duplicates are skipped.",
            ));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(260.0).id_salt("batch-text").show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut b.text)
                        .desired_rows(12)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("https://www.youtube.com/watch?v=…\nhttps://www.tiktok.com/@…/video/…\nhttps://vimeo.com/…"),
                );
            });
            ui.add_space(4.0);
            let found = match b.links.len() {
                0 => "No links yet".to_string(),
                1 => "1 link found".to_string(),
                n => format!("{n} links found"),
            };
            let extra = if queued > 0 { format!(" · {queued} already in the queue (skipped)") } else { String::new() };
            ui.label(RichText::new(format!("{found}{extra}")).family(bold()));
            ui.add_space(8.0);
            preset_picker(ui, "batch", &mut b.preset);
            ui.checkbox(&mut b.whole, "When a link is a video inside a playlist, download the whole playlist");
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Import from file…", icon::FILE_TEXT)).clicked() {
                    import = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let n = b.links.len().saturating_sub(queued);
                    let label = if n == 1 { "Add 1 link".to_string() } else { format!("Add {n} links") };
                    if widgets::primary_button(ui, &label, n > 0).clicked() {
                        add = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        });
        if import {
            let picked = rfd::FileDialog::new()
                .add_filter("Lists and pages", &["txt", "csv", "md", "html", "htm", "url", "json"])
                .add_filter("All files", &["*"])
                .pick_file();
            if let Some(path) = picked
                && let Ok(bytes) = std::fs::read(&path)
            {
                let text = String::from_utf8_lossy(&bytes);
                let links = util::extract_links(&text);
                if !b.text.is_empty() && !b.text.ends_with('\n') {
                    b.text.push('\n');
                }
                b.text.push_str(&links.join("\n"));
            }
        }
        if add {
            let opts = JobOptions {
                preset: b.preset,
                playlist: if b.whole { PlaylistMode::Whole } else { PlaylistMode::Single },
                ..Default::default()
            };
            {
                let mut c = hub.lock();
                let (added, skipped) = c.add_links(&b.links, &opts);
                if c.settings.batch_playlists != b.whole {
                    c.settings.batch_playlists = b.whole;
                    c.dirty.settings = true;
                }
                let msg = match (added, skipped) {
                    (1, 0) => "Added 1 link to the queue".to_string(),
                    (n, 0) => format!("Added {n} links to the queue"),
                    (n, s) => format!("Added {n} links ({s} were already queued)"),
                };
                c.toast(msg);
            }
            hub.wake();
            self.batch = None;
        } else if cancel || (modal.should_close() && !first_frame) {
            self.batch = None;
        }
    }

    // -- the queue ------------------------------------------------------------------

    fn queue(&mut self, ui: &mut Ui, hub: &Hub, thumbs: &mut Thumbs) -> Option<Outcome> {
        let mut acts: Vec<(String, Act)> = Vec::new();
        let mut header: Option<&'static str> = None;
        let mut parallel_change: Option<u32> = None;
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
                if running > 0 {
                    parts.push(format!("{running} downloading"));
                }
                if waiting > 0 {
                    parts.push(format!("{waiting} waiting"));
                }
                if paused > 0 {
                    parts.push(format!("{paused} paused"));
                }
                if done > 0 {
                    parts.push(format!("{done} finished"));
                }
                if failed > 0 {
                    parts.push(format!("{failed} failed"));
                }
                ui.label(small_muted(ui, parts.join(" · ")));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.filter == Filter::Failed {
                        if failed > 0 && ui.button(format!("{}  Clear failed", icon::BROOM)).clicked() {
                            header = Some("clear-failed");
                        }
                    } else if done > 0 && ui.button(format!("{}  Clear finished", icon::BROOM)).clicked() {
                        header = Some("clear");
                    }
                    if failed + partial > 0
                        && ui
                            .button(format!("{}  Retry failed", icon::ARROW_CLOCKWISE))
                            .on_hover_text("Try every failed download again")
                            .clicked()
                    {
                        header = Some("retry");
                    }
                    if c.queue_paused || paused > 0 {
                        if ui.button(format!("{}  Resume all", icon::PLAY)).clicked() {
                            header = Some("resume");
                        }
                    } else if running + waiting > 0 && ui.button(format!("{}  Pause all", icon::PAUSE)).clicked() {
                        header = Some("pause");
                    }
                });
            });

            ui.horizontal(|ui| {
                for (f, label, n) in [
                    (Filter::All, "All", c.jobs.len()),
                    (Filter::Active, "In progress", running + waiting + paused),
                    (Filter::Done, "Finished", done),
                    (Filter::Failed, "Failed", failed),
                ] {
                    let note = if n > 0 { n.to_string() } else { String::new() };
                    if chip(ui, self.filter == f, label, &note).clicked() {
                        self.filter = f;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut p = c.settings.parallel;
                    if ui
                        .add_enabled(p < 10, egui::Button::new(icon::PLUS).small())
                        .on_hover_text("More at once")
                        .clicked()
                    {
                        p += 1;
                    }
                    ui.label(RichText::new(p.to_string()).family(bold()));
                    if ui
                        .add_enabled(p > 1, egui::Button::new(icon::MINUS).small())
                        .on_hover_text("Fewer at once")
                        .clicked()
                    {
                        p -= 1;
                    }
                    ui.label(small_muted(ui, "At once"));
                    if p != c.settings.parallel {
                        parallel_change = Some(p);
                    }
                });
            });
            ui.add_space(4.0);

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

            if idx.is_empty() {
                empty_state(ui, c.jobs.is_empty());
            } else {
                let queue_paused = c.queue_paused;
                egui::ScrollArea::vertical().id_salt("queue").auto_shrink([false, false]).show_rows(
                    ui,
                    ROW_H,
                    idx.len(),
                    |ui, range| {
                        for i in range {
                            let j = &c.jobs[idx[i]];
                            if let Some(a) = job_row(ui, j, thumbs, queue_paused) {
                                acts.push((j.id.clone(), a));
                            }
                        }
                    },
                );
            }
        }

        let mut outcome = None;
        if let Some(p) = parallel_change {
            let mut c = hub.lock();
            c.settings.parallel = p;
            c.dirty.settings = true;
        }
        if let Some(h) = header {
            let mut c = hub.lock();
            match h {
                "clear" => c.clear_finished(),
                "clear-failed" => c.clear_failed(),
                "retry" => {
                    let n = c.retry_failed();
                    outcome = Some(Outcome::Toast(format!("Retrying {n} download{}", if n == 1 { "" } else { "s" })));
                }
                "resume" => c.resume_all(),
                "pause" => c.pause_all(),
                _ => {}
            }
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
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        let pal = Pal::of(ui);
        ui.label(RichText::new(icon::TRAY_ARROW_DOWN).size(42.0).color(pal.muted));
        ui.add_space(6.0);
        if nothing_at_all {
            ui.label(RichText::new("Nothing downloading yet").family(bold()).size(16.0));
            ui.add_space(2.0);
            for line in [
                "Paste a link above, or press Ctrl+V anywhere in Snag.",
                "Copy a link in your browser and Snag offers to grab it.",
                "Drop a text file of links on this window, or use Add many.",
            ] {
                ui.label(small_muted(ui, line));
            }
        } else {
            ui.label(small_muted(ui, "Nothing in this list."));
        }
    });
}

fn pick_quality(p: &Probe, want: Quality) -> Quality {
    if p.audio_only {
        return Quality::Audio;
    }
    if p.is_playlist {
        return want;
    }
    match want {
        Quality::Max(h) => {
            if p.top > 0 && p.top <= h {
                return Quality::Best;
            }
            p.choices
                .iter()
                .map(|c| c.quality)
                .find(|q| matches!(q, Quality::Max(x) if *x <= h))
                .unwrap_or(Quality::Best)
        }
        other => other,
    }
}

/// [1,2,3,5,7,8] → "1-3,5,7-8"
fn ranges(idx: &[u32]) -> String {
    let mut out = Vec::new();
    let mut i = 0;
    while i < idx.len() {
        let start = idx[i];
        let mut end = start;
        while i + 1 < idx.len() && idx[i + 1] == end + 1 {
            i += 1;
            end = idx[i];
        }
        out.push(if start == end { start.to_string() } else { format!("{start}-{end}") });
        i += 1;
    }
    out.join(",")
}

/// Reads the trim fields. An empty "to" means the end of the video (0.0).
fn clip_range(from: &str, to: &str, duration: u64) -> Result<(f64, f64), &'static str> {
    let start =
        if from.trim().is_empty() { 0.0 } else { util::parse_timestamp(from).ok_or("Use times like 1:30 or 90")? };
    let end = if to.trim().is_empty() { 0.0 } else { util::parse_timestamp(to).ok_or("Use times like 1:30 or 90")? };
    if end > 0.0 && end <= start {
        return Err("The end has to come after the start");
    }
    if duration > 0 && start >= duration as f64 {
        return Err("The start is past the end of the video");
    }
    if start == 0.0 && end == 0.0 {
        return Err("Enter a start or an end time");
    }
    Ok((start, end))
}

pub fn preset_picker(ui: &mut Ui, id: &str, preset: &mut Preset) {
    ui.horizontal(|ui| {
        ui.label("Quality");
        egui::ComboBox::from_id_salt(format!("{id}-q"))
            .selected_text(preset.quality.long_label())
            .width(170.0)
            .show_ui(ui, |ui| {
                for q in Quality::ALL {
                    ui.selectable_value(&mut preset.quality, q, q.long_label());
                }
            });
        ui.add_space(8.0);
        if preset.quality.is_audio() {
            ui.label("Format");
            egui::ComboBox::from_id_salt(format!("{id}-a")).selected_text(preset.audio.label()).width(120.0).show_ui(
                ui,
                |ui| {
                    for a in AudioFormat::ALL {
                        ui.selectable_value(&mut preset.audio, a, format!("{} · {}", a.label(), a.hint()));
                    }
                },
            );
        } else {
            ui.label("Format");
            egui::ComboBox::from_id_salt(format!("{id}-v")).selected_text(preset.video.label()).width(120.0).show_ui(
                ui,
                |ui| {
                    for v in VideoFormat::ALL {
                        ui.selectable_value(&mut preset.video, v, format!("{} · {}", v.label(), v.hint()));
                    }
                },
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn preview_body(
    ui: &mut Ui,
    pv: &mut Preview,
    url: &str,
    thumbs: &mut Thumbs,
    download_dir: &std::path::Path,
    close: &mut bool,
    download: &mut bool,
    retry: &mut Option<(String, PlaylistMode)>,
) {
    let pal = Pal::of(ui);
    let p = &pv.p;
    ui.horizontal_top(|ui| {
        let tex = thumbs.get(&p.thumb);
        let placeholder = if p.is_playlist {
            icon::PLAYLIST
        } else if p.audio_only {
            icon::MUSIC_NOTES
        } else {
            icon::FILM_STRIP
        };
        widgets::thumb(ui, tex.as_ref(), vec2(192.0, 108.0), placeholder);
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 40.0).max(100.0));
            ui.add(Label::new(RichText::new(&p.title).family(bold()).size(16.5)).wrap());
            let mut meta = Vec::new();
            if p.is_playlist {
                meta.push(format!("Playlist · {} video{}", p.count, if p.count == 1 { "" } else { "s" }));
            }
            if !p.uploader.is_empty() {
                meta.push(p.uploader.clone());
            }
            if p.duration > 0 {
                meta.push(util::fmt_duration(p.duration));
            }
            if !p.site.is_empty() {
                meta.push(p.site.clone());
            }
            ui.label(small_muted(ui, meta.join("  ·  ")));
            if p.is_live {
                ui.label(
                    RichText::new("Live now: Snag records from this moment until the stream ends or you stop it.")
                        .color(pal.amber),
                );
            }
            if let Some((when, target)) = &pv.already {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{}  Downloaded {}", icon::CHECK_CIRCLE, util::fmt_ago(*when)))
                            .color(pal.green),
                    );
                    if let Some(t) = target
                        && ui.link("Show it").clicked()
                    {
                        platform::reveal(t);
                    }
                });
            }
            if !p.is_playlist && util::video_in_playlist(url) {
                ui.horizontal(|ui| {
                    ui.label(small_muted(ui, "This video is part of a playlist."));
                    if ui.link("Get the whole playlist").clicked() {
                        let pl = util::playlist_url_of(url).unwrap_or_else(|| url.to_string());
                        *retry = Some((pl, PlaylistMode::Whole));
                    }
                });
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if icon_button(ui, icon::X, "Close").clicked() {
                *close = true;
            }
        });
    });

    ui.add_space(10.0);
    ui.label(small_muted(ui, "Quality"));
    ui.horizontal_wrapped(|ui| {
        for ch in &pv.p.choices {
            let mut note = ch.note.clone();
            let size = if pv.video == VideoFormat::Mp4 { ch.size_mp4 } else { ch.size };
            if let Some(sz) = size {
                if !note.is_empty() {
                    note.push_str(" · ");
                }
                note.push_str(&format!("≈{}", util::fmt_bytes(sz as f64)));
            }
            let label = match ch.quality {
                Quality::Audio => "Audio only".to_string(),
                q => q.label(),
            };
            if chip(ui, pv.quality == ch.quality, &label, &note).clicked() {
                pv.quality = ch.quality;
            }
        }
    });
    ui.add_space(6.0);
    if pv.quality.is_audio() {
        ui.label(small_muted(ui, "Audio format"));
        ui.horizontal_wrapped(|ui| {
            for a in AudioFormat::ALL {
                if chip(ui, pv.audio == a, a.label(), "").on_hover_text(a.hint()).clicked() {
                    pv.audio = a;
                }
            }
        });
    } else {
        ui.label(small_muted(ui, "Video format"));
        ui.horizontal_wrapped(|ui| {
            for v in VideoFormat::ALL {
                if chip(ui, pv.video == v, v.label(), v.hint()).clicked() {
                    pv.video = v;
                }
            }
        });
    }

    let mut valid = true;
    if !pv.p.is_playlist {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut pv.clip, format!("{}  Only download part of it", icon::SCISSORS));
            if pv.clip {
                ui.label("from");
                ui.add(egui::TextEdit::singleline(&mut pv.clip_from).hint_text("0:00").desired_width(64.0));
                ui.label("to");
                let end_hint = if pv.p.duration > 0 { util::fmt_duration(pv.p.duration) } else { "end".into() };
                ui.add(egui::TextEdit::singleline(&mut pv.clip_to).hint_text(end_hint).desired_width(64.0));
                match clip_range(&pv.clip_from, &pv.clip_to, pv.p.duration) {
                    Ok((a, b)) => {
                        let end = if b > 0.0 { util::fmt_timestamp(b) } else { "the end".into() };
                        ui.label(small_muted(ui, format!("Saves {} to {end}", util::fmt_timestamp(a))));
                    }
                    Err(e) => {
                        valid = false;
                        ui.label(RichText::new(e).color(pal.red).size(12.5));
                    }
                }
            }
        });
    }

    let mut selected = pv.picked.iter().filter(|x| **x).count();
    if pv.p.is_playlist && !pv.p.entries.is_empty() {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(small_muted(ui, format!("Videos · {selected} of {} selected", pv.p.entries.len())));
            if ui.small_button("All").clicked() {
                pv.picked.iter_mut().for_each(|x| *x = true);
            }
            if ui.small_button("None").clicked() {
                pv.picked.iter_mut().for_each(|x| *x = false);
            }
        });
        egui::Frame::new().stroke(egui::Stroke::new(1.0, pal.border)).corner_radius(6).inner_margin(6).show(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("entries").max_height(190.0).auto_shrink([false, true]).show_rows(
                ui,
                22.0,
                pv.p.entries.len(),
                |ui, range| {
                    for i in range {
                        let e = &pv.p.entries[i];
                        ui.horizontal(|ui| {
                            ui.set_height(22.0);
                            ui.checkbox(&mut pv.picked[i], "");
                            ui.label(small_muted(ui, format!("{:>3}", e.index)));
                            let w = (ui.available_width() - 60.0).max(50.0);
                            ui.allocate_ui_with_layout(vec2(w, 20.0), Layout::left_to_right(Align::Center), |ui| {
                                ui.set_max_width(w);
                                ui.add(Label::new(e.title.as_str()).truncate());
                            });
                            if e.duration > 0 {
                                ui.label(small_muted(ui, util::fmt_duration(e.duration)));
                            }
                        });
                    }
                },
            );
        });
        selected = pv.picked.iter().filter(|x| **x).count();
        if selected == 0 {
            valid = false;
        }
    }

    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 190.0).max(80.0);
        ui.allocate_ui_with_layout(vec2(w, 32.0), Layout::left_to_right(Align::Center), |ui| {
            ui.set_max_width(w);
            ui.add(
                Label::new(small_muted(ui, format!("{}  Saves to {}", icon::FOLDER_SIMPLE, download_dir.display())))
                    .truncate(),
            );
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if pv.p.is_playlist {
                if selected == 1 { "Download 1 video".to_string() } else { format!("Download {selected} videos") }
            } else {
                format!("{}  Download", icon::DOWNLOAD_SIMPLE)
            };
            if widgets::primary_button(ui, &label, valid).clicked() {
                *download = true;
            }
        });
    });
}

/// One line in the queue. Returns what the user clicked.
fn job_row(ui: &mut Ui, j: &Job, thumbs: &mut Thumbs, queue_paused: bool) -> Option<Act> {
    let pal = Pal::of(ui);
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), 8, pal.card_hover);
    }
    let mut act = None;
    let is_sub = !j.opts.sub_id.is_empty();
    let audio = j.opts.preset.quality.is_audio();

    let inner = rect.shrink2(vec2(8.0, 8.0));
    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let tex = thumbs.get(&j.thumb);
    let placeholder = if is_sub || j.is_playlist() {
        icon::PLAYLIST
    } else if audio {
        icon::MUSIC_NOTES
    } else {
        icon::FILM_STRIP
    };
    widgets::thumb(&mut row, tex.as_ref(), vec2(100.0, 56.0), placeholder);
    row.add_space(10.0);

    // Buttons for this state.
    let mut buttons: Vec<(&str, &str, Act)> = Vec::new();
    match j.status {
        Status::Running => {
            buttons.push((icon::PAUSE, "Pause", Act::Pause));
            buttons.push((icon::X, "Cancel", Act::Cancel));
        }
        Status::Queued => {
            buttons.push((icon::ARROW_LINE_UP, "Download next", Act::Top));
            buttons.push((icon::PAUSE, "Pause", Act::Pause));
            buttons.push((icon::X, "Remove", Act::Remove));
        }
        Status::Paused => {
            buttons.push((icon::PLAY, "Resume", Act::Resume));
            buttons.push((icon::X, "Cancel", Act::Cancel));
        }
        Status::Done => {
            if j.items_failed > 0 {
                buttons.push((icon::ARROW_CLOCKWISE, "Retry the ones that failed", Act::Resume));
            }
            if !j.files.is_empty() {
                buttons.push((icon::PLAY_CIRCLE, if j.files.len() == 1 { "Open" } else { "Open folder" }, Act::Open));
                buttons.push((icon::FOLDER_OPEN, "Show in folder", Act::Reveal));
            }
        }
        Status::Failed => {
            buttons.push((icon::ARROW_CLOCKWISE, "Try again", Act::Resume));
            buttons.push((icon::TERMINAL_WINDOW, "Show log", Act::Log));
        }
        Status::Cancelled => {
            buttons.push((icon::ARROW_CLOCKWISE, "Start over", Act::Resume));
            buttons.push((icon::X, "Remove", Act::Remove));
        }
    }
    let buttons_w = (buttons.len() as f32 + 1.0) * 32.0;
    let text_w = (row.available_width() - buttons_w - 8.0).max(60.0);

    // Text column.
    let (status_text, status_color, frac, bar_color, busy) = describe(j, &pal, queue_paused);
    row.allocate_ui_with_layout(vec2(text_w, inner.height()), Layout::top_down(Align::Min), |ui| {
        ui.set_width(text_w);
        ui.spacing_mut().item_spacing.y = 3.0;
        let title = j.display_title();
        ui.add(Label::new(RichText::new(&title).family(bold()).size(14.5)).truncate()).on_hover_text(&title);
        let mut meta = vec![j.site.clone(), j.opts.preset.label()];
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
        ui.add(Label::new(small_muted(ui, meta.join("  ·  "))).truncate());
        widgets::bar(ui, text_w, frac, bar_color, busy);
        let s = ui.add(Label::new(RichText::new(&status_text).size(12.5).color(status_color)).truncate());
        if j.status == Status::Failed || j.items_failed > 0 {
            s.on_hover_text(&j.error);
        }
    });

    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let menu = ui.menu_button(RichText::new(icon::DOTS_THREE_VERTICAL).size(17.0), |ui| {
            row_menu(ui, j, &mut act);
        });
        menu.response.on_hover_text("More");
        for (ic, tip, a) in buttons.iter().rev() {
            if icon_button(ui, ic, tip).clicked() {
                act = Some(*a);
            }
        }
    });

    if resp.double_clicked() && j.status == Status::Done {
        act = Some(Act::Open);
    }
    resp.context_menu(|ui| row_menu(ui, j, &mut act));
    act
}

fn row_menu(ui: &mut Ui, j: &Job, act: &mut Option<Act>) {
    let mut item = |ui: &mut Ui, text: String, a: Act| {
        if ui.button(text).clicked() {
            *act = Some(a);
            ui.close();
        }
    };
    if j.status == Status::Queued {
        item(ui, format!("{}  Download next", icon::ARROW_LINE_UP), Act::Top);
        item(ui, format!("{}  Move to the end", icon::ARROW_LINE_DOWN), Act::Bottom);
    }
    if j.status.is_finished() {
        item(ui, format!("{}  Download again", icon::ARROWS_CLOCKWISE), Act::Redo);
    }
    item(ui, format!("{}  Copy link", icon::COPY), Act::CopyLink);
    item(ui, format!("{}  Open the page", icon::ARROW_SQUARE_OUT), Act::OpenPage);
    item(ui, format!("{}  Show log", icon::TERMINAL_WINDOW), Act::Log);
    ui.separator();
    let remove = if j.status == Status::Running { "Cancel and remove" } else { "Remove from list" };
    item(ui, format!("{}  {remove}", icon::TRASH), Act::Remove);
}

/// Status line, its colour, and how full the bar is.
fn describe(j: &Job, pal: &Pal, queue_paused: bool) -> (String, egui::Color32, f32, egui::Color32, bool) {
    let pct = |p: f32| format!("{}%", (p * 100.0).floor() as u32);
    let is_sub = !j.opts.sub_id.is_empty();
    match j.status {
        Status::Running => {
            let l = &j.live;
            let mut parts = Vec::new();
            match l.stage {
                Stage::Starting => parts.push("Starting…".to_string()),
                Stage::Processing => {
                    parts.push(if l.phase.is_empty() { "Finishing up…".into() } else { format!("{}…", l.phase) });
                }
                Stage::Downloading => {
                    if is_sub {
                        parts.push(format!("Checking for new videos · {} downloaded", j.items_done));
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
                    if let Some(t) = l.total.filter(|t| *t > 0) {
                        parts.push(format!(
                            "{} of {}",
                            util::fmt_bytes(l.downloaded as f64),
                            util::fmt_bytes(t as f64)
                        ));
                    }
                }
            }
            let busy = l.stage == Stage::Starting || l.stage == Stage::Processing || is_sub;
            (parts.join("  ·  "), pal.muted, j.progress, pal.accent, busy)
        }
        Status::Queued => {
            let text = if queue_paused { "Waiting · queue paused" } else { "Waiting" };
            (text.into(), pal.muted, j.progress, pal.track, false)
        }
        Status::Paused => {
            let text = if j.progress > 0.0 { format!("Paused at {}", pct(j.progress)) } else { "Paused".into() };
            (text, pal.amber, j.progress, pal.amber, false)
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
                let mut t = format!("Saved {} videos · {}", j.files.len(), util::fmt_bytes(j.bytes as f64));
                if j.items_failed > 0 {
                    t.push_str(&format!(" · {} couldn't be downloaded", j.items_failed));
                }
                t
            };
            let color = if j.items_failed > 0 { pal.amber } else { pal.green };
            (text, color, 1.0, pal.green, false)
        }
        Status::Failed => (j.error.clone(), pal.red, j.progress.max(0.03), pal.red, false),
        Status::Cancelled => ("Cancelled".into(), pal.muted, 0.0, pal.track, false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ranges() {
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1-3,5,7-8");
        assert_eq!(ranges(&[4]), "4");
    }

    #[test]
    fn clip_validation() {
        assert_eq!(clip_range("1:00", "2:00", 300), Ok((60.0, 120.0)));
        assert_eq!(clip_range("", "0:30", 300), Ok((0.0, 30.0)));
        assert!(clip_range("2:00", "1:00", 300).is_err());
        assert!(clip_range("", "", 300).is_err());
    }
}
