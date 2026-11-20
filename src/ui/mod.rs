//! The window: tabs, status bar, notifications and the dialogs shared by
//! every tab.

mod downloads;
mod history;
mod icons;
mod picker;
mod queue;
mod settings_tab;
mod setup;
mod subs;
pub mod theme;
mod thumbs;
mod widgets;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Event, FontId, Id, Label, Layout, Rect, Sense, Stroke, Ui,
    ViewportCommand, vec2,
};

use crate::hub::Hub;
use crate::manager;
use crate::model::Status;
use crate::platform;
use crate::settings::ThemePref;
use crate::util;

use icons::Icon;
use theme::{ScriptFonts, bold, tk};
use thumbs::Thumbs;
use widgets::{Button, muted};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Downloads,
    History,
    Subscriptions,
    Settings,
}

pub enum Confirm {
    ClearHistory,
    ClearArchive,
    RemoveSub(String, String),
    Quit(usize),
}

pub struct SnagApp {
    hub: Hub,
    tab: Tab,
    thumbs: Thumbs,
    fonts: ScriptFonts,
    theme: Option<ThemePref>,
    title: String,
    toasts: Vec<(String, f64)>,
    dl: downloads::DownloadsUi,
    hist: history::HistoryUi,
    subs: subs::SubsUi,
    confirm: Option<Confirm>,
    /// Frame the current confirmation first showed (its opening click
    /// mustn't count as a click outside it).
    confirm_since: Option<u64>,
    log_for: Option<String>,
    allow_close: bool,
    free_space: Option<u64>,
    last_slow_tick: f64,
}

impl SnagApp {
    pub fn new(cc: &eframe::CreationContext<'_>, hub: Hub, links: Vec<String>) -> SnagApp {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(h) = cc.window_handle()
            && let RawWindowHandle::Win32(w) = h.as_raw()
        {
            platform::set_main_window(w.hwnd.get());
        }
        let ctx = &cc.egui_ctx;
        hub.set_ctx(ctx.clone());
        theme::install_fonts(ctx);
        hub.lock().inbox.extend(links);
        SnagApp {
            thumbs: Thumbs::new(ctx),
            hub,
            tab: Tab::Downloads,
            fonts: ScriptFonts::default(),
            theme: None,
            title: String::new(),
            toasts: Vec::new(),
            dl: downloads::DownloadsUi::default(),
            hist: history::HistoryUi::default(),
            subs: subs::SubsUi::default(),
            confirm: None,
            confirm_since: None,
            log_for: None,
            allow_close: false,
            free_space: None,
            last_slow_tick: -100.0,
        }
    }

    fn toast(&mut self, ctx: &egui::Context, msg: String) {
        let until = ctx.input(|i| i.time) + 4.5;
        self.toasts.push((msg, until));
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }
}

