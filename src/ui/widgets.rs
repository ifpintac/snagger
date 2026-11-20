//! Snag's components, drawn in the Neumorphism language.
//!
//! Depth has three levels: inset (inputs, wells, pressed controls), raised low
//! (controls) and raised mid (cards). Raised = a light shadow up-left and a
//! dark one down-right; inset = the same pair inside the edge. State is never
//! shown by depth alone: "on" also changes colour and adds an icon or weight,
//! inputs carry a 3:1 edge, and keyboard focus draws a 2 px accent ring.

use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, InnerResponse, Layout, Margin, Painter, Pos2, Rect, Response, RichText,
    Sense, Shape, Stroke, StrokeKind, TextureHandle, Ui, UiBuilder, Vec2, WidgetInfo, WidgetText, WidgetType,
    epaint::Mesh, epaint::RectShape, pos2, vec2,
};

use super::icons::{self, Icon};
use super::theme::{Tokens, bold, tk};

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

/// Hover/press blend, 160 ms (instant with reduced motion).
fn anim(ui: &Ui, id: egui::Id, on: bool) -> f32 {
    let t = tk(ui);
    let dur = if super::theme::reduced_motion() { 0.0 } else { t.dur_fast };
    ui.ctx().animate_bool_with_time(id, on, dur)
}

/// The two outer shadows of a raised shape. `lift` runs from 0 (controls)
/// to 1 (cards); `alpha` fades them in for hover-only depth.
fn raised_shadows(rect: Rect, r: f32, lift: f32, alpha: f32, t: &Tokens) -> [Shape; 2] {
    let o = 2.0 + 4.0 * lift;
    let blur = 5.0 + 9.0 * lift;
    let light =
        RectShape::filled(rect.translate(vec2(-o, -o)), cr(r), t.nm_l.gamma_multiply(alpha)).with_blur_width(blur);
    let dark = RectShape::filled(rect.translate(vec2(o, o)), cr(r), t.nm_d.gamma_multiply(alpha)).with_blur_width(blur);
    [light.into(), dark.into()]
}

pub fn raised(p: &Painter, rect: Rect, r: f32, fill: Color32, lift: f32, t: &Tokens) {
    p.extend(raised_shadows(rect, r, lift, 1.0, t));
    p.rect_filled(rect, cr(r), fill);
}

/// A raised surface whose shadows fade in with `alpha` (hover-only depth).
pub fn raised_faded(p: &Painter, rect: Rect, r: f32, fill: Color32, alpha: f32, t: &Tokens) {
    p.extend(raised_shadows(rect, r, 0.0, alpha, t));
    p.rect_filled(rect, cr(r), fill);
}

/// The outline of a rounded rectangle with each point's outward normal.
fn outline(rect: Rect, r: f32) -> Vec<(Pos2, Vec2)> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let seg = ((r / 2.0) as usize).clamp(2, 12);
    let corners = [
        (pos2(rect.left() + r, rect.top() + r), PI),
        (pos2(rect.right() - r, rect.top() + r), 1.5 * PI),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), 0.5 * PI),
    ];
    let mut out = Vec::with_capacity(4 * (seg + 1));
    for (c, a0) in corners {
        for i in 0..=seg {
            let a = a0 + (i as f32 / seg as f32) * 0.5 * PI;
            let n = vec2(a.cos(), a.sin());
            out.push((c + n * r, n));
        }
    }
    out
}

