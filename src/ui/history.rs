//! The History tab: everything Snagger has saved, searchable.

use std::collections::HashMap;

use eframe::egui::{self, Align, Label, Layout, Sense, Ui, UiBuilder, vec2};

use super::Confirm;
use super::icons::{self, Icon};
use super::theme::tk;
use super::thumbs::Thumbs;
use super::widgets::{self, Button, icon_button, menu_item, muted};
use crate::hub::Hub;
use crate::model::{HistoryItem, Job};
use crate::platform;
use crate::util;

#[derive(Default)]
pub struct HistoryUi {
    query: String,
    /// Saved transcripts, read once per search (cleared when the box is empty).
    transcripts: HashMap<String, String>,
    /// For the current search: rows found only by what was said, with the
    /// time and an excerpt.
    said: HashMap<String, (String, String)>,
    said_for: String,
}

#[derive(Clone, Copy)]
enum Act {
    Open,
    Reveal,
    Again,
    Copy,
    Page,
    Forget,
    Transcript,
    OpenTranscript,
}

const ROW_H: f32 = 60.0;

impl HistoryUi {
    /// Returns a message to show as a toast.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        hub: &Hub,
        thumbs: &mut Thumbs,
        confirm: &mut Option<Confirm>,
    ) -> Option<String> {
        let t = tk(ui);
        let mut acts: Vec<(String, Act)> = Vec::new();
        {
            let c = hub.lock();
            ui.horizontal(|ui| {
                widgets::title(ui, "History");
                ui.label(muted(ui, format!("{} downloads", c.history.len())));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !c.history.is_empty() && Button::secondary("Clear history").icon(Icon::Trash).show(ui).clicked()
                    {
                        *confirm = Some(Confirm::ClearHistory);
                    }
                    let edit = egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search titles, channels and what was said");
                    widgets::input(ui, edit, Some(Icon::Search), 300.0);
                });
            });
            ui.add_space(t.s(4));

            let q = self.query.trim().to_lowercase();
            let by_details = |h: &HistoryItem| {
                h.title.to_lowercase().contains(&q)
                    || h.uploader.to_lowercase().contains(&q)
                    || h.site.to_lowercase().contains(&q)
                    || h.url.to_lowercase().contains(&q)
            };
            if q.chars().count() < 3 {
                // Short searches stay on titles; an empty box forgets the cache
                // so new transcripts are found next time.
                if q.is_empty() {
                    self.transcripts.clear();
                }
                self.said.clear();
                self.said_for.clear();
            } else if q != self.said_for {
                self.said.clear();
                for h in c.history.iter().filter(|h| !by_details(h)) {
                    let text = self.transcripts.entry(h.id.clone()).or_insert_with(|| {
                        h.files
                            .iter()
                            .filter_map(|f| crate::transcript::existing(f))
                            .filter_map(|p| std::fs::read_to_string(p).ok())
                            .collect::<Vec<_>>()
                            .join("\n")
                    });
                    if let Some(hit) = crate::transcript::search(text, &q) {
                        self.said.insert(h.id.clone(), hit);
                    }
                }
                self.said_for = q.clone();
            }
            let rows: Vec<&HistoryItem> =
                c.history.iter().filter(|h| q.is_empty() || by_details(h) || self.said.contains_key(&h.id)).collect();
            if rows.is_empty() {
                ui.add_space(t.s(10));
                ui.vertical_centered(|ui| {
                    icons::show(ui, Icon::History, t.icon_size * 2.0, t.text2);
                    ui.add_space(t.s(2));
                    ui.label(muted(
                        ui,
                        if q.is_empty() { "Finished downloads show up here." } else { "Nothing matches that search." },
                    ));
                });
            } else {
                egui::ScrollArea::vertical().id_salt("history").auto_shrink([false, false]).show_rows(
                    ui,
                    ROW_H,
                    rows.len(),
                    |ui, range| {
                        for i in range {
                            if let Some(a) = history_row(ui, rows[i], thumbs, self.said.get(&rows[i].id)) {
                                acts.push((rows[i].id.clone(), a));
                            }
                        }
                    },
                );
            }
        }

        let mut toast = None;
        for (id, act) in acts {
            let mut c = hub.lock();
            let Some(h) = c.history.iter().find(|h| h.id == id).cloned() else { continue };
            match act {
                Act::Forget => {
                    c.history.retain(|x| x.id != id);
                    c.dirty.history = true;
                }
                Act::Again => {
                    let mut job = Job::new(&h.url, h.opts.clone());
                    job.title = h.title.clone();
                    job.thumb = h.thumb.clone();
                    job.site = h.site.clone();
                    let jid = c.add_job(job);
                    c.move_to_top(&jid);
                    toast = Some(format!("Downloading “{}” again", util::ellipsize(&h.title, 50)));
                }
                Act::Copy => {
                    c.clip_ignore = h.url.clone();
                    ui.ctx().copy_text(h.url.clone());
                    toast = Some("Link copied".into());
                }
                Act::Page => {
                    drop(c);
                    platform::open_url(&h.url);
                }
                Act::Transcript => {
                    drop(c);
                    if let Some(media) = h.files.first() {
                        if media.exists() {
                            crate::transcript::get_in_background(
                                hub,
                                h.url.clone(),
                                h.title.clone(),
                                media.clone(),
                                h.opts.clip,
                            );
                        } else {
                            toast = Some("That file has been moved or deleted.".into());
                        }
                    }
                }
                Act::OpenTranscript => {
                    drop(c);
                    match h.files.first().and_then(|m| crate::transcript::existing(m)) {
                        Some(p) => {
                            platform::open_path(&p);
                        }
                        None => toast = Some("That transcript has been moved or deleted.".into()),
                    }
                }
                Act::Open | Act::Reveal => {
                    drop(c);
                    match h.target().filter(|t| t.exists()) {
                        Some(t) if matches!(act, Act::Open) => {
                            platform::open_path(&t);
                        }
                        Some(t) => platform::reveal(&t),
                        None => toast = Some("That file has been moved or deleted.".into()),
                    }
                }
            }
        }
        hub.wake();
        toast
    }
}