impl eframe::App for SnagApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.thumbs.poll(ctx);

        let (inbox, toasts, theme, title, any_active) = {
            let mut c = self.hub.lock();
            let inbox = std::mem::take(&mut c.inbox);
            let toasts = std::mem::take(&mut c.toasts);
            let running: Vec<f32> = c.jobs.iter().filter(|j| j.status == Status::Running).map(|j| j.progress).collect();
            let waiting = c.jobs.iter().filter(|j| j.status == Status::Queued).count();
            let title = if running.is_empty() {
                if waiting > 0 && c.queue_paused { "Snag · queue paused".to_string() } else { "Snag".to_string() }
            } else {
                let pct = (running.iter().sum::<f32>() / running.len() as f32 * 100.0).floor();
                format!("Snag · {} downloading · {pct}%", running.len())
            };
            let any_active = !running.is_empty() || c.engine.busy.is_some();
            (inbox, toasts, c.settings.theme, title, any_active)
        };

        if self.theme != Some(theme) {
            theme::apply_style(ctx, theme);
            self.theme = Some(theme);
        }
        if !inbox.is_empty() {
            self.tab = Tab::Downloads;
            self.dl.receive_links(&self.hub, inbox);
        }
        for t in toasts {
            self.toast(ctx, t);
        }
        if let Some(msg) = self.dl.poll(&self.hub) {
            self.toast(ctx, msg);
        }
        if let Some(msg) = self.subs.poll() {
            self.toast(ctx, msg);
        }

        // Files dropped on the window: link lists, .url shortcuts, web pages.
        let dropped: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| p.is_file()).collect());
        if !dropped.is_empty() {
            let mut links = Vec::new();
            for p in dropped {
                let Ok(bytes) = std::fs::read(&p) else { continue };
                if bytes.len() > 20_000_000 {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes);
                let is_url_file = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("url"));
                links.extend(if is_url_file { util::links_from_url_file(&text) } else { util::extract_links(&text) });
            }
            links.dedup();
            if links.is_empty() {
                self.toast(ctx, "No links found in that file.".into());
            } else {
                self.tab = Tab::Downloads;
                self.dl.receive_links(&self.hub, links);
            }
        }

        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        // Once a second or so: fonts for new scripts, free disk space.
        let now = ctx.input(|i| i.time);
        if now - self.last_slow_tick > 1.0 {
            let c = self.hub.lock();
            let titles = c
                .jobs
                .iter()
                .map(|j| j.title.as_str())
                // File names are shown in monospace and can hold glyphs a title doesn't (e.g. a full-width colon).
                .chain(c.jobs.iter().flat_map(|j| j.files.iter().filter_map(|f| f.file_name()?.to_str())))
                .chain(c.history.iter().take(300).map(|h| h.title.as_str()))
                .chain(c.subs.iter().map(|s| s.title.as_str()))
                .chain(self.dl.preview_texts());
            self.fonts.check(ctx, titles);
            if now - self.last_slow_tick > 10.0 || self.free_space.is_none() {
                self.free_space = platform::free_space(&c.settings.download_dir);
            }
            drop(c);
            self.last_slow_tick = now;
        }
        if any_active {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        widgets::track_input_mode(&ctx);
        self.handle_close(&ctx);
        self.global_paste(&ctx);
        let t = tk(ui);
        let fill = t.bg;
        let side = t.s(4) as i8;

        egui::Panel::top("top")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin { left: side, right: side, top: t.s(2) as i8, bottom: t.s(1) as i8 })
                    .fill(fill),
            )
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status")
            .show_separator_line(false)
            .frame(egui::Frame::new().inner_margin(egui::Margin::symmetric(side, t.s(2) as i8)).fill(fill))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin { left: side, right: side, top: t.s(1) as i8, bottom: 0 })
                    .fill(fill),
            )
            .show(ui, |ui| {
                widgets::work_area(ui, |ui| {
                    self.banners(ui);
                    self.clip_offer(ui);
                    match self.tab {
                        Tab::Downloads => {
                            let setting_up = {
                                let c = self.hub.lock();
                                c.engine.setting_up || (c.engine.ready && c.engine.ytdlp.is_none())
                            };
                            if setting_up {
                                setup::show(ui, &self.hub);
                            } else if let Some(out) = self.dl.show(ui, &self.hub, &mut self.thumbs) {
                                match out {
                                    downloads::Outcome::ShowLog(id) => self.log_for = Some(id),
                                    downloads::Outcome::Toast(msg) => self.toast(&ctx, msg),
                                }
                            }
                        }
                        Tab::History => {
                            if let Some(msg) = self.hist.show(ui, &self.hub, &mut self.thumbs, &mut self.confirm) {
                                self.toast(&ctx, msg);
                            }
                        }
                        Tab::Subscriptions => self.subs.show(ui, &self.hub, &mut self.thumbs, &mut self.confirm),
                        Tab::Settings => settings_tab::show(ui, &self.hub, &mut self.confirm),
                    }
                })
            });

        self.dl.batch_dialog(&ctx, &self.hub);
        self.confirm_dialog(&ctx);
        self.log_window(&ctx);
        self.draw_toasts(&ctx);
        self.drop_overlay(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        manager::save_all(&self.hub);
        crate::ipc::release();
    }
}