/// A sunken fill with the inner shadow pair: dark along the top-left inner
/// edge, light along the bottom-right, fading inwards over `depth`.
fn inset_shapes(rect: Rect, r: f32, fill: Color32, depth: f32, t: &Tokens) -> Vec<Shape> {
    let mut shapes: Vec<Shape> = vec![RectShape::filled(rect, cr(r), fill).into()];
    let pts = outline(rect.shrink(0.5), r);
    let up_left = vec2(-1.0, -1.0).normalized();
    // (fraction of depth, fraction of edge strength)
    let rings = [(0.0, 1.0), (0.45, 0.55), (1.0, 0.0)];
    let mut mesh = Mesh::default();
    for &(p, n) in &pts {
        let d = n.dot(up_left);
        let edge = if d > 0.0 { t.nm_d.gamma_multiply(d) } else { t.nm_l.gamma_multiply(-d * 0.9) };
        for (f, s) in rings {
            mesh.colored_vertex(p - n * depth * f, edge.gamma_multiply(s));
        }
    }
    let k = rings.len() as u32;
    let count = pts.len() as u32;
    for i in 0..count {
        let j = (i + 1) % count;
        for ring in 0..k - 1 {
            let (a, b) = (i * k + ring, j * k + ring);
            mesh.add_triangle(a, b, b + 1);
            mesh.add_triangle(a, b + 1, a + 1);
        }
    }
    shapes.push(Shape::mesh(mesh));
    shapes
}

pub fn inset(p: &Painter, rect: Rect, r: f32, fill: Color32, depth: f32, t: &Tokens) {
    p.extend(inset_shapes(rect, r, fill, depth, t));
}

fn focus_ring(p: &Painter, rect: Rect, r: f32, t: &Tokens) {
    p.rect_stroke(rect.expand(3.0), cr(r + 3.0), Stroke::new(2.0, t.accent), StrokeKind::Middle);
}

fn flat_disabled(p: &Painter, rect: Rect, r: f32, t: &Tokens) {
    p.rect_filled(rect, cr(r), t.sunken);
    p.rect_stroke(rect, cr(r), Stroke::new(1.0, t.border), StrokeKind::Inside);
}

fn galley(ui: &Ui, text: &str, font: FontId) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER)
}

// ---------------------------------------------------------------------------
// Buttons
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Solid accent pill: the one main action on a screen or dialog.
    Primary,
    /// Raised pill.
    Secondary,
    /// Accent text, no surface: low-emphasis actions.
    Quiet,
    /// Solid danger pill, for confirming something destructive.
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
        let g = galley(ui, self.text, FontId::new(t.fs, bold()));
        let has_icon = self.icon.is_some() || self.loading;
        let icon_w = if has_icon { t.icon_size + if self.text.is_empty() { 0.0 } else { t.s(2) } } else { 0.0 };
        let content_w = icon_w + g.size().x;
        let padx = if self.kind == Kind::Quiet { t.s(2) } else { t.padx + t.s(1) };
        let size = vec2((padx * 2.0 + content_w).max(self.min_width), t.ctrl);
        let sense = if enabled { Sense::click() } else { Sense::hover() };
        let (rect, resp) = ui.allocate_exact_size(size, sense);
        let text = self.text;
        resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
        if ui.is_rect_visible(rect) {
            let hover = anim(ui, resp.id.with("hover"), resp.hovered() && enabled);
            let pressed = enabled && resp.is_pointer_button_down_on();
            let r = t.r_btn.min(size.y / 2.0);
            let p = ui.painter();
            let fg = match self.kind {
                Kind::Primary | Kind::Danger => {
                    let (base, fg) =
                        if self.kind == Kind::Primary { (t.pri_bg, t.pri_fg) } else { (t.danger, t.on_accent) };
                    if !enabled && !self.loading {
                        flat_disabled(p, rect, r, t);
                        t.text2
                    } else {
                        if !pressed {
                            p.extend(raised_shadows(rect, r, 0.35 * hover, 1.0, t));
                        }
                        let fill = if pressed { mix(base, t.text, 0.18) } else { mix(base, t.nm_l, 0.08 * hover) };
                        p.rect_filled(rect, cr(r), fill);
                        fg
                    }
                }
                Kind::Secondary => {
                    if !enabled && !self.loading {
                        flat_disabled(p, rect, r, t);
                        t.text2
                    } else if pressed {
                        inset(p, rect, r, t.sunken, 5.0, t);
                        t.text
                    } else {
                        raised(p, rect, r, t.surface, 0.35 * hover, t);
                        t.text
                    }
                }
                Kind::Quiet => {
                    if pressed {
                        inset(p, rect, r, t.sunken, 4.0, t);
                    }
                    if enabled { t.accent_text } else { t.text2 }
                }
            };
            let cy = rect.center().y;
            let mut x = rect.center().x - content_w / 2.0;
            if self.loading {
                let s = Rect::from_center_size(pos2(x + t.icon_size / 2.0, cy), Vec2::splat(t.icon_size - 4.0));
                egui::Spinner::new().size(s.width()).color(fg).paint_at(ui, s);
            } else if let Some(i) = self.icon {
                icons::paint_at(p, i, pos2(x + t.icon_size / 2.0, cy), t.icon_size, fg);
            }
            x += icon_w;
            let gs = g.size();
            p.galley(pos2(x, cy - gs.y / 2.0), g, fg);
            if self.kind == Kind::Quiet && hover > 0.5 {
                p.hline(x..=x + gs.x, cy + gs.y / 2.0 - 1.0, Stroke::new(1.0, fg));
            }
            if focus_visible(&resp) {
                focus_ring(p, rect, r, t);
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

/// A 36 px round icon button: flat at rest, raised on hover, pressed in when
/// clicked. `tip` is both the tooltip and the accessible name.
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
        let r = size / 2.0;
        let p = ui.painter();
        if pressed {
            inset(p, rect, r, t.sunken, 4.0, t);
        } else if hover > 0.0 {
            p.extend(raised_shadows(rect, r, 0.0, hover, t));
            p.rect_filled(rect, cr(r), t.surface);
        }
        let color = if !enabled { t.border } else { mix(t.text2, t.text, hover) };
        icons::paint_at(p, icon, rect.center(), t.icon_size, color);
        if focus_visible(&resp) {
            focus_ring(p, rect, r, t);
        }
    }
    resp.on_hover_text(tip)
}

