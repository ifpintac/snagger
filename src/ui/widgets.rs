//! Snag's components, drawn in the command-console language.
//!
//! One pale colour does everything: text, lines and fills are the accent at
//! different strengths on near-black. Modules are unboxed: a hairline with a
//! tick at each end and a small, widely tracked uppercase title. Overlays and
//! the work area sit in a 1 px frame with two clipped corners. Buttons are
//! keys: thin outlines that fill solid when pressed or chosen. No shadows and
//! no glow. State never rides on colour alone: a chosen key fills solid and
//! adds a check or marker, inputs carry a 3:1 edge, and keyboard focus draws
//! a 2 px accent ring.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontFamily, FontId, Galley, InnerResponse, Layout, Margin, Painter, Pos2, Rect,
    Response, RichText, Sense, Shape, Stroke, StrokeKind, TextFormat, TextureHandle, Ui, UiBuilder, Vec2, WidgetInfo,
    WidgetText, WidgetType, pos2, text::LayoutJob, vec2,
};

use super::icons::{self, Icon};
use super::theme::{Tokens, bold, head, tk, tk_ctx};

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

fn cr(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

/// Mixes two opaque colours in sRGB.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

static KEYBOARD: AtomicBool = AtomicBool::new(false);

/// Call once a frame: focus rings show only while the keyboard is driving
/// (the `:focus-visible` rule), not after every mouse click.
pub fn track_input_mode(ctx: &egui::Context) {
    ctx.input(|i| {
        for e in &i.events {
            match e {
                egui::Event::Key { key: egui::Key::Tab, pressed: true, .. } => KEYBOARD.store(true, Ordering::Relaxed),
                egui::Event::PointerButton { pressed: true, .. } => KEYBOARD.store(false, Ordering::Relaxed),
                _ => {}
            }
        }
    });
}

pub fn focus_visible(resp: &Response) -> bool {
    resp.has_focus() && KEYBOARD.load(Ordering::Relaxed)
}

/// Hover blend, --dur-fast (instant with reduced motion).
fn anim(ui: &Ui, id: egui::Id, on: bool) -> f32 {
    let t = tk(ui);
    let dur = if super::theme::reduced_motion() { 0.0 } else { t.dur_fast };
    ui.ctx().animate_bool_with_time(id, on, dur)
}

/// The six corners of a frame whose top-left and bottom-right corners are
/// cut off at 45°, `cut` points deep.
fn clipped_points(rect: Rect, cut: f32) -> Vec<Pos2> {
    let c = cut.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    vec![
        pos2(rect.left() + c, rect.top()),
        pos2(rect.right(), rect.top()),
        pos2(rect.right(), rect.bottom() - c),
        pos2(rect.right() - c, rect.bottom()),
        pos2(rect.left(), rect.bottom()),
        pos2(rect.left(), rect.top() + c),
    ]
}

/// A clipped-corner frame. The line sits half a pixel in, on whole pixels.
pub fn clipped_shape(rect: Rect, cut: f32, fill: Color32, stroke: Stroke) -> Shape {
    let r = rect.shrink(stroke.width / 2.0);
    Shape::convex_polygon(clipped_points(r, cut), fill, stroke)
}

/// The module mark: a hairline with a short tick down at each end.
pub fn rule(p: &Painter, left: f32, right: f32, y: f32, t: &Tokens) {
    let y = y.round() + 0.5;
    p.hline(left..=right, y, Stroke::new(t.bw, t.dk_line));
    let tick = Stroke::new(t.bw, t.border_strong);
    for x in [left.round() + 0.5, right.round() - 0.5] {
        p.vline(x, y - 0.5..=y + t.s(1), tick);
    }
}

/// 2 px accent ring, 2 px outside the control.
fn focus_ring(p: &Painter, rect: Rect, t: &Tokens) {
    p.rect_stroke(rect.expand(2.0), cr(t.r_sm), Stroke::new(2.0, t.accent), StrokeKind::Outside);
}

/// One run of text with optional letter spacing, coloured when painted.
fn galley_spaced(ui: &Ui, text: &str, font: FontId, spacing: f32) -> Arc<Galley> {
    let job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat { font_id: font, extra_letter_spacing: spacing, color: Color32::PLACEHOLDER, ..Default::default() },
    );
    ui.painter().layout_job(job)
}

fn galley(ui: &Ui, text: &str, font: FontId) -> Arc<Galley> {
    galley_spaced(ui, text, font, 0.0)
}

/// A key's label: uppercase, --fw-label, lightly tracked.
fn key_galley(ui: &Ui, text: &str) -> Arc<Galley> {
    let t = tk(ui);
    galley_spaced(ui, &text.to_uppercase(), FontId::new(t.fs_sm, bold()), t.tracking(t.fs_sm) * 0.5)
}

/// How a key looks this frame.
struct KeyLook {
    fill: Option<Color32>,
    edge: Option<Color32>,
    fg: Color32,
}

