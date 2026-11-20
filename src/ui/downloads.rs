//! The Downloads tab: the link box (a link, or words to search for), the
//! preview card (a video, a playlist, search results or a plain file),
//! "Add many", and the queue below.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use eframe::egui::{self, Align, Event, Id, Key, Label, Layout, Ui, vec2};

use super::icons::{self, Icon};
use super::picker::Picker;
use super::queue::QueueUi;
use super::theme::tk;
use super::thumbs::Thumbs;
use super::widgets::{self, Button, chip, icon_button, muted};
use crate::direct;
use crate::hub::Hub;
use crate::model::{AudioFormat, Control, Job, JobOptions, PlaylistMode, Preset, Quality, STOP_CANCEL, VideoFormat};
use crate::platform;
use crate::util;
use crate::ytdlp::{self, Entry, Probe, SearchSite};

pub enum Outcome {
    ShowLog(String),
    Toast(String),
}

#[derive(Default)]
pub struct DownloadsUi {
    link: String,
    focus_link: bool,
    look: Option<Look>,
    batch: Option<Batch>,
    queue: QueueUi,
    /// Search results to go back to after previewing one of them.
    back: Option<Box<Results>>,
    site: SearchSite,
}

/// Whatever is being looked at above the queue.
struct Look {
    /// The link, or the search words.
    what: String,
    search: Option<(SearchSite, u32)>,
    ctl: Arc<Control>,
    rx: Option<Receiver<Result<Found, String>>>,
    state: LookState,
}

enum Found {
    Media(Probe),
    Results(Vec<Entry>),
    File(direct::Probe),
}

enum LookState {
    Loading(String),
    Failed(String),
    Ready(Box<Preview>),
    Results(Box<Results>),
    File(Box<direct::Probe>),
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
    picker: Picker,
    already: Option<(u64, Option<PathBuf>)>,
}