fn history_row(ui: &mut Ui, h: &HistoryItem, thumbs: &mut Thumbs, said: Option<&(String, String)>) -> Option<Act> {
    let t = tk(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let inner = widgets::list_row(ui, rect, &resp, false);
    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let audio = h.preset.quality.is_audio();
    let tex = thumbs.get(&h.thumb);
    let icon_ph = if h.opts.direct {
        Icon::Page
    } else if h.items > 1 {
        Icon::Playlist
    } else if audio {
        Icon::Music
    } else {
        Icon::Video
    };
    widgets::thumb(&mut row, tex.as_ref(), vec2(86.0, 48.0), icon_ph);
    row.add_space(t.s(3));
    let exists = h.files.iter().any(|f| f.exists());
    let mut act = None;
    let text_w = (row.available_width() - 4.0 * (t.ctrl + t.s(1)) - t.s(2)).max(60.0);
    row.allocate_ui_with_layout(vec2(text_w, inner.height()), Layout::top_down(Align::Min), |ui| {
        ui.set_width(text_w);
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(t.s(1));
        let title = widgets::strong(ui, &h.title);
        let title = if exists { title } else { title.color(t.text2) };
        ui.add(Label::new(title).truncate()).on_hover_text(&h.title);
        let kind = if h.opts.direct { "File".to_string() } else { h.preset.label() };
        let mut meta = vec![h.site.clone(), kind, util::fmt_date(h.finished)];
        if h.items > 1 {
            meta.push(format!("{} videos", h.items));
        }
        if h.bytes > 0 {
            meta.push(util::fmt_bytes(h.bytes as f64));
        }
        if !exists {
            meta.push("file moved or deleted".into());
        }
        meta.retain(|m| !m.is_empty());
        match said {
            // Found by what was said: show where.
            Some((stamp, snippet)) => {
                let line = format!("At {stamp}  ·  “{snippet}”");
                ui.add(Label::new(widgets::caption(ui, &line).color(t.text)).truncate()).on_hover_text(&line);
            }
            None => {
                ui.add(Label::new(widgets::caption(ui, meta.join("  ·  "))).truncate());
            }
        }
    });
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = t.s(1);
        widgets::more_menu(ui, |ui| menu_items(ui, h, &mut act));
        if icon_button(ui, Icon::Refresh, "Download again").clicked() {
            act = Some(Act::Again);
        }
        if exists {
            if icon_button(ui, Icon::Folder, "Show in folder").clicked() {
                act = Some(Act::Reveal);
            }
            if icon_button(ui, Icon::Open, "Open").clicked() {
                act = Some(Act::Open);
            }
        }
    });
    if resp.double_clicked() && exists {
        act = Some(Act::Open);
    }
    widgets::context_menu(&resp, |ui| menu_items(ui, h, &mut act));
    act
}

fn menu_items(ui: &mut Ui, h: &HistoryItem, act: &mut Option<Act>) {
    // One video from a page: its captions can be read for a transcript.
    let transcript = (h.files.len() == 1 && !h.opts.direct).then(|| match crate::transcript::existing(&h.files[0]) {
        Some(_) => (Icon::Page, "Open transcript", Act::OpenTranscript),
        None => (Icon::Page, "Get transcript", Act::Transcript),
    });
    let items = [
        Some((Icon::Refresh, "Download again", Act::Again)),
        transcript,
        Some((Icon::Copy, "Copy link", Act::Copy)),
        Some((Icon::Globe, "Open the page", Act::Page)),
        Some((Icon::Trash, "Remove from history", Act::Forget)),
    ];
    for (icon, text, a) in items.into_iter().flatten() {
        if menu_item(ui, Some(icon), text).clicked() {
            *act = Some(a);
        }
    }
}
