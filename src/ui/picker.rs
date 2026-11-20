//! A list of videos to choose from: a playlist, a channel or search results.
//! Click a row to tick it, Shift+click to tick a run of rows, filter by title.

use eframe::egui::{self, Align, Layout, Rect, Sense, Stroke, Ui, UiBuilder, vec2};

use super::icons::{self, Icon};
use super::theme::tk;
use super::thumbs::Thumbs;
use super::widgets::{self, Button, icon_button, muted};
use crate::util;
use crate::ytdlp::Entry;

pub struct Picker {
    pub entries: Vec<Entry>,
    pub picked: Vec<bool>,
    pub filter: String,
    anchor: Option<usize>,
}

impl Picker {
    pub fn new(entries: Vec<Entry>, all_on: bool) -> Picker {
        let picked = vec![all_on; entries.len()];
        Picker { entries, picked, filter: String::new(), anchor: None }
    }

    pub fn selected(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|&i| self.picked[i]).collect()
    }

    pub fn count(&self) -> usize {
        self.picked.iter().filter(|p| **p).count()
    }

    fn matches(&self, i: usize) -> bool {
        let f = self.filter.trim().to_lowercase();
        if f.is_empty() {
            return true;
        }
        let e = &self.entries[i];
        e.title.to_lowercase().contains(&f) || e.uploader.to_lowercase().contains(&f)
    }

    /// Draws the list. Returns the row whose preview button was clicked.
    pub fn show(
        &mut self,
        ui: &mut Ui,
        thumbs: &mut Thumbs,
        id: &str,
        max_height: f32,
        preview: bool,
    ) -> Option<usize> {
        let t = tk(ui);
        let visible: Vec<usize> = (0..self.entries.len()).filter(|&i| self.matches(i)).collect();

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = t.s(1);
            widgets::input(
                ui,
                egui::TextEdit::singleline(&mut self.filter).hint_text("Filter titles"),
                Some(Icon::Search),
                220.0,
            );
            ui.add_space(t.s(2));
            if Button::quiet("All").show(ui).clicked() {
                visible.iter().for_each(|&i| self.picked[i] = true);
            }
            if Button::quiet("None").show(ui).clicked() {
                visible.iter().for_each(|&i| self.picked[i] = false);
            }
            if Button::quiet("Invert").tip("Tick what's unticked and the other way round").show(ui).clicked() {
                visible.iter().for_each(|&i| self.picked[i] = !self.picked[i]);
            }
            if Button::quiet("First 10").tip("On a channel, the 10 newest").show(ui).clicked() {
                self.picked.iter_mut().for_each(|p| *p = false);
                visible.iter().take(10).for_each(|&i| self.picked[i] = true);
            }
            if !self.filter.trim().is_empty()
                && Button::quiet("Only matching").tip("Tick exactly the rows that match").show(ui).clicked()
            {
                for i in 0..self.entries.len() {
                    self.picked[i] = self.matches(i);
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let total: u64 = self.selected().iter().map(|&i| self.entries[i].duration).sum();
                let mut text = format!("{} of {} ticked", self.count(), self.entries.len());
                if total > 0 {
                    text.push_str(&format!(" · {}", util::fmt_duration(total)));
                }
                ui.label(muted(ui, text));
            });
        });
        ui.add_space(t.s(2));

        let mut clicked: Option<(usize, bool)> = None;
        let mut preview_row = None;
        widgets::well(ui, |ui| {
            ui.set_width(ui.available_width());
            if visible.is_empty() {
                ui.add_space(t.s(3));
                ui.vertical_centered(|ui| ui.label(muted(ui, "Nothing matches that filter.")));
                ui.add_space(t.s(3));
                return;
            }
            egui::ScrollArea::vertical().id_salt(id).max_height(max_height).auto_shrink([false, true]).show_rows(
                ui,
                t.row,
                visible.len(),
                |ui, range| {
                    for vi in range {
                        let i = visible[vi];
                        let (row_click, wants_preview) = row(ui, &self.entries[i], self.picked[i], thumbs, preview);
                        if row_click {
                            clicked = Some((vi, widgets::click_modifiers(ui).shift));
                        }
                        if wants_preview {
                            preview_row = Some(i);
                        }
                    }
                },
            );
        });

        if let Some((vi, shift)) = clicked {
            let i = visible[vi];
            let on = !self.picked[i];
            match (shift, self.anchor.and_then(|a| visible.iter().position(|&x| x == a))) {
                (true, Some(av)) => {
                    let (a, b) = if av <= vi { (av, vi) } else { (vi, av) };
                    visible[a..=b].iter().for_each(|&x| self.picked[x] = on);
                }
                _ => self.picked[i] = on,
            }
            self.anchor = Some(i);
        }
        preview_row
    }
}