struct Results {
    query: String,
    site: SearchSite,
    count: u32,
    picker: Picker,
    preset: Preset,
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
                self.back = None;
                self.start_look(hub, url, PlaylistMode::Auto);
            }
            _ => self.open_batch(hub, links.join("\n")),
        }
    }

    /// A link (or several) gets a preview; anything else is a search.
    fn receive_text(&mut self, hub: &Hub, text: &str) {
        let links = util::extract_links(text);
        if !links.is_empty() {
            self.receive_links(hub, links);
        } else if let Some(one) = util::parse_single_link(text) {
            self.receive_links(hub, vec![one]);
        } else if !text.trim().is_empty() {
            let site = self.site;
            self.start_search(hub, text.trim().to_string(), site, 30);
        }
    }

    fn fail_look(&mut self, what: &str, msg: &str) {
        self.cancel_look();
        self.look = Some(Look {
            what: what.into(),
            search: None,
            ctl: Hub::new_control(),
            rx: None,
            state: LookState::Failed(msg.into()),
        });
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
        let file_first = util::file_like_url(&url);
        if paths.is_none() && !file_first {
            self.fail_look(&url, "Snag is still setting up its download engine. Try again in a moment.");
            return;
        }
        let ctl = Hub::new_control();
        let (tx, rx) = mpsc::channel();
        let (u, c2, h) = (url.clone(), ctl.clone(), hub.clone());
        thread::spawn(move || {
            let probe_media = || match &paths {
                Some(p) => ytdlp::probe(p, &settings, &u, mode, &c2).map(Found::Media),
                None => Err("Snag is still setting up its download engine. Try again in a moment.".into()),
            };
            let r = if file_first {
                // Looks like a file (…/plans.zip): Snag's own engine, unless it's really a page.
                match direct::inspect(&u) {
                    Ok(f) if !f.is_page() => Ok(Found::File(f)),
                    _ => probe_media(),
                }
            } else {
                match probe_media() {
                    // yt-dlp doesn't know the site: maybe it's a plain file link.
                    Err(e) if e.starts_with("Snag can't find a video") => match direct::inspect(&u) {
                        Ok(f) if !f.is_page() => Ok(Found::File(f)),
                        _ => Err(e),
                    },
                    other => other,
                }
            };
            let _ = tx.send(r);
            h.repaint();
        });
        self.look = Some(Look {
            what: url,
            search: None,
            ctl,
            rx: Some(rx),
            state: LookState::Loading("Reading the link…".into()),
        });
    }

    fn start_search(&mut self, hub: &Hub, query: String, site: SearchSite, count: u32) {
        self.cancel_look();
        self.back = None;
        self.site = site;
        let (paths, settings) = {
            let c = hub.lock();
            (c.engine.paths(), c.settings.clone())
        };
        let Some(paths) = paths else {
            self.fail_look(&query, "Snag is still setting up its download engine. Try again in a moment.");
            return;
        };
        self.link = query.clone();
        let ctl = Hub::new_control();
        let (tx, rx) = mpsc::channel();
        let (q, c2, h) = (query.clone(), ctl.clone(), hub.clone());
        thread::spawn(move || {
            let _ = tx.send(ytdlp::search(&paths, &settings, &q, site, count, &c2).map(Found::Results));
            h.repaint();
        });
        self.look = Some(Look {
            what: query,
            search: Some((site, count)),
            ctl,
            rx: Some(rx),
            state: LookState::Loading(format!("Searching {}…", site.label())),
        });
    }

    pub fn poll(&mut self, hub: &Hub) -> Option<String> {
        let look = self.look.as_mut()?;
        let rx = look.rx.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(found)) => {
                look.rx = None;
                let c = hub.lock();
                let want = c.settings.preset;
                look.state = match found {
                    Found::Media(p) => {
                        let already = c.already_downloaded(&p.vid, &p.url).map(|h| (h.finished, h.target()));
                        let quality = pick_quality(&p, want.quality);
                        LookState::Ready(Box::new(Preview {
                            picker: Picker::new(p.entries.clone(), true),
                            initial: Preset { quality, ..want },
                            quality,
                            video: want.video,
                            audio: want.audio,
                            clip: false,
                            clip_from: String::new(),
                            clip_to: String::new(),
                            already,
                            p,
                        }))
                    }
                    Found::Results(entries) if entries.is_empty() => {
                        LookState::Failed(format!("No results for “{}”. Try other words.", look.what))
                    }
                    Found::Results(entries) => {
                        let (site, count) = look.search.unwrap_or_default();
                        LookState::Results(Box::new(Results {
                            query: look.what.clone(),
                            site,
                            count,
                            picker: Picker::new(entries, false),
                            preset: want,
                        }))
                    }
                    Found::File(f) => LookState::File(Box::new(f)),
                };
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
            Some(LookState::Results(r)) => {
                Box::new(r.picker.entries.iter().flat_map(|e| [e.title.as_str(), e.uploader.as_str()]))
            }
            Some(LookState::File(f)) => Box::new(std::iter::once(f.name.as_str())),
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
        let t = tk(ui);
        self.link_bar(ui, hub);
        ui.add_space(t.s(4));
        let warnings: Vec<&str> = {
            let c = hub.lock();
            if c.engine.ready && c.engine.busy.is_none() { c.engine.warnings() } else { Vec::new() }
        };
        if !warnings.is_empty() {
            ui.horizontal_wrapped(|ui| {
                icons::show(ui, Icon::Warning, t.icon_size, t.warning_text);
                ui.label(widgets::colored(ui, warnings.join(" "), t.warning_text));
                if Button::quiet("Install what's missing").show(ui).clicked() {
                    crate::engine::install_missing(hub);
                }
            });
            ui.add_space(t.s(2));
        }
        let a = self.look_card(ui, hub, thumbs);
        let b = self.queue.show(ui, hub, thumbs);
        a.or(b)
    }

    fn link_bar(&mut self, ui: &mut Ui, hub: &Hub) {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 372.0).max(220.0);
            let edit = egui::TextEdit::singleline(&mut self.link)
                .hint_text(format!("Paste a link, or type to search {}", self.site.label()));
            let resp = widgets::input(ui, edit, Some(Icon::Link), w);
            if std::mem::take(&mut self.focus_link) {
                resp.request_focus();
            }
            let pasted = resp.changed() && ui.input(|i| i.events.iter().any(|e| matches!(e, Event::Paste(_))));
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if pasted || enter {
                let text = std::mem::take(&mut self.link);
                self.receive_text(hub, &text);
            }
            if Button::secondary("Paste")
                .icon(Icon::Paste)
                .tip("Paste a link from the clipboard (or press Ctrl+V anywhere)")
                .show(ui)
                .clicked()
            {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                self.receive_text(hub, &text);
            }
            if Button::secondary("Add many")
                .icon(Icon::AddMany)
                .tip("Paste a whole list of links at once")
                .show(ui)
                .clicked()
            {
                self.open_batch(hub, String::new());
            }
            if Button::secondary("Import")
                .icon(Icon::Import)
                .tip("Add every link in a text file, bookmark export or web page")
                .show(ui)
                .clicked()
            {
                self.import_file(hub);
            }
        });
    }

    fn look_card(&mut self, ui: &mut Ui, hub: &Hub, thumbs: &mut Thumbs) -> Option<Outcome> {
        let look = self.look.as_mut()?;
        let t = tk(ui);
        let mut close = false;
        let mut retry: Option<(String, PlaylistMode)> = None;
        let mut download = false;
        let mut search: Option<(String, SearchSite, u32)> = None;
        let mut preview_entry: Option<String> = None;
        let mut back = false;
        let has_back = self.back.is_some();
        let download_dir = hub.lock().settings.download_dir.clone();
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            match &mut look.state {
                LookState::Loading(msg) => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(t.icon_size).color(t.text2));
                        ui.label(widgets::strong(ui, msg.as_str()));
                        ui.add(Label::new(muted(ui, util::short_url(&look.what))).truncate());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if widgets::button(ui, "Cancel").clicked() {
                                close = true;
                            }
                        });
                    });
                }
                LookState::Failed(msg) => {
                    ui.horizontal_top(|ui| {
                        icons::show(ui, Icon::Alert, t.icon_size, t.danger_text);
                        ui.vertical(|ui| {
                            ui.set_width(ui.available_width() - t.ctrl - t.s(2));
                            ui.add(Label::new(widgets::strong(ui, msg.as_str())).wrap());
                            ui.add(Label::new(muted(ui, util::short_url(&look.what))).truncate());
                            ui.add_space(t.s(2));
                            ui.horizontal(|ui| {
                                if Button::secondary("Try again").icon(Icon::Refresh).show(ui).clicked() {
                                    match look.search {
                                        Some((site, count)) => search = Some((look.what.clone(), site, count)),
                                        None => retry = Some((look.what.clone(), PlaylistMode::Auto)),
                                    }
                                }
                                if util::video_in_playlist(&look.what)
                                    && Button::secondary("Try the whole playlist")
                                        .icon(Icon::Playlist)
                                        .show(ui)
                                        .clicked()
                                {
                                    let pl = util::playlist_url_of(&look.what).unwrap_or(look.what.clone());
                                    retry = Some((pl, PlaylistMode::Whole));
                                }
                            });
                        });
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            if icon_button(ui, Icon::Close, "Close").clicked() {
                                close = true;
                            }
                        });
                    });
                }
                LookState::Ready(pv) => {
                    if has_back && Button::quiet("Back to the results").icon(Icon::Back).show(ui).clicked() {
                        back = true;
                    }
                    preview_body(ui, pv, &look.what, thumbs, &download_dir, &mut close, &mut download, &mut retry);
                }
                LookState::Results(r) => {
                    let act = results_body(ui, r, thumbs, &download_dir);
                    match act {
                        ResultsAct::None => {}
                        ResultsAct::Close => close = true,
                        ResultsAct::Download => download = true,
                        ResultsAct::Search(site, count) => search = Some((r.query.clone(), site, count)),
                        ResultsAct::Preview(i) => preview_entry = Some(r.picker.entries[i].url.clone()),
                    }
                }
                LookState::File(f) => {
                    file_body(ui, f, &look.what, &download_dir, &mut close, &mut download);
                }
            }
        });
        ui.add_space(t.s(5));

        if let Some(url) = preview_entry {
            if let Some(Look { state: LookState::Results(r), .. }) = self.look.take() {
                self.back = Some(r);
            }
            self.link = url.clone();
            self.start_look(hub, url, PlaylistMode::Single);
            return None;
        }
        if let Some((q, site, count)) = search {
            self.start_search(hub, q, site, count);
            return None;
        }
        if back
            || (close
                && self.back.is_some()
                && matches!(self.look.as_ref().map(|l| &l.state), Some(LookState::Ready(_))))
        {
            self.restore_results();
            return None;
        }
        if close {
            self.cancel_look();
            self.back = None;
            self.link.clear();
        }
        if let Some((url, mode)) = retry {
            self.link = url.clone();
            self.start_look(hub, url, mode);
        }
        if download {
            return match self.look.as_ref().map(|l| &l.state) {
                Some(LookState::Results(_)) => self.download_results(hub),
                Some(LookState::File(_)) => self.download_file(hub),
                _ => self.download_preview(hub),
            };
        }
        None
    }

    fn restore_results(&mut self) {
        self.cancel_look();
        if let Some(r) = self.back.take() {
            self.link = r.query.clone();
            self.look = Some(Look {
                what: r.query.clone(),
                search: Some((r.site, r.count)),
                ctl: Hub::new_control(),
                rx: None,
                state: LookState::Results(r),
            });
        }
    }

    fn download_preview(&mut self, hub: &Hub) -> Option<Outcome> {
        let look = self.look.take()?;
        let LookState::Ready(pv) = look.state else { return None };
        let p = &pv.p;
        let picked: Vec<u32> = pv.picker.selected().iter().map(|&i| pv.picker.entries[i].index).collect();
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
        if self.back.is_some() {
            self.restore_results();
        } else {
            self.focus_link = true;
        }
        Some(Outcome::Toast(format!("Downloading “{}”", util::ellipsize(&title, 60))))
    }

    fn download_results(&mut self, hub: &Hub) -> Option<Outcome> {
        let look = self.look.as_mut()?;
        let LookState::Results(r) = &mut look.state else { return None };
        let picked = r.picker.selected();
        let opts = JobOptions { preset: r.preset, playlist: PlaylistMode::Single, ..Default::default() };
        let mut added = 0;
        {
            let mut c = hub.lock();
            for &i in &picked {
                let e = &r.picker.entries[i];
                if c.is_queued(&e.url) {
                    continue;
                }
                let mut job = Job::new(&e.url, opts.clone());
                job.title = e.title.clone();
                job.thumb = e.thumb.clone();
                job.uploader = e.uploader.clone();
                job.duration = e.duration;
                job.site = r.site.label().to_string();
                c.add_job(job);
                added += 1;
            }
        }
        // Untick them, so the next picks start fresh.
        picked.iter().for_each(|&i| r.picker.picked[i] = false);
        hub.wake();
        Some(Outcome::Toast(match added {
            0 => "Those are already in the queue".to_string(),
            1 => "Downloading 1 video".to_string(),
            n => format!("Downloading {n} videos"),
        }))
    }

    fn download_file(&mut self, hub: &Hub) -> Option<Outcome> {
        let look = self.look.take()?;
        let LookState::File(f) = look.state else { return None };
        let mut job = Job::new(&look.what, JobOptions { direct: true, ..Default::default() });
        job.title = f.name.clone();
        job.site = util::host_of(&look.what);
        {
            let mut c = hub.lock();
            let id = c.add_job(job);
            c.move_to_top(&id);
        }
        hub.wake();
        self.link.clear();
        self.focus_link = true;
        Some(Outcome::Toast(format!("Downloading “{}”", util::ellipsize(&f.name, 60))))
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
        let files = b.links.iter().filter(|l| util::file_like_url(l)).count();
        let mut add = false;
        let mut cancel = false;
        let mut import = false;
        let modal = widgets::modal(ctx, Id::new("batch"), |ui| {
            let t = tk(ui);
            ui.set_width(640.0);
            widgets::icon_heading(ui, Icon::AddMany, "Add many links");
            ui.add_space(t.s(1));
            widgets::hint(
                ui,
                "Paste links below: one per line, or any text that contains them (a list, a chat, a web page). Duplicates are skipped.",
            );
            ui.add_space(t.s(3));
            let edit = egui::TextEdit::multiline(&mut b.text)
                .font(egui::TextStyle::Monospace)
                .hint_text("https://www.youtube.com/watch?v=…\nhttps://www.tiktok.com/@…/video/…\nhttps://vimeo.com/…");
            let w = ui.available_width();
            let area = widgets::text_area(ui, edit, vec2(w, 240.0), "batch-text");
            if first_frame {
                area.request_focus();
            }
            ui.add_space(t.s(2));
            let found = match b.links.len() {
                0 => "No links yet".to_string(),
                1 => "1 link found".to_string(),
                n => format!("{n} links found"),
            };
            let mut extra = String::new();
            if files > 0 {
                extra.push_str(&format!(
                    " · {files} plain file{} (Snag's own engine)",
                    if files == 1 { "" } else { "s" }
                ));
            }
            if queued > 0 {
                extra.push_str(&format!(" · {queued} already in the queue (skipped)"));
            }
            ui.label(widgets::strong(ui, format!("{found}{extra}")));
            ui.add_space(t.s(3));
            preset_picker(ui, "batch", &mut b.preset);
            ui.add_space(t.s(1));
            widgets::checkbox(
                ui,
                &mut b.whole,
                "When a link is a video inside a playlist, download the whole playlist",
            );
            ui.add_space(t.s(5));
            ui.horizontal(|ui| {
                if Button::secondary("Import from file…").icon(Icon::Import).show(ui).clicked() {
                    import = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let n = b.links.len().saturating_sub(queued);
                    let label = if n == 1 { "Add 1 link".to_string() } else { format!("Add {n} links") };
                    if Button::primary(&label).icon(Icon::Plus).enabled(n > 0).show(ui).clicked() {
                        add = true;
                    }
                    if widgets::button(ui, "Cancel").clicked() {
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
                // Plain file links go to Snag's own engine, the rest to yt-dlp.
                let (file_links, media): (Vec<String>, Vec<String>) =
                    b.links.iter().cloned().partition(|l| util::file_like_url(l));
                let (mut added, mut skipped) = c.add_links(&media, &opts);
                let file_opts = JobOptions { direct: true, ..Default::default() };
                let (fa, fs) = c.add_links(&file_links, &file_opts);
                added += fa;
                skipped += fs;
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
    let t = tk(ui);
    ui.horizontal(|ui| {
        ui.label(widgets::field_label(ui, "Quality"));
        egui::ComboBox::from_id_salt(format!("{id}-q"))
            .selected_text(preset.quality.long_label())
            .width(170.0)
            .show_ui(ui, |ui| {
                for q in Quality::ALL {
                    ui.selectable_value(&mut preset.quality, q, q.long_label());
                }
            });
        ui.add_space(t.s(3));
        ui.label(widgets::field_label(ui, "Format"));
        if preset.quality.is_audio() {
            egui::ComboBox::from_id_salt(format!("{id}-a")).selected_text(preset.audio.label()).width(120.0).show_ui(
                ui,
                |ui| {
                    for a in AudioFormat::ALL {
                        ui.selectable_value(&mut preset.audio, a, format!("{} · {}", a.label(), a.hint()));
                    }
                },
            );
        } else {
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

/// "Saves to …" on the left, the primary action on the right.
fn footer(ui: &mut Ui, download_dir: &std::path::Path, label: &str, icon: Option<Icon>, enabled: bool) -> bool {
    let t = tk(ui);
    let mut clicked = false;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 230.0).max(80.0);
        ui.allocate_ui_with_layout(vec2(w, t.ctrl), Layout::left_to_right(Align::Center), |ui| {
            ui.set_max_width(w);
            icons::show(ui, Icon::Folder, t.icon_size, t.text2);
            ui.label(muted(ui, "Saves to"));
            ui.add(Label::new(widgets::mono(ui, download_dir.display().to_string())).truncate());
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut b = Button::primary(label).enabled(enabled).min_width(140.0);
            if let Some(i) = icon {
                b = b.icon(i);
            }
            clicked = b.show(ui).clicked();
        });
    });
    clicked
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
    let t = tk(ui);
    let p = &pv.p;
    ui.horizontal_top(|ui| {
        let tex = thumbs.get(&p.thumb);
        let placeholder = if p.is_playlist {
            Icon::Playlist
        } else if p.audio_only {
            Icon::Music
        } else {
            Icon::Video
        };
        widgets::thumb(ui, tex.as_ref(), vec2(192.0, 108.0), placeholder);
        ui.add_space(t.s(2));
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - t.ctrl - t.s(2)).max(100.0));
            ui.add(Label::new(widgets::content_title(ui, &p.title)).wrap());
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
            ui.label(muted(ui, meta.join("  ·  ")));
            if p.is_live {
                ui.horizontal_wrapped(|ui| {
                    icons::show(ui, Icon::Info, t.icon_size, t.warning_text);
                    ui.label(widgets::colored(
                        ui,
                        "Live now: Snag records from this moment until the stream ends or you stop it.",
                        t.warning_text,
                    ));
                });
            }
            if let Some((when, target)) = &pv.already {
                ui.horizontal(|ui| {
                    icons::show(ui, Icon::Done, t.icon_size, t.success_text);
                    ui.label(widgets::colored(ui, format!("Downloaded {}", util::fmt_ago(*when)), t.success_text));
                    if let Some(tg) = target
                        && Button::quiet("Show it").show(ui).clicked()
                    {
                        platform::reveal(tg);
                    }
                });
            }
            if !p.is_playlist && util::video_in_playlist(url) {
                ui.horizontal(|ui| {
                    ui.label(muted(ui, "This video is part of a playlist."));
                    if Button::quiet("Get the whole playlist").show(ui).clicked() {
                        let pl = util::playlist_url_of(url).unwrap_or_else(|| url.to_string());
                        *retry = Some((pl, PlaylistMode::Whole));
                    }
                });
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if icon_button(ui, Icon::Close, "Close").clicked() {
                *close = true;
            }
        });
    });

    ui.add_space(t.s(4));
    ui.label(widgets::field_label(ui, "Quality"));
    ui.add_space(t.s(1));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(t.gap, t.gap);
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
    ui.add_space(t.s(3));
    if pv.quality.is_audio() {
        ui.label(widgets::field_label(ui, "Audio format"));
        ui.add_space(t.s(1));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.gap, t.gap);
            for a in AudioFormat::ALL {
                if chip(ui, pv.audio == a, a.label(), "").on_hover_text(a.hint()).clicked() {
                    pv.audio = a;
                }
            }
        });
    } else {
        ui.label(widgets::field_label(ui, "Video format"));
        ui.add_space(t.s(1));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(t.gap, t.gap);
            for v in VideoFormat::ALL {
                if chip(ui, pv.video == v, v.label(), v.hint()).clicked() {
                    pv.video = v;
                }
            }
        });
    }

    let mut valid = true;
    if !pv.p.is_playlist {
        ui.add_space(t.s(3));
        ui.horizontal(|ui| {
            icons::show(ui, Icon::Scissor, t.icon_size, t.text2);
            widgets::checkbox(ui, &mut pv.clip, "Only download part of it");
            if pv.clip {
                ui.add_space(t.s(2));
                ui.label("From");
                widgets::input(ui, egui::TextEdit::singleline(&mut pv.clip_from).hint_text("0:00"), None, 76.0);
                ui.label("to");
                let end_hint = if pv.p.duration > 0 { util::fmt_duration(pv.p.duration) } else { "end".into() };
                widgets::input(ui, egui::TextEdit::singleline(&mut pv.clip_to).hint_text(end_hint), None, 76.0);
                let untouched = pv.clip_from.trim().is_empty() && pv.clip_to.trim().is_empty();
                match clip_range(&pv.clip_from, &pv.clip_to, pv.p.duration) {
                    // Nothing typed yet is not a mistake: say what to do, quietly.
                    Err(_) if untouched => {
                        valid = false;
                        ui.label(muted(ui, "Enter a start, an end, or both"));
                    }
                    Ok((a, b)) => {
                        let end = if b > 0.0 { util::fmt_timestamp(b) } else { "the end".into() };
                        ui.label(muted(ui, format!("Saves {} to {end}", util::fmt_timestamp(a))));
                    }
                    Err(e) => {
                        valid = false;
                        icons::show(ui, Icon::Alert, t.icon_size, t.danger_text);
                        ui.label(widgets::colored(ui, e, t.danger_text));
                    }
                }
            }
        });
    }

    if pv.p.is_playlist && !pv.p.entries.is_empty() {
        ui.add_space(t.s(3));
        pv.picker.show(ui, thumbs, "entries", 280.0, false);
        if pv.picker.count() == 0 {
            valid = false;
        }
    }

    ui.add_space(t.s(4));
    let selected = pv.picker.count();
    let label = if pv.p.is_playlist {
        if selected == 1 { "Download 1 video".to_string() } else { format!("Download {selected} videos") }
    } else {
        "Download".to_string()
    };
    if footer(ui, download_dir, &label, Some(Icon::Download), valid) {
        *download = true;
    }
}