/// Menus are one width, so every row's hover shape lines up.
const MENU_W: f32 = 260.0;

/// The ⋯ button that opens a row's menu.
pub fn more_menu(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> Response {
    let resp = icon_button(ui, Icon::More, "More actions");
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(MENU_W);
        add(ui)
    });
    resp
}

/// One row of a menu: icon and label, the whole width. Closes the menu.
pub fn menu_item(ui: &mut Ui, icon: Option<Icon>, text: &str) -> Response {
    let t = tk(ui);
    let g = galley(ui, text, FontId::proportional(t.fs));
    let w = (t.s(3) * 2.0 + t.icon_size + t.s(3) + g.size().x).max(MENU_W);
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(w, t.s(8)), sense);
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, text));
    let p = ui.painter();
    let hot = enabled && (resp.hovered() || resp.has_focus());
    if hot {
        p.rect_filled(rect, cr(t.r_sm), t.sunken);
    }
    let fg = if enabled { t.text } else { t.text2 };
    let x = rect.left() + t.s(3);
    if let Some(i) = icon {
        icons::paint_at(
            p,
            i,
            pos2(x + t.icon_size / 2.0, rect.center().y),
            t.icon_size,
            if hot { t.text } else { t.text2 },
        );
    }
    let tx = x + t.icon_size + t.s(3);
    p.galley(pos2(tx, rect.center().y - g.size().y / 2.0), g, fg);
    if focus_visible(&resp) {
        focus_ring(p, rect, t.r_sm, t);
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

/// A choice chip: `main` text with an optional quieter note. Off: raised.
/// On: accent fill with a check mark (colour is never the only cue).
pub fn chip(ui: &mut Ui, selected: bool, main: &str, note: &str) -> Response {
    let t = tk(ui);
    let gm = galley(ui, main, FontId::new(t.fs, bold()));
    let gn = (!note.is_empty()).then(|| galley(ui, note, FontId::proportional(t.fs_xs)));
    let check = t.s(4);
    let text_w = gm.size().x + gn.as_ref().map_or(0.0, |g| t.s(1) + 2.0 + g.size().x);
    let size = vec2(t.s(3) * 2.0 + check + t.s(1) + text_w, t.s(8));
    let enabled = ui.is_enabled();
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let label = if note.is_empty() { main.to_string() } else { format!("{main} {note}") };
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, &label));
    if ui.is_rect_visible(rect) {
        let hover = anim(ui, resp.id.with("hover"), resp.hovered() && enabled);
        let pressed = enabled && resp.is_pointer_button_down_on();
        let r = size.y / 2.0;
        let p = ui.painter();
        let (fg, fg2) = if selected {
            let fill = if pressed { mix(t.accent, t.text, 0.18) } else { mix(t.accent, t.nm_l, 0.08 * hover) };
            p.rect_filled(rect, cr(r), fill);
            (t.on_accent, t.on_accent.gamma_multiply(0.85))
        } else if !enabled {
            flat_disabled(p, rect, r, t);
            (t.text2, t.text2)
        } else if pressed {
            inset(p, rect, r, t.sunken, 4.0, t);
            (t.text, t.text2)
        } else {
            raised(p, rect, r, t.surface, 0.35 * hover, t);
            (t.text, t.text2)
        };
        let cy = rect.center().y;
        let mut x = if selected {
            let x = rect.left() + t.s(3);
            icons::paint_at(p, Icon::Check, pos2(x + check / 2.0, cy), check, fg);
            x + check + t.s(1)
        } else {
            rect.center().x - text_w / 2.0
        };
        let gms = gm.size();
        p.galley(pos2(x, cy - gms.y / 2.0), gm, fg);
        x += gms.x + t.s(1) + 2.0;
        if let Some(g) = gn {
            let y = cy - g.size().y / 2.0 + 1.0;
            p.galley(pos2(x, y), g, fg2);
        }
        if focus_visible(&resp) {
            focus_ring(p, rect, r, t);
        }
    }
    resp
}

