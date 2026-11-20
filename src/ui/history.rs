//! The History tab: everything Snag has saved, searchable.

use eframe::egui::{self, Align, Label, Layout, RichText, Sense, Ui, UiBuilder, vec2};
use egui_phosphor::regular as icon;

use super::Confirm;
use super::theme::{Pal, bold};
use super::thumbs::Thumbs;
use super::widgets::{self, icon_button, small_muted};
use crate::hub::Hub;
use crate::model::{HistoryItem, Job};
use crate::platform;
use crate::util;

#[derive(Default)]
pub struct HistoryUi {
    query: String,
}

#[derive(Clone, Copy)]
enum Act {
    Open,
    Reveal,
    Again,
    Copy,
    Page,
    Forget,
}

const ROW_H: f32 = 64.0;

impl HistoryUi {
    /// Returns a message to show as a toast.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        hub: &Hub,
        thumbs: &mut Thumbs,
        confirm: &mut Option<Confirm>,
    ) -> Option<String> {
        let mut acts: Vec<(String, Act)> = Vec::new();
        {
            let c = hub.lock();
            ui.horizontal(|ui| {
                widgets::heading(ui, "History");
                ui.label(small_muted(ui, format!("{} downloads", c.history.len())));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !c.history.is_empty() && ui.button(format!("{}  Clear history", icon::TRASH)).clicked() {
                        *confirm = Some(Confirm::ClearHistory);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(format!("{}  Search titles, channels, sites", icon::MAGNIFYING_GLASS))
                            .desired_width(300.0),
                    );
                });
            });
            ui.add_space(6.0);

            let q = self.query.trim().to_lowercase();
            let rows: Vec<&HistoryItem> = c
                .history
                .iter()
                .filter(|h| {
                    q.is_empty()
                        || h.title.to_lowercase().contains(&q)
                        || h.uploader.to_lowercase().contains(&q)
                        || h.site.to_lowercase().contains(&q)
                        || h.url.to_lowercase().contains(&q)
                })
                .collect();
            if rows.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    let pal = Pal::of(ui);
                    ui.label(RichText::new(icon::CLOCK_COUNTER_CLOCKWISE).size(40.0).color(pal.muted));
                    ui.label(small_muted(
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
                            if let Some(a) = history_row(ui, rows[i], thumbs) {
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

fn history_row(ui: &mut Ui, h: &HistoryItem, thumbs: &mut Thumbs) -> Option<Act> {
    let pal = Pal::of(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(0.0, 2.0)), 8, pal.card_hover);
    }
    let inner = rect.shrink2(vec2(8.0, 6.0));
    let mut row = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let audio = h.preset.quality.is_audio();
    let tex = thumbs.get(&h.thumb);
    let icon_ph = if h.items > 1 {
        icon::PLAYLIST
    } else if audio {
        icon::MUSIC_NOTES
    } else {
        icon::FILM_STRIP
    };
    widgets::thumb(&mut row, tex.as_ref(), vec2(86.0, 48.0), icon_ph);
    row.add_space(10.0);
    let exists = h.files.iter().any(|f| f.exists());
    let mut act = None;
    let text_w = (row.available_width() - 5.0 * 32.0).max(60.0);
    row.allocate_ui_with_layout(vec2(text_w, inner.height()), Layout::top_down(Align::Min), |ui| {
        ui.set_width(text_w);
        ui.spacing_mut().item_spacing.y = 3.0;
        let title = RichText::new(&h.title).family(bold()).size(14.5);
        let title = if exists { title } else { title.color(pal.muted) };
        ui.add(Label::new(title).truncate()).on_hover_text(&h.title);
        let mut meta = vec![h.site.clone(), h.preset.label(), util::fmt_date(h.finished)];
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
        ui.add(Label::new(small_muted(ui, meta.join("  ·  "))).truncate());
    });
    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let menu = ui.menu_button(RichText::new(icon::DOTS_THREE_VERTICAL).size(17.0), |ui| menu_items(ui, &mut act));
        menu.response.on_hover_text("More");
        if icon_button(ui, icon::ARROWS_CLOCKWISE, "Download again").clicked() {
            act = Some(Act::Again);
        }
        if exists {
            if icon_button(ui, icon::FOLDER_OPEN, "Show in folder").clicked() {
                act = Some(Act::Reveal);
            }
            if icon_button(ui, icon::PLAY_CIRCLE, "Open").clicked() {
                act = Some(Act::Open);
            }
        }
    });
    if resp.double_clicked() && exists {
        act = Some(Act::Open);
    }
    resp.context_menu(|ui| menu_items(ui, &mut act));
    act
}

fn menu_items(ui: &mut Ui, act: &mut Option<Act>) {
    for (text, a) in [
        (format!("{}  Download again", icon::ARROWS_CLOCKWISE), Act::Again),
        (format!("{}  Copy link", icon::COPY), Act::Copy),
        (format!("{}  Open the page", icon::ARROW_SQUARE_OUT), Act::Page),
        (format!("{}  Remove from history", icon::TRASH), Act::Forget),
    ] {
        if ui.button(text).clicked() {
            *act = Some(a);
            ui.close();
        }
    }
}