/// One row. Returns (row clicked, preview clicked).
fn row(ui: &mut Ui, e: &Entry, on: bool, thumbs: &mut Thumbs, preview: bool) -> (bool, bool) {
    let t = tk(ui);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), t.row), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, &e.title));
    let bg = rect.shrink(1.0);
    if on {
        ui.painter().rect_filled(bg, t.r_sm as u8, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(bg, t.r_sm as u8, t.surface);
    }
    if widgets::focus_visible(&resp) {
        ui.painter().rect_stroke(bg, t.r_sm as u8, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    let inner = rect.shrink2(vec2(t.s(2), t.s(1)));
    let mut r = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));

    // Tick box, painted so the whole row toggles it.
    let (bx, _) = r.allocate_exact_size(vec2(t.s(6), t.s(6)), Sense::hover());
    let b = Rect::from_center_size(bx.center(), vec2(18.0, 18.0));
    let br = t.r_sm * 0.5;
    if on {
        r.painter().rect_filled(b, br as u8, t.accent);
        icons::paint_at(r.painter(), Icon::Check, b.center(), 16.0, t.on_accent);
    } else {
        r.painter().rect_filled(b, br as u8, t.surface);
        r.painter().rect_stroke(b, br as u8, Stroke::new(1.0, t.border_strong), egui::StrokeKind::Inside);
    }
    r.add_space(t.s(1));
    let tex = thumbs.get(&e.thumb);
    widgets::thumb(&mut r, tex.as_ref(), vec2(64.0, 36.0), Icon::Video);
    r.add_space(t.s(2));

    let right = if preview { t.ctrl + t.s(1) } else { 0.0 } + 56.0;
    let w = (r.available_width() - right).max(60.0);
    r.allocate_ui_with_layout(vec2(w, inner.height()), Layout::top_down(Align::Min), |ui| {
        ui.set_width(w);
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add_space(2.0);
        ui.add(egui::Label::new(widgets::strong(ui, &e.title).size(t.fs_sm)).truncate()).on_hover_text(&e.title);
        let mut sub = Vec::new();
        if !e.uploader.is_empty() {
            sub.push(e.uploader.clone());
        }
        if let Some(v) = e.views {
            sub.push(views(v));
        }
        ui.add(egui::Label::new(widgets::caption(ui, sub.join("  ·  "))).truncate());
    });
    let mut wants_preview = false;
    r.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if preview && icon_button(ui, Icon::Eye, "Preview this one: quality, trim, size").clicked() {
            wants_preview = true;
        }
        if e.duration > 0 {
            ui.label(muted(ui, util::fmt_duration(e.duration)));
        }
    });
    (resp.clicked(), wants_preview)
}

fn views(n: u64) -> String {
    match n {
        0..=999 => format!("{n} views"),
        1_000..=999_999 => format!("{:.1}K views", n as f64 / 1e3).replace(".0K", "K"),
        1_000_000..=999_999_999 => format!("{:.1}M views", n as f64 / 1e6).replace(".0M", "M"),
        _ => format!("{:.1}B views", n as f64 / 1e9).replace(".0B", "B"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_counts() {
        assert_eq!(views(950), "950 views");
        assert_eq!(views(581_026), "581K views");
        assert_eq!(views(1_200_000), "1.2M views");
        assert_eq!(views(3_000_000), "3M views");
    }
}