/// A checkbox: an inset box with a 3:1 edge; on = accent fill and a check.
pub fn checkbox(ui: &mut Ui, value: &mut bool, label: &str) -> Response {
    let t = tk(ui);
    let g = galley(ui, label, FontId::proportional(t.fs));
    let bx = 18.0;
    let size = vec2(bx + if label.is_empty() { 0.0 } else { t.s(2) + g.size().x }, t.s(7));
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
        let r = t.r_sm * 0.5;
        if on {
            let fill = if enabled { t.accent } else { t.border_strong };
            p.rect_filled(b, cr(r), fill);
            icons::paint_at(p, Icon::Check, b.center(), 16.0, t.on_accent);
        } else {
            inset(p, b, r, t.sunken, 3.0, t);
            let edge = if !enabled {
                t.border
            } else if resp.hovered() {
                t.text
            } else {
                t.border_strong
            };
            p.rect_stroke(b, cr(r), Stroke::new(1.0, edge), StrokeKind::Inside);
        }
        let fg = if enabled { t.text } else { t.text2 };
        p.galley(pos2(b.right() + t.s(2), rect.center().y - g.size().y / 2.0), g, fg);
        if focus_visible(&resp) {
            focus_ring(p, b, r, t);
        }
    }
    resp
}

/// A segmented control in an inset track; the chosen option is an accent
/// pill with bold text.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> Response {
    let t = tk(ui);
    let pad = 3.0;
    let font = FontId::new(t.fs_sm, bold());
    let widths: Vec<f32> = options.iter().map(|(_, l)| galley(ui, l, font.clone()).size().x + t.s(3) * 2.0).collect();
    let size = vec2(widths.iter().sum::<f32>() + pad * 2.0, t.ctrl);
    let (rect, mut group) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter().clone();
    let r = size.y / 2.0;
    inset(&p, rect, r, t.sunken, 4.0, t);
    let mut x = rect.left() + pad;
    for ((opt, label), w) in options.iter().zip(widths) {
        let seg = Rect::from_min_size(pos2(x, rect.top() + pad), vec2(w, size.y - pad * 2.0));
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
            p.rect_filled(seg, cr(seg.height() / 2.0), t.accent);
            t.on_accent
        } else if resp.hovered() {
            t.text
        } else {
            t.text2
        };
        let g = galley(ui, label, font.clone());
        p.galley(seg.center() - g.size() / 2.0, g, fg);
        if focus_visible(&resp) {
            focus_ring(&p, seg, seg.height() / 2.0, t);
        }
    }
    group
}