enum ResultsAct {
    None,
    Close,
    Download,
    Search(SearchSite, u32),
    Preview(usize),
}

fn results_body(ui: &mut Ui, r: &mut Results, thumbs: &mut Thumbs, download_dir: &std::path::Path) -> ResultsAct {
    let t = tk(ui);
    let mut act = ResultsAct::None;
    ui.horizontal(|ui| {
        icons::show(ui, Icon::Search, t.icon_size, t.text2);
        ui.add(Label::new(widgets::content_title(ui, &format!("Results for “{}”", r.query))).truncate());
        ui.add_space(t.s(2));
        for site in SearchSite::ALL {
            if chip(ui, r.site == site, site.label(), "").clicked() && r.site != site {
                act = ResultsAct::Search(site, 30);
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if icon_button(ui, Icon::Close, "Close").clicked() {
                act = ResultsAct::Close;
            }
        });
    });
    ui.label(muted(ui, "Tick the ones you want, or press the eye on a row to preview it first."));
    ui.add_space(t.s(3));
    if let Some(i) = r.picker.show(ui, thumbs, "results", 330.0, true) {
        act = ResultsAct::Preview(i);
    }
    ui.add_space(t.s(3));
    ui.horizontal(|ui| {
        preset_picker(ui, "search", &mut r.preset);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if r.count < 90 && Button::secondary("More results").icon(Icon::Plus).show(ui).clicked() {
                act = ResultsAct::Search(r.site, r.count + 30);
            }
        });
    });
    ui.add_space(t.s(2));
    let n = r.picker.count();
    let label = match n {
        0 => "Download".to_string(),
        1 => "Download 1 video".to_string(),
        n => format!("Download {n} videos"),
    };
    if footer(ui, download_dir, &label, Some(Icon::Download), n > 0) {
        act = ResultsAct::Download;
    }
    act
}