fn paint_key(p: &Painter, rect: Rect, look: &KeyLook, t: &Tokens) {
    if let Some(fill) = look.fill {
        p.rect_filled(rect, cr(t.r_btn), fill);
    }
    if let Some(edge) = look.edge {
        p.rect_stroke(rect, cr(t.r_btn), Stroke::new(t.bw, edge), StrokeKind::Inside);
    }
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A lit key: the one main action on a screen or dialog.
    Primary,
    /// An outlined key that lights up while pressed.
    Secondary,
    /// No outline: low-emphasis actions.
    Quiet,
    /// An outlined key in the danger colours, for something destructive.
    Danger,
}

pub struct Button<'a> {
    kind: Kind,
    text: &'a str,
    icon: Option<Icon>,
    enabled: bool,
    loading: bool,
    min_width: f32,
    tip: Option<&'a str>,
}

impl<'a> Button<'a> {
    pub fn new(kind: Kind, text: &'a str) -> Button<'a> {
        Button { kind, text, icon: None, enabled: true, loading: false, min_width: 0.0, tip: None }
    }
    pub fn primary(text: &'a str) -> Button<'a> {
        Button::new(Kind::Primary, text)
    }
    pub fn secondary(text: &'a str) -> Button<'a> {
        Button::new(Kind::Secondary, text)
    }
    pub fn quiet(text: &'a str) -> Button<'a> {
        Button::new(Kind::Quiet, text)
    }
    pub fn danger(text: &'a str) -> Button<'a> {
        Button::new(Kind::Danger, text)
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
        self
    }
    /// Shows a spinner in place of the icon and ignores clicks.
    pub fn loading(mut self, on: bool) -> Self {
        self.loading = on;
        self
    }
    pub fn min_width(mut self, w: f32) -> Self {
        self.min_width = w;
        self
    }
    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let t = tk(ui);
        let enabled = self.enabled && !self.loading && ui.is_enabled();
        let g = key_galley(ui, self.text);
        let has_icon = self.icon.is_some() || self.loading;
        let icon_w = if has_icon { t.icon_size + if self.text.is_empty() { 0.0 } else { t.s(2) } } else { 0.0 };
        let content_w = icon_w + g.size().x;
        let padx = if self.kind == Kind::Quiet { t.s(2) } else { t.padx };
        let size = vec2((padx * 2.0 + content_w).max(self.min_width), t.ctrl);
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(size, sense);
        let text = self.text;
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
        if ui.is_rect_visible(rect) {
            let hover = anim(ui, resp.id.with("hover"), resp.hovered() && enabled);
            let pressed = enabled && resp.is_pointer_button_down_on();
            let disabled = !enabled && !self.loading;
            let look = if disabled {
                let edge = (self.kind != Kind::Quiet).then_some(t.border);
                KeyLook { fill: None, edge, fg: t.border_strong }
            } else {
                match self.kind {
                    // Lit at rest; dims a step on hover and another when pressed.
                    Kind::Primary => KeyLook {
                        fill: Some(mix(t.pri_bg, t.bg, if pressed { 0.28 } else { 0.14 * hover })),
                        edge: Some(t.pri_border),
                        fg: t.pri_fg,
                    },
                    Kind::Secondary if pressed => {
                        KeyLook { fill: Some(t.pri_bg), edge: Some(t.pri_border), fg: t.pri_fg }
                    }
                    Kind::Secondary => KeyLook {
                        fill: (hover > 0.0).then(|| t.dk_fill.gamma_multiply(hover)),
                        edge: Some(mix(t.border_strong, t.text, hover)),
                        fg: t.text,
                    },
                    Kind::Quiet => KeyLook {
                        fill: if pressed {
                            Some(t.dk_fill2)
                        } else {
                            (hover > 0.0).then(|| t.dk_fill.gamma_multiply(hover))
                        },
                        edge: None,
                        fg: t.accent_text,
                    },
                    Kind::Danger if pressed => {
                        KeyLook { fill: Some(t.danger_text), edge: Some(t.danger_text), fg: t.on_accent }
                    }
                    Kind::Danger => KeyLook {
                        fill: (hover > 0.0).then(|| t.dk_fill.gamma_multiply(hover)),
                        edge: Some(mix(t.danger, t.danger_text, hover)),
                        fg: t.danger_text,
                    },
                }
            };
            let p = ui.painter();
            paint_key(p, rect, &look, t);
            // Keys travel a pixel when pressed.
            let cy = rect.center().y + if pressed { 1.0 } else { 0.0 };
            let mut x = rect.center().x - content_w / 2.0;
            if self.loading {
                let s = Rect::from_center_size(pos2(x + t.icon_size / 2.0, cy), Vec2::splat(t.icon_size - 2.0));
                egui::Spinner::new().size(s.width()).color(look.fg).paint_at(ui, s);
            } else if let Some(i) = self.icon {
                icons::paint_at(p, i, pos2(x + t.icon_size / 2.0, cy), t.icon_size, look.fg);
            }
            x += icon_w;
            let gs = g.size();
            p.galley(pos2(x, cy - gs.y / 2.0), g, look.fg);
            if focus_visible(&resp) {
                focus_ring(p, rect, t);
            }
        }
        match self.tip {
            Some(tip) => resp.on_hover_text(tip),
            None => resp,
        }
    }
}

pub fn button(ui: &mut Ui, text: &str) -> Response {
    Button::secondary(text).show(ui)
}

/// A square icon key: bare at rest, outlined on hover, lit when pressed.
/// `tip` is both the tooltip and the accessible name.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    icon_button_sized(ui, icon, tip, tk(ui).ctrl)
}

pub fn icon_button_sized(ui: &mut Ui, icon: Icon, tip: &str, size: f32) -> Response {
    let t = tk(ui);
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), sense);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, tip));
    if ui.is_rect_visible(rect) {
        let hover = anim(ui, resp.id.with("hover"), resp.hovered() && enabled);
        let pressed = enabled && resp.is_pointer_button_down_on();
        let look = if !enabled {
            KeyLook { fill: None, edge: None, fg: t.border }
        } else if pressed {
            KeyLook { fill: Some(t.pri_bg), edge: Some(t.pri_border), fg: t.pri_fg }
        } else {
            KeyLook {
                fill: (hover > 0.0).then(|| t.dk_fill.gamma_multiply(hover)),
                edge: (hover > 0.0).then(|| t.border_strong.gamma_multiply(hover)),
                fg: mix(t.text2, t.text, hover),
            }
        };
        let p = ui.painter();
        paint_key(p, rect, &look, t);
        icons::paint_at(p, icon, rect.center(), t.icon_size, look.fg);
        if focus_visible(&resp) {
            focus_ring(p, rect, t);
        }
    }
    resp.on_hover_text(tip)
}

