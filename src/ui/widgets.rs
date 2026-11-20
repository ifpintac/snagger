//! Small building blocks shared by the tabs.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Rect, Response, RichText, Sense, Stroke, TextureHandle, Ui, Vec2,
    pos2, vec2,
};

use super::theme::{Pal, bold};

/// A frameless icon button with a hover background and tooltip.
pub fn icon_button(ui: &mut Ui, icon: &str, tip: &str) -> Response {
    let size = vec2(30.0, 28.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let pal = Pal::of(ui);
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let color = if resp.hovered() { pal.text } else { pal.muted };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, icon, FontId::proportional(17.0), color);
    resp.on_hover_text(tip)
}

pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let size = vec2(120.0, 32.0);
    if !enabled {
        // egui's own greyed-out look; white text would vanish here.
        let label = RichText::new(text).family(bold()).size(14.5);
        return ui.add_enabled(false, egui::Button::new(label).min_size(size));
    }
    let label = RichText::new(text).color(Color32::WHITE).family(bold()).size(14.5);
    ui.add(egui::Button::new(label).fill(Pal::of(ui).accent).min_size(size))
}

/// A choice chip: `main` text with an optional quieter note.
pub fn chip(ui: &mut Ui, selected: bool, main: &str, note: &str) -> Response {
    let pal = Pal::of(ui);
    let mut job = egui::text::LayoutJob::default();
    let main_color = if selected { Color32::WHITE } else { pal.text };
    job.append(
        main,
        0.0,
        egui::TextFormat { font_id: FontId::new(14.0, bold()), color: main_color, ..Default::default() },
    );
    if !note.is_empty() {
        let note_color = if selected { Color32::from_white_alpha(200) } else { pal.muted };
        job.append(
            note,
            6.0,
            egui::TextFormat { font_id: FontId::proportional(12.5), color: note_color, ..Default::default() },
        );
    }
    let fill = if selected { pal.accent } else { ui.visuals().widgets.inactive.weak_bg_fill };
    let stroke = if selected { Stroke::NONE } else { Stroke::new(1.0, pal.border) };
    ui.add(
        egui::Button::new(job)
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(14))
            .min_size(vec2(0.0, 30.0)),
    )
}

/// A thin progress bar. `striped` animates for unknown progress.
pub fn bar(ui: &mut Ui, width: f32, frac: f32, color: Color32, busy: bool) {
    let pal = Pal::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let r = CornerRadius::same(3);
    ui.painter().rect_filled(rect, r, pal.track);
    if busy {
        let t = ui.input(|i| i.time) as f32;
        let w = rect.width() * 0.3;
        let x = rect.left() + ((t * 0.6).fract() * (rect.width() + w)) - w;
        let seg =
            Rect::from_min_max(pos2(x.max(rect.left()), rect.top()), pos2((x + w).min(rect.right()), rect.bottom()));
        if seg.width() > 0.0 {
            ui.painter().rect_filled(seg, r, color);
        }
        ui.ctx().request_repaint();
    } else if frac > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2(rect.width() * frac.clamp(0.0, 1.0), rect.height()));
        ui.painter().rect_filled(fill, r, color);
    }
}

/// A thumbnail cropped to fill `size`, or a quiet placeholder.
pub fn thumb(ui: &mut Ui, tex: Option<&TextureHandle>, size: Vec2, icon: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let pal = Pal::of(ui);
    let r = CornerRadius::same(6);
    match tex {
        Some(t) => {
            let [w, h] = t.size();
            let (w, h) = (w as f32, h as f32);
            let target = size.x / size.y;
            let uv = if w / h > target {
                let keep = (h * target) / w;
                Rect::from_min_max(pos2((1.0 - keep) / 2.0, 0.0), pos2((1.0 + keep) / 2.0, 1.0))
            } else {
                let keep = (w / target) / h;
                Rect::from_min_max(pos2(0.0, (1.0 - keep) / 2.0), pos2(1.0, (1.0 + keep) / 2.0))
            };
            egui::Image::from_texture((t.id(), size)).uv(uv).corner_radius(r).paint_at(ui, rect);
        }
        None => {
            ui.painter().rect_filled(rect, r, pal.track);
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                icon,
                FontId::proportional(size.y * 0.36),
                pal.muted,
            );
        }
    }
    resp
}

pub fn small_muted(ui: &Ui, text: impl Into<String>) -> RichText {
    RichText::new(text).color(Pal::of(ui).muted).size(12.5)
}

pub fn heading(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).family(bold()).size(16.0));
}

pub fn card(ui: &Ui) -> egui::Frame {
    let pal = Pal::of(ui);
    egui::Frame::new()
        .fill(pal.card)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(14))
}

/// A section of the settings page.
pub fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        heading(ui, title);
        ui.add_space(6.0);
        add(ui);
    });
    ui.add_space(10.0);
}

/// One labelled row in a settings section.
pub fn setting_row(ui: &mut Ui, label: &str, hint: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(190.0, 28.0), Sense::hover());
        ui.painter().text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(14.0),
            ui.visuals().text_color(),
        );
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| add(ui));
            if !hint.is_empty() {
                ui.label(small_muted(ui, hint));
            }
        });
    });
    ui.add_space(4.0);
}