/// A top-level tab. Off: flat. On: pressed into the surface, accent label,
/// weight 600. The width is the bold width either way, so nothing shifts.
pub fn tab(ui: &mut Ui, selected: bool, icon: Icon, text: &str) -> Response {
    let t = tk(ui);
    let gb = galley(ui, text, FontId::new(t.fs, bold()));
    let size = vec2(t.padx * 2.0 + t.icon_size + t.s(2) + gb.size().x, t.ctrl);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, text));
    if ui.is_rect_visible(rect) {
        let hover = anim(ui, resp.id.with("hover"), resp.hovered() && !selected);
        let p = ui.painter();
        let r = size.y / 2.0;
        let fg = if selected {
            inset(p, rect, r, t.sunken, 5.0, t);
            t.accent_text
        } else {
            if hover > 0.0 {
                p.extend(raised_shadows(rect, r, 0.0, hover, t));
                p.rect_filled(rect, cr(r), t.surface);
            }
            mix(t.text2, t.text, hover)
        };
        let g = if selected { gb } else { galley(ui, text, FontId::proportional(t.fs)) };
        let x = rect.left() + t.padx;
        icons::paint_at(p, icon, pos2(x + t.icon_size / 2.0, rect.center().y), t.icon_size, fg);
        p.galley(pos2(x + t.icon_size + t.s(2), rect.center().y - g.size().y / 2.0), g, fg);
        if focus_visible(&resp) {
            focus_ring(p, rect, r, t);
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

fn field_shapes(rect: Rect, r: f32, focused: bool, hovered: bool, t: &Tokens) -> Shape {
    let mut shapes = inset_shapes(rect, r, t.sunken, 5.0, t);
    let edge = if focused {
        Stroke::new(2.0, t.accent)
    } else if hovered {
        Stroke::new(1.0, t.text2)
    } else {
        Stroke::new(1.0, t.border_strong)
    };
    shapes.push(RectShape::stroke(rect, cr(r), edge, StrokeKind::Inside).into());
    Shape::Vec(shapes)
}

/// A one-line text field: an inset well with a 3:1 edge that turns into a
/// 2 px accent ring while typing. Configure `edit` as usual (hint, id…).
pub fn input(ui: &mut Ui, edit: egui::TextEdit<'_>, icon: Option<Icon>, width: f32) -> Response {
    let t = tk(ui);
    let (rect, outer) = ui.allocate_exact_size(vec2(width, t.ctrl), Sense::click());
    let idx = ui.painter().add(Shape::Noop);
    let left = t.s(3) + icon.map_or(0.0, |_| t.icon_size + t.s(2));
    let inner = Rect::from_min_max(pos2(rect.left() + left, rect.top()), pos2(rect.right() - t.s(3), rect.bottom()));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let resp = child.add(edit.frame(egui::Frame::NONE).desired_width(inner.width()).vertical_align(Align::Center));
    if outer.clicked() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    let hovered = resp.hovered() || outer.hovered();
    ui.painter().set(idx, field_shapes(rect, t.r_sm, focused, hovered, t));
    if let Some(i) = icon {
        let c = if focused { t.text } else { t.text2 };
        icons::paint_at(
            ui.painter(),
            i,
            pos2(rect.left() + t.s(3) + t.icon_size / 2.0, rect.center().y),
            t.icon_size,
            c,
        );
    }
    resp
}

/// A multi-line field of a fixed size, scrolling inside the well.
pub fn text_area(ui: &mut Ui, edit: egui::TextEdit<'_>, size: Vec2, id_salt: &str) -> Response {
    let t = tk(ui);
    let (rect, outer) = ui.allocate_exact_size(size, Sense::click());
    let idx = ui.painter().add(Shape::Noop);
    let inner = rect.shrink2(vec2(t.s(3), t.s(2)));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::top_down(Align::Min)));
    let resp = egui::ScrollArea::vertical()
        .id_salt(id_salt)
        .max_height(inner.height())
        .auto_shrink([false, false])
        .show(&mut child, |ui| {
            let rows = (inner.height() / ui.text_style_height(&egui::TextStyle::Monospace)).floor().max(1.0);
            ui.add(edit.frame(egui::Frame::NONE).desired_width(f32::INFINITY).desired_rows(rows as usize))
        })
        .inner;
    if outer.clicked() {
        resp.request_focus();
    }
    let focused = resp.has_focus();
    ui.painter().set(idx, field_shapes(rect, t.r_sm, focused, outer.hovered() || resp.hovered(), t));
    resp
}