/// Menus are one width, so every row's hover band lines up.
const MENU_W: f32 = 240.0;

/// A popup's body: the overlay frame with two clipped corners.
fn menu_body(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let t = tk(ui);
    framed(
        ui,
        Margin::same(t.s(1) as i8),
        |rect| clipped_shape(rect, t.s(2), t.surface, Stroke::new(t.bw, t.border_strong)),
        |ui| {
            ui.set_min_width(MENU_W);
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        },
    );
}

/// The ⋮ key that opens a row's menu.
pub fn more_menu(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> Response {
    let resp = icon_button(ui, Icon::More, "More actions");
    egui::Popup::menu(&resp).frame(egui::Frame::NONE).show(|ui| menu_body(ui, add));
    resp
}

/// The right-click menu of a row, in the same frame as [`more_menu`].
pub fn context_menu(resp: &Response, add: impl FnOnce(&mut Ui)) {
    egui::Popup::context_menu(resp).frame(egui::Frame::NONE).show(|ui| menu_body(ui, add));
}

/// One row of a menu: icon and label, the whole width. Closes the menu.
pub fn menu_item(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let t = tk(ui);
    let g = galley(ui, text, FontId::proportional(t.fs));
    let w = (t.s(3) * 2.0 + t.icon_size + t.s(2) + g.size().x).max(MENU_W);
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, t.ctrl + t.s(1)), sense);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
    let p = ui.painter();
    let hot = enabled && (resp.hovered() || resp.has_focus());
    if hot {
        p.rect_filled(rect, cr(t.r_sm), t.dk_fill2);
    }
    let fg = if enabled { t.text } else { t.border_strong };
    let x = rect.left() + t.s(3);
    if let Some(i) = icon {
        let c = if !enabled {
            t.border_strong
        } else if hot {
            t.text
        } else {
            t.text2
        };
        icons::paint_at(p, i, pos2(x + t.icon_size / 2.0, rect.center().y), t.icon_size, c);
    }
    let tx = x + t.icon_size + t.s(2);
    p.galley(pos2(tx, rect.center().y - g.size().y / 2.0), g, fg);
    if focus_visible(&resp) {
        p.rect_stroke(rect.shrink(1.0), cr(t.r_sm), Stroke::new(2.0, t.accent), StrokeKind::Inside);
    }
    if resp.clicked() {
        ui.close();
    }
    resp
}