impl SnagApp {
    fn top_bar(&mut self, ui: &mut Ui) {
        let (n_active, n_failed, engine_label, engine_busy, engine_tip) = {
            let c = self.hub.lock();
            let active = c.jobs.iter().filter(|j| j.status.is_active()).count();
            let failed = c.jobs.iter().filter(|j| j.status == Status::Failed).count();
            let e = &c.engine;
            let (label, busy) = match &e.busy {
                Some(t) => (t.title.clone(), true),
                None if e.ytdlp.is_some() => (format!("yt-dlp {}", e.ytdlp_version), false),
                None if !e.ready => ("Starting…".to_string(), true),
                None => ("Engine missing".to_string(), false),
            };
            let mut tip = String::new();
            tip.push_str(&format!(
                "yt-dlp: {}\n",
                if e.ytdlp.is_some() { e.ytdlp_version.as_str() } else { "not installed" }
            ));
            tip.push_str(&format!(
                "ffmpeg: {}\n",
                match (&e.ffmpeg, e.ffmpeg_system) {
                    (Some(_), true) => "found on this PC",
                    (Some(_), false) => "installed",
                    (None, _) => "missing",
                }
            ));
            tip.push_str(&format!(
                "JavaScript: {}",
                match &e.js {
                    Some(j) if j.system => format!("{} (found on this PC)", j.kind),
                    Some(j) => format!("{} (installed)", j.kind),
                    None => "missing".into(),
                }
            ));
            (active, failed, label, busy, tip)
        };
        let t = tk(ui);
        ui.horizontal(|ui| {
            widgets::title(ui, "Snag");
            ui.add_space(t.s(6));
            let tabs = [
                (Tab::Downloads, Icon::Download, "Downloads"),
                (Tab::History, Icon::History, "History"),
                (Tab::Subscriptions, Icon::Feed, "Subscriptions"),
                (Tab::Settings, Icon::Settings, "Settings"),
            ];
            for (tab, ic, name) in tabs {
                let text = if tab == Tab::Downloads && n_active > 0 {
                    format!("{name}  {n_active}")
                } else {
                    name.to_string()
                };
                let r = widgets::tab(ui, self.tab == tab, ic, &text);
                let r = if tab == Tab::Downloads && n_failed > 0 {
                    // A small dot when something failed (the tooltip says how many).
                    let dot = Rect::from_center_size(r.rect.right_top() + vec2(-t.s(2), t.s(2)), vec2(6.0, 6.0));
                    ui.painter().rect_filled(dot.expand(1.5), CornerRadius::ZERO, t.bg);
                    ui.painter().rect_filled(dot, CornerRadius::ZERO, t.danger_text);
                    r.on_hover_text(format!("{n_failed} failed"))
                } else {
                    r
                };
                if r.clicked() {
                    self.tab = tab;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let color = if engine_busy {
                    t.warning_text
                } else if engine_label == "Engine missing" {
                    t.danger_text
                } else {
                    t.success_text
                };
                widgets::dot(ui, color);
                let l = ui.add(Label::new(widgets::mono(ui, engine_label)).sense(Sense::click()));
                if l.on_hover_text(engine_tip).clicked() {
                    self.tab = Tab::Settings;
                }
                if engine_busy {
                    ui.add(egui::Spinner::new().size(t.s(4)).color(t.text2));
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let (running, waiting, speed, paused_queue, dir, note) = {
            let c = self.hub.lock();
            let running = c.jobs.iter().filter(|j| j.status == Status::Running).count();
            let waiting = c.jobs.iter().filter(|j| j.status == Status::Queued).count();
            let speed: f64 = c.jobs.iter().filter(|j| j.status == Status::Running).filter_map(|j| j.live.speed).sum();
            let note = if !c.engine.error.is_empty() { c.engine.error.clone() } else { c.engine.note.clone() };
            (running, waiting, speed, c.queue_paused, c.settings.download_dir.clone(), note)
        };
        ui.horizontal(|ui| {
            let mut parts = Vec::new();
            if running > 0 {
                parts.push(format!("{running} downloading"));
                if speed > 0.0 {
                    parts.push(format!("{}/s", util::fmt_bytes(speed)));
                }
            }
            if waiting > 0 {
                parts.push(if paused_queue {
                    format!("{waiting} waiting (queue paused)")
                } else {
                    format!("{waiting} waiting")
                });
            }
            if parts.is_empty() {
                parts.push("Ready".into());
            }
            ui.label(widgets::mono(ui, parts.join("  ·  ")));
            if !note.is_empty() {
                ui.label(widgets::mono(ui, format!("  ·  {}", util::ellipsize(&note, 90))));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(free) = self.free_space {
                    ui.label(widgets::mono(ui, format!("{} free", util::fmt_bytes(free as f64))));
                    ui.label(widgets::mono(ui, "·"));
                }
                let shown = util::ellipsize(&dir.display().to_string(), 60);
                let r = ui.add(Label::new(widgets::mono(ui, shown)).sense(Sense::click()));
                let t = tk(ui);
                icons::show(ui, Icon::Folder, t.s(4), t.text2);
                if r.on_hover_text("Open the download folder").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    platform::open_path(&dir);
                }
            });
        });
    }

    /// Shown on every tab: a pending sleep/shut-down, and downloads waiting
    /// for the download hours.
    fn banners(&mut self, ui: &mut Ui) {
        let (power, waiting_until, disk_hold) = {
            let c = self.hub.lock();
            let waiting = c.jobs.iter().any(|j| j.status == Status::Queued);
            let blocked = !c.window_override && !c.settings.in_window(crate::settings::local_minute());
            let hold = c.disk_hold.then(|| (c.disk_free.unwrap_or(0), c.settings.download_dir.clone()));
            (c.power_at, (waiting && blocked).then_some(c.settings.window_start), hold)
        };
        let t = tk(ui);
        if let Some((free, dir)) = disk_hold {
            widgets::banner(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Warning, t.icon_size, t.warning_text);
                    ui.add(
                        Label::new(widgets::strong(
                            ui,
                            format!(
                                "Downloads are on hold: {} has only {} free. Free up some space and they start by themselves.",
                                util::drive_name(&dir),
                                util::fmt_bytes(free as f64)
                            ),
                        ))
                        .wrap(),
                    );
                });
            });
            ui.add_space(t.s(4));
        }
        if let Some((what, at)) = power {
            let left = at.saturating_sub(util::now());
            let mut cancel = false;
            widgets::banner(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Power, t.icon_size, t.warning_text);
                    let verb = if what == crate::hub::WhenDone::Sleep { "go to sleep" } else { "shut down" };
                    ui.label(widgets::strong(ui, format!("Downloads finished. The PC will {verb} in {left} s.")));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if Button::primary("Keep the PC on").show(ui).clicked() {
                            cancel = true;
                        }
                    });
                });
            });
            ui.add_space(t.s(4));
            if cancel {
                self.hub.lock().power_at = None;
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        if let Some(start) = waiting_until {
            let mut now = false;
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Clock, t.icon_size, t.text2);
                ui.label(muted(
                    ui,
                    format!(
                        "Downloads wait until {} (download hours in Settings).",
                        crate::settings::fmt_minute(start)
                    ),
                ));
                if Button::quiet("Start now anyway").show(ui).clicked() {
                    now = true;
                }
            });
            ui.add_space(t.s(2));
            if now {
                self.hub.lock().window_override = true;
                self.hub.wake();
            }
        }
    }

    /// "You copied a link" bar, shown on every tab.
    fn clip_offer(&mut self, ui: &mut Ui) {
        let offer = self.hub.lock().clip_offer.clone();
        if offer.is_empty() {
            return;
        }
        let t = tk(ui);
        let mut act = 0;
        widgets::banner(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Paste, t.icon_size, t.text2);
                let text = if offer.len() == 1 {
                    format!("You copied a link: {}", util::short_url(&offer[0]))
                } else {
                    format!("You copied {} links", offer.len())
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::icon_button(ui, Icon::Close, "Dismiss").clicked() {
                        act = 3;
                    }
                    if Button::secondary("Download")
                        .icon(Icon::Download)
                        .tip("Add to the queue with your default quality")
                        .show(ui)
                        .clicked()
                    {
                        act = 2;
                    }
                    if offer.len() == 1 && Button::secondary("Preview").icon(Icon::Eye).show(ui).clicked() {
                        act = 1;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(Label::new(widgets::strong(ui, text)).truncate());
                    });
                });
            });
        });
        ui.add_space(t.s(4));
        if act != 0 {
            let links = std::mem::take(&mut self.hub.lock().clip_offer);
            match act {
                1 => {
                    self.tab = Tab::Downloads;
                    self.dl.receive_links(&self.hub, links);
                }
                2 => {
                    let mut c = self.hub.lock();
                    let opts = c.settings.job_options();
                    let (n, _) = c.add_links(&links, &opts);
                    c.toast(if n == 1 {
                        "Added to the queue".to_string()
                    } else {
                        format!("Added {n} links to the queue")
                    });
                    drop(c);
                    self.hub.wake();
                }
                _ => {}
            }
        }
    }

    /// Ctrl+V anywhere (outside a text box) pastes a link.
    fn global_paste(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) || self.confirm.is_some() || self.dl.batch_open() {
            return;
        }
        let pasted =
            ctx.input(|i| i.events.iter().find_map(|e| if let Event::Paste(s) = e { Some(s.clone()) } else { None }));
        if let Some(text) = pasted {
            let links = util::extract_links(&text);
            if !links.is_empty() {
                self.tab = Tab::Downloads;
                self.dl.receive_links(&self.hub, links);
            } else if let Some(one) = util::parse_single_link(&text) {
                self.tab = Tab::Downloads;
                self.dl.receive_links(&self.hub, vec![one]);
            }
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        let running = self.hub.lock().jobs.iter().filter(|j| j.status == Status::Running).count();
        if running > 0 {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Quit(running));
        }
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(which) = &self.confirm else {
            self.confirm_since = None;
            return;
        };
        let frame = ctx.cumulative_frame_nr();
        let first_frame = *self.confirm_since.get_or_insert(frame) == frame;
        let destructive = !matches!(which, Confirm::Quit(_));
        let (title, body, ok) = match which {
            Confirm::ClearHistory => (
                "Clear history?".to_string(),
                "This empties the History list. Your downloaded files stay where they are.".to_string(),
                "Clear history",
            ),
            Confirm::ClearArchive => (
                "Forget downloaded videos?".to_string(),
                "Snag will no longer skip videos it downloaded before. Files aren't touched.".to_string(),
                "Forget",
            ),
            Confirm::RemoveSub(_, name) => (
                format!("Unfollow {name}?"),
                "Snag stops checking it for new videos. Files already downloaded stay.".to_string(),
                "Unfollow",
            ),
            Confirm::Quit(n) => (
                "Quit Snag?".to_string(),
                format!(
                    "{n} download{} running. {} pause now and continue the next time you open Snag.",
                    if *n == 1 { " is" } else { "s are" },
                    if *n == 1 { "It will" } else { "They will" }
                ),
                "Quit",
            ),
        };
        let mut decided: Option<bool> = None;
        let modal = widgets::modal(ctx, Id::new("confirm"), |ui| {
            let t = tk(ui);
            ui.set_width(400.0);
            widgets::heading(ui, &title);
            ui.add_space(t.s(2));
            widgets::para(ui, body);
            ui.add_space(t.s(6));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let b = if destructive { Button::danger(ok) } else { Button::primary(ok) };
                if b.show(ui).clicked() {
                    decided = Some(true);
                }
                if widgets::button(ui, "Cancel").clicked() {
                    decided = Some(false);
                }
            });
        });
        if modal.should_close() && decided.is_none() && !first_frame {
            decided = Some(false);
        }
        let Some(yes) = decided else { return };
        let which = self.confirm.take();
        self.confirm_since = None;
        if !yes {
            return;
        }
        match which {
            Some(Confirm::ClearHistory) => {
                let mut c = self.hub.lock();
                c.history.clear();
                c.dirty.history = true;
            }
            Some(Confirm::ClearArchive) => {
                let _ = std::fs::remove_file(crate::settings::archive_path());
                self.toast(ctx, "Snag forgot which videos it downloaded before.".into());
            }
            Some(Confirm::RemoveSub(id, _)) => self.hub.lock().remove_subscription(&id),
            Some(Confirm::Quit(_)) => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            None => {}
        }
    }

    fn log_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.log_for.clone() else { return };
        let (title, lines, url) = {
            let c = self.hub.lock();
            match c.job(&id) {
                Some(j) => (j.display_title(), j.log.join("\n"), j.url.clone()),
                None => {
                    self.log_for = None;
                    return;
                }
            }
        };
        let mut open = true;
        let tt = theme::tk_ctx(ctx);
        let log_title =
            egui::RichText::new("LOG").family(bold()).size(tt.fs_xs).extra_letter_spacing(tt.tracking(tt.fs_xs));
        egui::Window::new(log_title)
            .id(Id::new("log-window"))
            .open(&mut open)
            .default_size(vec2(720.0, 420.0))
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(widgets::strong(ui, &title));
                ui.label(muted(ui, &url));
                ui.horizontal(|ui| {
                    if Button::secondary("Copy log").icon(Icon::Copy).show(ui).clicked() {
                        self.hub.lock().clip_ignore = lines.clone();
                        ui.ctx().copy_text(format!("{title}\n{url}\n\n{lines}"));
                    }
                });
                ui.separator();
                egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    let text = if lines.is_empty() {
                        "Nothing logged yet. The log fills in while the download runs.".to_string()
                    } else {
                        lines.clone()
                    };
                    ui.add(egui::Label::new(egui::RichText::new(text).monospace()).wrap());
                });
            });
        if !open {
            self.log_for = None;
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.toasts.retain(|(_, until)| *until > now);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(Id::new("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                let t = tk(ui);
                for (msg, _) in &self.toasts {
                    widgets::toast_frame(ui, |ui| {
                        ui.set_max_width(380.0);
                        ui.add(Label::new(egui::RichText::new(msg).color(t.text)).wrap());
                    });
                    ui.add_space(t.s(2));
                }
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn drop_overlay(&self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let t = theme::tk_ctx(ctx);
        let rect = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, Id::new("drop")));
        painter.rect_filled(rect, CornerRadius::ZERO, t.bg.gamma_multiply(0.94));
        painter.add(widgets::clipped_shape(
            rect.shrink(t.s(4)),
            t.s(4),
            Color32::TRANSPARENT,
            Stroke::new(t.bw, t.accent),
        ));
        icons::paint_at(&painter, Icon::Download, rect.center() - vec2(0.0, t.s(8)), t.icon_size * 2.0, t.text);
        let size = t.fs_xl * t.heading_scale;
        let job = egui::text::LayoutJob::single_section(
            "DROP TO ADD THE LINKS IN THIS FILE".into(),
            egui::TextFormat {
                font_id: FontId::new(size, theme::head()),
                extra_letter_spacing: t.tracking(size),
                color: t.text,
                ..Default::default()
            },
        );
        let g = painter.layout_job(job);
        painter.galley(rect.center() + vec2(-g.size().x / 2.0, t.s(2) - g.size().y / 2.0), g, t.text);
    }
}