/// egui's slider with a handle sized to the rail rather than the 36 px row.
pub fn slider(ui: &mut Ui, slider: egui::Slider<'_>) -> Response {
    let t = tk(ui);
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = t.s(7);
        ui.add(slider)
    })
    .inner
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

/// The frame every dialog uses: surface, large radius, modal shadow, 24 px in.
pub fn dialog_frame(ctx: &egui::Context) -> egui::Frame {
    let t = super::theme::tk_ctx(ctx);
    egui::Frame::new()
        .fill(t.surface)
        .stroke(Stroke::new(1.0, t.border))
        .shadow(t.shadow_modal)
        .corner_radius(cr(t.r_lg))
        .inner_margin(Margin::same(t.s(6) as i8))
}

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

/// A raised card (mid lift, large radius).
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    framed(
        ui,
        Margin::same(t.s(5) as i8),
        |rect| {
            let mut s = raised_shadows(rect, t.r_lg, 1.0, 1.0, t).to_vec();
            s.push(RectShape::filled(rect, cr(t.r_lg), t.surface).into());
            Shape::Vec(s)
        },
        add,
    )
}

/// A raised banner (low lift, medium radius) for notices above a tab.
pub fn banner<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    framed(
        ui,
        Margin::symmetric(t.s(4) as i8, t.s(2) as i8),
        |rect| {
            let mut s = raised_shadows(rect, t.r_md, 0.4, 1.0, t).to_vec();
            s.push(RectShape::filled(rect, cr(t.r_md), t.surface).into());
            Shape::Vec(s)
        },
        add,
    )
}

/// An inset well: lists and pickers sit in one.
pub fn well<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let t = tk(ui);
    framed(ui, Margin::same(t.s(2) as i8), |rect| Shape::Vec(inset_shapes(rect, t.r_md, t.sunken, 6.0, t)), add)
}

// ---------------------------------------------------------------------------
// Feedback
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Running: neutral, so "busy" never looks like "selected".
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
        Tone::Failed => t.danger,
        Tone::Paused => t.warning_text,
        Tone::Idle => t.border_strong,
    }
}

/// A progress bar in an inset track. `busy` animates for unknown progress
/// (a still segment with reduced motion).
pub fn progress(ui: &mut Ui, width: f32, frac: f32, tone: Tone, busy: bool) {
    let t = tk(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, t.s(2)), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = ui.painter();
    let r = rect.height() / 2.0;
    inset(p, rect, r, t.sunken, 3.0, t);
    let color = tone_color(t, tone);
    let track = rect.shrink(1.0);
    if busy {
        let w = track.width() * 0.3;
        let x = if super::theme::reduced_motion() {
            track.left()
        } else {
            ui.ctx().request_repaint();
            let time = ui.input(|i| i.time) as f32;
            track.left() + ((time * 0.6).fract() * (track.width() + w)) - w
        };
        let seg = Rect::from_min_max(
            pos2(x.max(track.left()), track.top()),
            pos2((x + w).min(track.right()), track.bottom()),
        );
        if seg.width() > 0.0 {
            p.rect_filled(seg, cr(r), color);
        }
    } else if frac > 0.0 {
        let fill = Rect::from_min_size(track.min, vec2(track.width() * frac.clamp(0.0, 1.0), track.height()));
        p.rect_filled(fill, cr(r), color);
    }
}