/// The Ctrl/Shift state of this frame's click, taken from the click event
/// itself: a quick Ctrl+click can release Ctrl within the same frame.
pub fn click_modifiers(ui: &Ui) -> egui::Modifiers {
    ui.input(|i| {
        i.events
            .iter()
            .rev()
            .find_map(|e| match e {
                egui::Event::PointerButton { pressed: false, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(i.modifiers)
    })
}

// ---------------------------------------------------------------------------
// Selection controls
// ---------------------------------------------------------------------------

/// A choice key: `main` text with an optional quieter `note`; a note that is
/// a value (a size, a frame rate) is set in monospace, words are not.
/// Off: outlined. On: lit solid with a check mark (colour is never the only
/// cue). The width is the same either way, so a row of them never shifts.
pub fn chip(ui: &mut Ui, selected: bool, main: &str, note: &str) -> Response {
    let t = tk(ui);
    let gm = galley(ui, main, FontId::new(t.fs_sm, bold()));
    let is_value = note.chars().next().is_some_and(|c| c.is_ascii_digit() || matches!(c, '≈' | '~' | '+'));
    let note_font = if is_value { FontId::monospace(t.fs_xs) } else { FontId::proportional(t.fs_xs) };
    let gn = (!note.is_empty()).then(|| galley(ui, note, note_font));
    let check = t.icon_size;
    let text_w = gm.size().x + gn.as_ref().map_or(0.0, |g| t.s(1) + 2.0 + g.size().x);
    let size = vec2(t.padx * 2.0 + check + t.s(1) + text_w, t.ctrl);
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let label = if note.is_empty() { main.to_string() } else { format!("{main} {note}") };
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &label));
    if ui.is_rect_visible(rect) {
        let hover = anim(ui, resp.id.with("hover"), resp.hovered() && enabled);
        let pressed = enabled && resp.is_pointer_button_down_on();
        let (look, fg2) = if selected {
            let fill =
                if !enabled { t.border_strong } else { mix(t.accent, t.bg, if pressed { 0.28 } else { 0.1 * hover }) };
            (KeyLook { fill: Some(fill), edge: None, fg: t.on_accent }, mix(t.on_accent, t.accent, 0.3))
        } else if !enabled {
            (KeyLook { fill: None, edge: Some(t.border), fg: t.border_strong }, t.border_strong)
        } else {
            let fill = if pressed { Some(t.dk_fill2) } else { (hover > 0.0).then(|| t.dk_fill.gamma_multiply(hover)) };
            (KeyLook { fill, edge: Some(mix(t.border_strong, t.text, hover)), fg: t.text }, t.text2)
        };
        let p = ui.painter();
        paint_key(p, rect, &look, t);
        let cy = rect.center().y + if pressed { 1.0 } else { 0.0 };
        let mut x = if selected {
            let x = rect.left() + t.padx;
            icons::paint_at(p, Icon::Check, pos2(x + check / 2.0, cy), check, look.fg);
            x + check + t.s(1)
        } else {
            rect.center().x - text_w / 2.0
        };
        let gms = gm.size();
        p.galley(pos2(x, cy - gms.y / 2.0), gm, look.fg);
        x += gms.x + t.s(1) + 2.0;
        if let Some(g) = gn {
            p.galley(pos2(x, cy - g.size().y / 2.0), g, fg2);
        }
        if focus_visible(&resp) {
            focus_ring(p, rect, t);
        }
    }
    resp
}

/// A tick box: an outlined square with a 3:1 edge; on = a lit square inside
/// (a shape change, not just a colour change).
pub fn tick_box(p: &Painter, b: Rect, on: bool, enabled: bool, hovered: bool, t: &Tokens) {
    let edge = if !enabled {
        t.border
    } else if on || hovered {
        t.text
    } else {
        t.border_strong
    };
    p.rect_filled(b, cr(t.r_sm), t.sunken);
    p.rect_stroke(b, cr(t.r_sm), Stroke::new(t.bw, edge), StrokeKind::Inside);
    if on {
        let fill = if enabled { t.accent } else { t.border_strong };
        p.rect_filled(b.shrink(3.0), cr(1.0), fill);
    }
}

/// A checkbox: [`tick_box`] and a label; the whole row toggles it.
pub fn checkbox(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let t = tk(ui);
    let g = galley(ui, label, FontId::proportional(t.fs));
    let bx = t.s(4);
    let size = vec2(bx + if label.is_empty() { 0.0 } else { t.s(2) + g.size().x }, t.ctrl);
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(size, sense);
    if resp.clicked() {
        *value = !*value;
        resp.mark_changed();
    }
    let on = *value;
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, label));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let b = Rect::from_center_size(pos2(rect.left() + bx / 2.0, rect.center().y), Vec2::splat(bx));
        tick_box(p, b, on, enabled, resp.hovered(), t);
        let fg = if enabled { t.text } else { t.border_strong };
        p.galley(pos2(b.right() + t.s(2), rect.center().y - g.size().y / 2.0), g, fg);
        if focus_visible(&resp) {
            focus_ring(p, b, t);
        }
    }
    resp
}

