//! The Subscriptions tab: follow channels and playlists, and fetch only
//! what's new each time.

use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui::{self, Align, Label, Layout, RichText, Sense, Ui, UiBuilder, vec2};
use egui_phosphor::regular as icon;

use super::Confirm;
use super::downloads::preset_picker;
use super::theme::{Pal, bold};
use super::thumbs::Thumbs;
use super::widgets::{self, icon_button, small_muted};
use crate::hub::Hub;
use crate::model::{Preset, SubState, Subscription};
use crate::platform;
use crate::settings::{self, SubCheck};
use crate::util;
use crate::ytdlp;

pub struct SubsUi {
    url: String,
    preset: Option<Preset>,
    only_new: bool,
    adding: Option<Receiver<Result<String, String>>>,
    error: String,
}

impl Default for SubsUi {
    fn default() -> Self {
        SubsUi { url: String::new(), preset: None, only_new: true, adding: None, error: String::new() }
    }
}

#[derive(Clone, Copy)]
enum Act {
    Check,
    Folder,
    Copy,
    Page,
    ToggleAuto,
    Remove,
}

const ROW_H: f32 = 64.0;

impl SubsUi {
    /// Finished "follow" requests. Returns a message for a toast.
    pub fn poll(&mut self) -> Option<String> {
        let rx = self.adding.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(msg)) => {
                self.adding = None;
                self.url.clear();
                self.error.clear();
                Some(msg)
            }
            Ok(Err(e)) => {
                self.adding = None;
                self.error = e;
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.adding = None;
                None
            }
        }
    }

    fn follow(&mut self, hub: &Hub, url: String, preset: Preset) {
        let (paths, s) = {
            let c = hub.lock();
            if c.subs.iter().any(|x| x.url == url) {
                self.error = "You already follow that.".into();
                return;
            }
            (c.engine.paths(), c.settings.clone())
        };
        let Some(paths) = paths else {
            self.error = "Snag is still setting up its download engine.".into();
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.adding = Some(rx);
        self.error.clear();
        let only_new = self.only_new;
        let hub = hub.clone();
        thread::spawn(move || {
            let (title, thumb) = match ytdlp::playlist_title(&paths, &s, &url) {
                Ok(t) => t,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    hub.repaint();
                    return;
                }
            };
            let sub = Subscription {
                id: util::new_id(),
                url: url.clone(),
                folder: util::safe_name(&title, 80),
                title: title.clone(),
                thumb,
                preset,
                created: util::now(),
                auto: true,
                state: if only_new { SubState::Preparing } else { SubState::Idle },
                ..Default::default()
            };
            let id = sub.id.clone();
            {
                let mut c = hub.lock();
                c.subs.push(sub);
                c.dirty.subs = true;
            }
            hub.repaint();
            if only_new {
                let _ =
                    tx.send(Ok(format!("Following {title}. Getting its current videos so only new ones download…")));
                hub.repaint();
                let r = ytdlp::mark_existing(&paths, &s, &url, &settings::subs_archive_path());
                let mut c = hub.lock();
                if let Some(sub) = c.subs.iter_mut().find(|x| x.id == id) {
                    sub.state = SubState::Idle;
                    sub.last_checked = util::now();
                    match r {
                        Ok((n, _)) => {
                            sub.error.clear();
                            c.toast(format!("Following {title}. Skipped {n} existing videos; new ones will download."));
                        }
                        Err(e) => sub.error = e,
                    }
                    c.dirty.subs = true;
                }
            } else {
                let mut c = hub.lock();
                c.check_subscription(&id);
                drop(c);
                hub.wake();
                let _ = tx.send(Ok(format!("Following {title}. Downloading its videos…")));
            }
            hub.repaint();
        });
    }

    pub fn show(&mut self, ui: &mut Ui, hub: &Hub, thumbs: &mut Thumbs, confirm: &mut Option<Confirm>) {
        let pal = Pal::of(ui);
        widgets::heading(ui, "Subscriptions");
        ui.label(small_muted(
            ui,
            "Follow a channel or playlist and Snag downloads each new video into its own folder. It checks while Snag is open.",
        ));
        ui.add_space(8.0);

        let default_preset = hub.lock().settings.preset;
        self.preset.get_or_insert(default_preset);
        widgets::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 110.0).max(200.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.url)
                        .hint_text(format!(
                            "{}  Channel or playlist link, e.g. https://www.youtube.com/@channel",
                            icon::RSS
                        ))
                        .desired_width(w)
                        .margin(vec2(10.0, 7.0)),
                );
                let busy = self.adding.is_some();
                let url = util::parse_single_link(&self.url);
                if busy {
                    ui.spinner();
                } else if widgets::primary_button(ui, "Follow", url.is_some()).clicked()
                    && let Some(u) = url
                {
                    let p = self.preset.unwrap_or(default_preset);
                    self.follow(hub, u, p);
                }
            });
            ui.add_space(4.0);
            if let Some(p) = self.preset.as_mut() {
                preset_picker(ui, "sub", p);
            }
            ui.checkbox(&mut self.only_new, "Only new videos from now on (skip what's already there)");
            if !self.error.is_empty() {
                ui.label(RichText::new(&self.error).color(pal.red));
            }
        });
        ui.add_space(8.0);

        let mut acts: Vec<(String, Act)> = Vec::new();
        let mut check_all = false;
        let mut new_interval = None;
        {
            let c = hub.lock();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Following {}", c.subs.len())).family(bold()));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut interval = c.settings.sub_check;
                    egui::ComboBox::from_id_salt("sub-interval")
                        .selected_text(match interval {
                            SubCheck::Off => "Only when I ask",
                            SubCheck::Every6h => "Every 6 hours",
                            SubCheck::Daily => "Once a day",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut interval, SubCheck::Daily, "Once a day");
                            ui.selectable_value(&mut interval, SubCheck::Every6h, "Every 6 hours");
                            ui.selectable_value(&mut interval, SubCheck::Off, "Only when I ask");
                        });
                    ui.label(small_muted(ui, "Check"));
                    if interval != c.settings.sub_check {
                        new_interval = Some(interval);
                    }
                    if !c.subs.is_empty() && ui.button(format!("{}  Check all now", icon::ARROWS_CLOCKWISE)).clicked() {
                        check_all = true;
                    }
                });
            });
            ui.add_space(4.0);
            if c.subs.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icon::RSS).size(40.0).color(pal.muted));
                    ui.label(small_muted(ui, "Nothing followed yet."));
                });
            } else {
                let checking: Vec<String> = c
                    .jobs
                    .iter()
                    .filter(|j| {
                        !j.opts.sub_id.is_empty() && (j.status.is_active() || j.status == crate::model::Status::Paused)
                    })
                    .map(|j| j.opts.sub_id.clone())
                    .collect();
                egui::ScrollArea::vertical().id_salt("subs").auto_shrink([false, false]).show_rows(
                    ui,
                    ROW_H,
                    c.subs.len(),
                    |ui, range| {
                        for i in range {
                            let s = &c.subs[i];
                            let busy = checking.contains(&s.id);
                            if let Some(a) = sub_row(ui, s, busy, thumbs, &c.settings.download_dir) {
                                acts.push((s.id.clone(), a));
                            }
                        }
                    },
                );
            }
        }

        let mut c = hub.lock();
        if let Some(i) = new_interval {
            c.settings.sub_check = i;
            c.dirty.settings = true;
        }
        if check_all {
            let ids: Vec<String> = c.subs.iter().filter(|s| s.state == SubState::Idle).map(|s| s.id.clone()).collect();
            let n = ids.iter().filter(|id| c.check_subscription(id)).count();
            c.toast(format!("Checking {n} subscription{} for new videos", if n == 1 { "" } else { "s" }));
        }
        for (id, act) in acts {
            let Some(s) = c.subs.iter().find(|s| s.id == id).cloned() else { continue };
            match act {
                Act::Check => {
                    if c.check_subscription(&id) {
                        c.toast(format!("Checking {} for new videos", s.title));
                    }
                }
                Act::Folder => {
                    let dir = c.settings.download_dir.join(&s.folder);
                    let _ = std::fs::create_dir_all(&dir);
                    platform::open_path(&dir);
                }
                Act::Copy => {
                    c.clip_ignore = s.url.clone();
                    ui.ctx().copy_text(s.url.clone());
                }
                Act::Page => {
                    platform::open_url(&s.url);
                }
                Act::ToggleAuto => {
                    if let Some(x) = c.subs.iter_mut().find(|x| x.id == id) {
                        x.auto = !x.auto;
                    }
                    c.dirty.subs = true;
                }
                Act::Remove => *confirm = Some(Confirm::RemoveSub(id.clone(), s.title.clone())),
            }
        }
        drop(c);
        hub.wake();
    }
}