/// A small status dot. Always sits next to words that say the same thing.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// A thumbnail cropped to fill `size`, or an inset placeholder with an icon.
pub fn thumb(ui: &mut Ui, tex: Option<&TextureHandle>, size: Vec2, icon: Icon) -> Response {
    let t = tk(ui);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let r = t.r_sm;
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
            egui::Image::from_texture((tex.id(), size)).uv(uv).corner_radius(cr(r)).paint_at(ui, rect);
        }
        None => {
            let p = ui.painter();
            inset(p, rect, r, t.sunken, (size.y * 0.12).clamp(3.0, 6.0), t);
            let s = (size.y * 0.4).clamp(t.icon_size * 0.8, t.icon_size * 2.0);
            icons::paint_at(p, icon, rect.center(), s, t.text2);
        }
    }
    resp
}

// ---------------------------------------------------------------------------
// Type
// ---------------------------------------------------------------------------

/// Screen title: --fs-xl, 600, tracked -0.01 em.
pub fn title_text(ui: &Ui, text: &str) -> RichText {
    let t = tk(ui);
    RichText::new(text).family(bold()).size(t.fs_xl).color(t.text).extra_letter_spacing(t.fs_xl * t.heading_tracking)
}

/// Section heading: --fs-lg, 600, tracked -0.01 em.
pub fn heading_text(ui: &Ui, text: &str) -> RichText {
    let t = tk(ui);
    RichText::new(text).family(bold()).size(t.fs_lg).color(t.text).extra_letter_spacing(t.fs_lg * t.heading_tracking)
}

pub fn title(ui: &mut Ui, text: &str) -> Response {
    let rt = title_text(ui, text);
    ui.label(rt)
}

pub fn heading(ui: &mut Ui, text: &str) -> Response {
    let rt = heading_text(ui, text);
    ui.label(rt)
}

/// A heading with a leading 20 px icon.
pub fn icon_heading(ui: &mut Ui, icon: Icon, text: &str) {
    let t = tk(ui);
    ui.horizontal(|ui| {
        icons::show(ui, icon, t.icon_size, t.text2);
        heading(ui, text);
    });
}

/// A field label: --fs-sm, weight 600, --text2.
pub fn field_label(ui: &Ui, text: impl Into<String>) -> RichText {
    let t = tk(ui);
    RichText::new(text).family(bold()).size(t.fs_sm).color(t.text2)
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

pub fn colored(ui: &Ui, text: impl Into<String>, color: Color32) -> RichText {
    RichText::new(text).size(tk(ui).fs_sm).color(color)
}

/// A wrapped paragraph at --lh 1.5, capped near 75 characters a line.
pub fn para(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let t = tk(ui);
    let max = (t.fs_sm * 0.5 * 75.0).min(ui.available_width());
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

/// The gap between groups inside a section (--gap's smaller step, 8 px).
pub const SECTION_GAP: f32 = Tokens::light().s(2);

/// A section of the settings page: a raised card with an icon heading.
pub fn section(ui: &mut Ui, icon: Icon, title: &str, add: impl FnOnce(&mut Ui)) {
    let t = tk(ui);
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        icon_heading(ui, icon, title);
        ui.add_space(t.s(3));
        add(ui);
    });
    ui.add_space(t.s(6));
}

/// One labelled row in a settings section.
pub fn setting_row(ui: &mut Ui, label: &str, hint_text: &str, add: impl FnOnce(&mut Ui)) {
    let t = tk(ui);
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(200.0, t.ctrl), Sense::hover());
        let g = ui.painter().layout(label.to_owned(), FontId::new(t.fs, bold()), t.text, rect.width() - t.s(2));
        ui.painter().galley(pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, t.text);
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| add(ui));
            if !hint_text.is_empty() {
                hint(ui, hint_text);
            }
        });
    });
    ui.add_space(t.s(2));
}