/// A row of joined keys; the chosen one is lit with an uppercase label.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    let t = tk(ui);
    let galleys: Vec<_> = options.iter().map(|(_, l)| key_galley(ui, l)).collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + t.padx * 2.0).collect();
    let size = vec2(widths.iter().sum::<f32>(), t.ctrl);
    let (rect, mut group) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter().clone();
    let mut x = rect.left();
    for (i, (((opt, label), w), g)) in options.iter().zip(widths).zip(galleys).enumerate() {
        let seg = Rect::from_min_size(pos2(x, rect.top()), vec2(w, size.y));
        x += w;
        let id = group.id.with(label);
        let resp = ui.interact(seg, id, Sense::click());
        let on = *value == *opt;
        resp.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, on, *label));
        if resp.clicked() && !on {
            *value = *opt;
            group.mark_changed();
        }
        let fg = if on {
            p.rect_filled(seg.shrink(2.0), cr(t.r_sm), t.accent);
            t.on_accent
        } else {
            if resp.hovered() {
                p.rect_filled(seg.shrink(2.0), cr(t.r_sm), t.dk_fill2);
            }
            if i > 0 {
                p.vline(seg.left().round() + 0.5, seg.y_range().shrink(t.s(2)), Stroke::new(t.bw, t.dk_line));
            }
            if resp.hovered() { t.text } else { t.text2 }
        };
        p.galley(seg.center() - g.size() / 2.0, g, fg);
        if focus_visible(&resp) {
            focus_ring(&p, seg, t);
        }
    }
    p.rect_stroke(rect, cr(t.r_btn), Stroke::new(t.bw, t.border_strong), StrokeKind::Inside);
    group
}

/// A top-level tab: uppercase, widely tracked. On: full-strength label with a
/// 2 px accent bar under it. Off: --text2, a hairline under it on hover.
pub fn tab(ui: &mut Ui, selected: bool, icon: Icon, text: &str) -> Response {
    let t = tk(ui);
    let g = galley_spaced(ui, &text.to_uppercase(), FontId::new(t.fs_sm, bold()), t.tracking(t.fs_sm));
    let size = vec2(t.padx * 2.0 + t.icon_size + t.s(2) + g.size().x, t.ctrl + t.s(1));
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text));
    if ui.is_rect_visible(rect) {
        let hover = anim(ui, resp.id.with("hover"), resp.hovered() && !selected);
        let p = ui.painter();
        let fg = if selected { t.text } else { mix(t.text2, t.text, hover) };
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max);
        if selected {
            p.rect_filled(bar, CornerRadius::ZERO, t.accent);
        } else if hover > 0.0 {
            p.hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(t.bw, t.dk_line.gamma_multiply(hover)));
        }
        let x = rect.left() + t.padx;
        let cy = rect.center().y - 1.0;
        icons::paint_at(p, icon, pos2(x + t.icon_size / 2.0, cy), t.icon_size, fg);
        p.galley(pos2(x + t.icon_size + t.s(2), cy - g.size().y / 2.0), g, fg);
        if focus_visible(&resp) {
            focus_ring(p, rect, t);
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

fn field_shapes(rect: Rect, focused: bool, hovered: bool, t: &Tokens) -> Shape {
    let edge = if focused {
        Stroke::new(2.0, t.accent)
    } else if hovered {
        Stroke::new(t.bw, t.text2)
    } else {
        Stroke::new(t.bw, t.border_strong)
    };
    Shape::Vec(vec![
        Shape::rect_filled(rect, cr(t.r_sm), t.sunken),
        Shape::rect_stroke(rect, cr(t.r_sm), edge, StrokeKind::Inside),
    ])
}

/// A one-line text field in monospace: a sunken well with a 3:1 edge that
/// thickens to a 2 px accent edge while typing. Configure `edit` as usual.
pub fn input(ui: &mut Ui, edit: egui::TextEdit<'_>, icon: Option<Icon>, width: f32) -> Response {
    let t = tk(ui);
    let (rect, outer) = ui.allocate_exact_size(vec2(width, t.ctrl), Sense::click());
    let idx = ui.painter().add(Shape::Noop);
    let left = t.s(2) + icon.map_or(0.0, |_| t.icon_size + t.s(2));
    let inner = Rect::from_min_max(pos2(rect.left() + left, rect.top()), pos2(rect.right() - t.s(2), rect.bottom()));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let resp = child.add(
        edit.frame(egui::Frame::NONE)
            .font(egui::TextStyle::Monospace)
            .desired_width(inner.width())
            .vertical_align(Align::Center),
    );
    if outer.clicked() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    let hovered = resp.hovered() || outer.hovered();
    ui.painter().set(idx, field_shapes(rect, focused, hovered, t));
    if let Some(i) = icon {
        let c = if focused { t.text } else { t.text2 };
        icons::paint_at(
            ui.painter(),
            i,
            pos2(rect.left() + t.s(2) + t.icon_size / 2.0, rect.center().y),
            t.icon_size,
            c,
        );
    }
    resp
}

/// A multi-line monospace field of a fixed size, scrolling inside the well.
pub fn text_area(ui: &mut Ui, edit: egui::TextEdit<'_>, size: Vec2, id_salt: &str) -> Response {
    let t = tk(ui);
    let (rect, outer) = ui.allocate_exact_size(size, Sense::click());
    let idx = ui.painter().add(Shape::Noop);
    let inner = rect.shrink2(vec2(t.s(2), t.s(2)));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    let resp = egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .max_height(inner.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let rows = (inner.height() / ui.text_style_height(&egui::TextStyle::Monospace)).floor().max(1.0);
            ui.add(
                edit.frame(egui::Frame::NONE)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .desired_rows(rows as usize),
            )
        })
        .inner;
    if outer.clicked() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    ui.painter().set(idx, field_shapes(rect, focused, outer.hovered() || resp.hovered(), t));
    resp
}