fn file_body(
    ui: &mut Ui,
    f: &direct::Probe,
    url: &str,
    download_dir: &std::path::Path,
    close: &mut bool,
    download: &mut bool,
) {
    let t = tk(ui);
    ui.horizontal_top(|ui| {
        widgets::thumb(ui, None, vec2(108.0, 108.0), file_icon(&f.name));
        ui.add_space(t.s(2));
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - t.ctrl - t.s(2)).max(100.0));
            ui.add(Label::new(widgets::content_title(ui, &f.name)).wrap());
            let mut meta = vec![f.size.map_or("Size unknown".to_string(), |s| util::fmt_bytes(s as f64))];
            let mime = f.mime.split(';').next().unwrap_or("").trim();
            if !mime.is_empty() {
                meta.push(mime.to_string());
            }
            meta.push(util::host_of(url));
            ui.label(muted(ui, meta.join("  ·  ")));
            ui.add_space(t.s(1));
            let how = if f.ranges {
                "A plain file. Snag downloads it with its own engine, several connections at once, and can pause and resume it."
            } else {
                "A plain file. This server doesn't allow resuming, so Snag downloads it in one go."
            };
            widgets::para(ui, how);
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            if icon_button(ui, Icon::Close, "Close").clicked() {
                *close = true;
            }
        });
    });
    ui.add_space(t.s(4));
    if footer(ui, download_dir, "Download", Some(Icon::Download), true) {
        *download = true;
    }
}

fn file_icon(name: &str) -> Icon {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" => Icon::Archive,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "tif" | "tiff" | "heic" | "psd" | "psb" => Icon::Image,
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" => Icon::Video,
        "mp3" | "m4a" | "wav" | "flac" | "ogg" | "opus" | "aac" | "aif" | "aiff" => Icon::Note,
        "xls" | "xlsx" | "csv" => Icon::Table,
        "ppt" | "pptx" => Icon::Slides,
        "exe" | "msi" | "msix" | "apk" | "dmg" | "pkg" | "appimage" | "deb" | "rpm" => Icon::Package,
        "blend" | "fbx" | "obj" | "stl" | "step" | "stp" | "3mf" | "glb" | "gltf" | "usdz" => Icon::Cube,
        _ => Icon::Page,
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