fn sub_row(ui: &mut Ui, s: &Subscription, busy: bool, thumbs: &mut Thumbs, dir: &std::path::Path) -> Option<Act> {
    let pal = Pal::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), 8, pal.card_hover);
    }
    let inner = rect.shrink2(vec2(8.0, 6.0));
    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let tex = thumbs.get(&s.thumb);
    widgets::thumb(&mut row, tex.as_ref(), vec2(86.0, 48.0), icon::RSS);
    row.add_space(10.0);
    let mut act = None;
    let text_w = (row.available_width() - 4.0 * 32.0).max(60.0);
    row.allocate_ui_with_layout(vec2(text_w, inner.height()), Layout::top_down(Align::Min), |ui| {
        ui.set_width(text_w);
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.add(Label::new(RichText::new(&s.title).family(bold()).size(14.5)).truncate());
        let status = if s.state == SubState::Preparing {
            "Getting the current video list…".to_string()
        } else if busy {
            "Checking for new videos…".to_string()
        } else if !s.error.is_empty() {
            s.error.clone()
        } else if s.last_checked == 0 {
            "Not checked yet".to_string()
        } else {
            format!(
                "Checked {} · {} new last time · {} in total",
                util::fmt_ago(s.last_checked),
                s.last_new,
                s.total_new
            )
        };
        let color = if !s.error.is_empty() && !busy { pal.red } else { pal.muted };
        let mut meta = vec![s.preset.label(), format!("→ {}", dir.join(&s.folder).display())];
        if !s.auto {
            meta.push("auto-check off".into());
        }
        ui.add(Label::new(small_muted(ui, meta.join("  ·  "))).truncate());
        ui.add(Label::new(RichText::new(status).size(12.5).color(color)).truncate()).on_hover_text(&s.url);
    });
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let menu = ui.menu_button(RichText::new(icon::DOTS_THREE_VERTICAL).size(17.0), |ui| {
            let auto = if s.auto { "Stop checking automatically" } else { "Check automatically" };
            for (text, a) in [
                (format!("{}  {auto}", icon::CLOCK_COUNTER_CLOCKWISE), Act::ToggleAuto),
                (format!("{}  Copy link", icon::COPY), Act::Copy),
                (format!("{}  Open the page", icon::ARROW_SQUARE_OUT), Act::Page),
                (format!("{}  Unfollow", icon::TRASH), Act::Remove),
            ] {
                if ui.button(text).clicked() {
                    act = Some(a);
                    ui.close();
                }
            }
        });
        menu.response.on_hover_text("More");
        if icon_button(ui, icon::FOLDER_OPEN, "Open its folder").clicked() {
            act = Some(Act::Folder);
        }
        let can_check = !busy && s.state == SubState::Idle;
        if can_check && icon_button(ui, icon::ARROWS_CLOCKWISE, "Check for new videos now").clicked() {
            act = Some(Act::Check);
        }
        if !can_check {
            ui.spinner();
        }
    });
    act
}