/// egui's slider with a key-shaped handle sized to the rail.
pub fn slider(ui: &mut Ui, slider: egui::Slider<'_>) -> Response {
    let t = tk(ui);
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = t.s(5);
        ui.add(slider)
    })
    .inner
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

fn framed<R>(
    ui: &mut Ui,
    margin: Margin,
    paint: impl FnOnce(Rect) -> Shape,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let idx = ui.painter().add(Shape::Noop);
    let inner = egui::Frame::new().inner_margin(margin).show(ui, add);
    ui.painter().set(idx, paint(inner.response.rect));
    inner
}

/// A dialog: the overlay frame (1 px, two clipped corners, no shadow) over
/// a dimmed page.
pub fn modal<R>(ctx: &egui::Context, id: egui::Id, add: impl FnOnce(&mut Ui) -> R) -> egui::ModalResponse<R> {
    let t = tk_ctx(ctx);
    egui::Modal::new(id).frame(egui::Frame::NONE).backdrop_color(t.bg.gamma_multiply(0.8)).show(ctx, |ui| {
        framed(
            ui,
            Margin::same(t.s(6) as i8),
            |rect| clipped_shape(rect, t.s(3), t.surface, Stroke::new(t.bw, t.border_strong)),
            add,
        )
        .inner
    })
}

/// A toast: the overlay frame, small.
pub fn toast_frame<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    framed(
        ui,
        Margin::symmetric(t.s(4) as i8, t.s(3) as i8),
        |rect| clipped_shape(rect, t.s(2), t.surface, Stroke::new(t.bw, t.border_strong)),
        add,
    )
}

/// The main work area: a 1 px frame with two clipped corners filling the
/// rest of the panel. Content gets 16 px inside it.
pub fn work_area<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let t = tk(ui);
    let rect = ui.available_rect_before_wrap();
    ui.painter().add(clipped_shape(rect, t.s(3), t.surface, Stroke::new(t.bw, t.dk_line)));
    let inner = rect.shrink2(vec2(t.s(4), t.s(3)));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    child.shrink_clip_rect(rect.shrink(1.0));
    let r = add(&mut child);
    ui.advance_cursor_after_rect(rect);
    r
}

/// A module: no box, just the hairline mark (with end ticks) on top and an
/// optional small uppercase title under it.
pub fn module<R>(ui: &mut Ui, title: Option<(Icon, &str)>, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    ui.vertical(|ui| {
        let w = ui.available_width();
        let (mark, _) = ui.allocate_exact_size(vec2(w, t.s(2)), Sense::hover());
        rule(ui.painter(), mark.left(), mark.right(), mark.top(), t);
        if let Some((icon, title)) = title {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.s(2);
                icons::show(ui, icon, t.icon_size, t.text2);
                ui.label(module_title(ui, title));
            });
            ui.add_space(t.s(2));
        }
        add(ui)
    })
}

/// A module without a title: the hairline mark, then the content.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    module(ui, None, add)
}

/// A notice above a tab: a 1 px box (the one boxed thing on a page, so it
/// reads as system status).
pub fn banner<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    framed(
        ui,
        Margin::symmetric(t.s(3) as i8, t.s(1) as i8),
        |rect| {
            Shape::Vec(vec![
                Shape::rect_filled(rect, cr(t.r_md), t.sunken),
                Shape::rect_stroke(rect, cr(t.r_md), Stroke::new(t.bw, t.border_strong), StrokeKind::Inside),
            ])
        },
        add,
    )
}

/// A sunken well: lists and pickers sit in one.
pub fn well<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    framed(
        ui,
        Margin::same(t.s(1) as i8),
        |rect| {
            Shape::Vec(vec![
                Shape::rect_filled(rect, cr(t.r_md), t.sunken),
                Shape::rect_stroke(rect, cr(t.r_md), Stroke::new(t.bw, t.border), StrokeKind::Inside),
            ])
        },
        add,
    )
}

/// Paints one list row and returns the rect for its content. Rows sit on
/// hairlines; hover fills faintly; a chosen row fills with --accent-soft
/// and gets a 2 px accent marker on the left.
pub fn list_row(ui: &Ui, rect: Rect, resp: &Response, selected: bool) -> Rect {
    let t = tk(ui);
    let p = ui.painter();
    let hover = anim(ui, resp.id.with("hover"), resp.hovered() && !selected);
    if selected {
        p.rect_filled(rect, CornerRadius::ZERO, t.accent_soft);
        p.rect_filled(Rect::from_min_size(rect.min, vec2(2.0, rect.height())), CornerRadius::ZERO, t.accent);
    } else if hover > 0.0 {
        p.rect_filled(rect, CornerRadius::ZERO, t.dk_fill.gamma_multiply(hover));
    }
    p.hline(rect.x_range(), rect.bottom().round() - 0.5, Stroke::new(t.bw, t.border));
    if focus_visible(resp) {
        p.rect_stroke(rect.shrink(1.0), cr(t.r_sm), Stroke::new(2.0, t.accent), StrokeKind::Inside);
    }
    rect.shrink2(vec2(t.s(3), t.s(1)))
}

/// Secondary text on a row: --text2, or --text on a chosen row, where
/// --text2 on --accent-soft would fall under 4.5:1.
pub fn sub_color(t: &Tokens, selected: bool) -> Color32 {
    if selected { t.text } else { t.text2 }
}

// ---------------------------------------------------------------------------
// Feedback
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Running: the line colour at half strength, so "busy" never looks
    /// like "chosen".
    Busy,
    Done,
    Failed,
    Paused,
    Idle,
}

pub fn tone_color(t: &Tokens, tone: Tone) -> Color32 {
    match tone {
        Tone::Busy => t.text2,
        Tone::Done => t.success_text,
        Tone::Failed => t.danger_text,
        Tone::Paused => t.warning_text,
        Tone::Idle => t.border_strong,
    }
}

/// A 4 px meter on a dim track. `busy` sweeps for unknown progress (a still
/// segment with reduced motion).
pub fn progress(ui: &mut Ui, width: f32, frac: f32, tone: Tone, busy: bool) {
    let t = tk(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, t.s(1)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::ZERO, t.dk_dim);
    let color = tone_color(t, tone);
    if busy {
        let w = rect.width() * 0.3;
        let x = if super::theme::reduced_motion() {
            rect.left()
        } else {
            ui.ctx().request_repaint();
            let time = ui.input(|i| i.time) as f32;
            rect.left() + ((time * 0.6).fract() * (rect.width() + w)) - w
        };
        let seg =
            Rect::from_min_max(pos2(x.max(rect.left()), rect.top()), pos2((x + w).min(rect.right()), rect.bottom()));
        if seg.width() > 0.0 {
            p.rect_filled(seg, CornerRadius::ZERO, color);
        }
    } else if frac > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2(rect.width() * frac.clamp(0.0, 1.0), rect.height()));
        p.rect_filled(fill, CornerRadius::ZERO, color);
    }
}

/// A small square status pip. Always sits next to words that say the same.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().rect_filled(Rect::from_center_size(rect.center(), Vec2::splat(6.0)), CornerRadius::ZERO, color);
}

/// A thumbnail cropped to fill `size`, or a sunken placeholder with an icon.
pub fn thumb(ui: &mut Ui, tex: Option<&TextureHandle>, size: Vec2, icon: Icon) -> Response {
    let t = tk(ui);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    match tex {
        Some(tex) => {
            let [w, h] = tex.size();
            let (w, h) = (w as f32, h as f32);
            let target = size.x / size.y;
            let uv = if w / h > target {
                let keep = (h * target) / w;
                Rect::from_min_max(pos2((1.0 - keep) / 2.0, 0.0), pos2((1.0 + keep) / 2.0, 1.0))
            } else {
                let keep = (w / target) / h;
                Rect::from_min_max(pos2(0.0, (1.0 - keep) / 2.0), pos2(1.0, (1.0 + keep) / 2.0))
            };
            egui::Image::from_texture((tex.id(), size)).uv(uv).corner_radius(cr(t.r_sm)).paint_at(ui, rect);
        }
        None => {
            let p = ui.painter();
            p.rect_filled(rect, cr(t.r_sm), t.sunken);
            p.rect_stroke(rect, cr(t.r_sm), Stroke::new(t.bw, t.border), StrokeKind::Inside);
            let s = (size.y * 0.4).clamp(t.icon_size, t.icon_size * 2.0);
            icons::paint_at(p, icon, rect.center(), s, t.text2);
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------

fn heading_rt(ui: &Ui, text: &str, size: f32) -> RichText {
    let t = tk(ui);
    let size = size * t.heading_scale;
    RichText::new(text.to_uppercase()).family(head()).size(size).color(t.text).extra_letter_spacing(t.tracking(size))
}

/// Screen title: --fs-xl, Saira 500, uppercase, tracked .12 em.
pub fn title_text(ui: &Ui, text: &str) -> RichText {
    heading_rt(ui, text, tk(ui).fs_xl)
}

/// Section heading: --fs-lg, Saira 500, uppercase, tracked .12 em.
pub fn heading_text(ui: &Ui, text: &str) -> RichText {
    heading_rt(ui, text, tk(ui).fs_lg)
}

/// A content title (a video, a file, a search): --fs-lg, Saira 500, kept in
/// its own case, because names shouldn't be rewritten.
pub fn content_title(ui: &Ui, text: &str) -> RichText {
    let t = tk(ui);
    RichText::new(text).family(head()).size(t.fs_lg).color(t.text)
}

pub fn title(ui: &mut Ui, text: &str) -> Response {
    let rt = title_text(ui, text);
    ui.label(rt)
}

pub fn heading(ui: &mut Ui, text: &str) -> Response {
    let rt = heading_text(ui, text);
    ui.label(rt)
}

/// A heading with a leading icon.
pub fn icon_heading(ui: &mut Ui, icon: Icon, text: &str) {
    let t = tk(ui);
    ui.horizontal(|ui| {
        icons::show(ui, icon, t.icon_size, t.text2);
        heading(ui, text);
    });
}

/// A module title: --fs-xs (12 px, the floor for small caps), 600,
/// uppercase, tracked .12 em, full strength.
pub fn module_title(ui: &Ui, text: &str) -> RichText {
    let t = tk(ui);
    RichText::new(text.to_uppercase())
        .family(bold())
        .size(t.fs_xs)
        .color(t.text)
        .extra_letter_spacing(t.tracking(t.fs_xs))
}

/// A field label: --fs-xs, 600, uppercase, tracked .12 em, --text2.
pub fn field_label(ui: &Ui, text: impl Into<String>) -> RichText {
    let t = tk(ui);
    RichText::new(text.into().to_uppercase())
        .family(bold())
        .size(t.fs_xs)
        .color(t.text2)
        .extra_letter_spacing(t.tracking(t.fs_xs))
}

/// Body text in the label weight (600).
pub fn strong(ui: &Ui, text: impl Into<String>) -> RichText {
    let t = tk(ui);
    RichText::new(text).family(bold()).size(t.fs).color(t.text)
}

/// Secondary text: --fs-sm, --text2.
pub fn muted(ui: &Ui, text: impl Into<String>) -> RichText {
    let t = tk(ui);
    RichText::new(text).size(t.fs_sm).color(t.text2)
}

/// Captions and fine print: --fs-xs, --text2.
pub fn caption(ui: &Ui, text: impl Into<String>) -> RichText {
    let t = tk(ui);
    RichText::new(text).size(t.fs_xs).color(t.text2)
}

/// Values (sizes, speeds, times, versions, paths): Fira Code, --fs-xs, --text2.
pub fn mono(ui: &Ui, text: impl Into<String>) -> RichText {
    let t = tk(ui);
    RichText::new(text).family(FontFamily::Monospace).size(t.fs_xs).color(t.text2)
}

pub fn colored(ui: &Ui, text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(tk(ui).fs_sm).color(color)
}

/// A wrapped paragraph, capped near 75 characters a line.
pub fn para(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let t = tk(ui);
    let max = (t.fs * 0.5 * 75.0).min(ui.available_width());
    ui.scope(|ui| {
        ui.set_max_width(max);
        ui.add(egui::Label::new(text.into()).wrap())
    })
    .inner
}

pub fn hint(ui: &mut Ui, text: &str) -> Response {
    let t = tk(ui);
    let rt = RichText::new(text).size(t.fs_sm).color(t.text2).line_height(Some(t.fs_sm * t.lh));
    para(ui, rt)
}

// ---------------------------------------------------------------------------
// Settings layout
// ---------------------------------------------------------------------------

/// The gap between groups inside a section (8 px).
pub const SECTION_GAP: f32 = Tokens::dark().s(2);

/// A section of the settings page: a module with an icon and a title.
pub fn section(ui: &mut Ui, icon: Icon, title: &str, add: impl FnOnce(&mut Ui)) {
    let t = tk(ui);
    module(ui, Some((icon, title)), |ui| {
        ui.set_width(ui.available_width());
        add(ui);
    });
    ui.add_space(t.s(6));
}

/// One labelled row in a settings section.
pub fn setting_row(ui: &mut Ui, label: &str, hint_text: &str, add: impl FnOnce(&mut Ui)) {
    let t = tk(ui);
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(200.0, t.ctrl), Sense::hover());
        let g = ui.painter().layout(label.to_owned(), FontId::proportional(t.fs), t.text, rect.width() - t.s(2));
        ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.text);
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| add(ui));
            if !hint_text.is_empty() {
                hint(ui, hint_text);
            }
        });
    });
    ui.add_space(t.s(1));
}
