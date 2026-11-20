//! Design tokens, fonts and the base egui style.
//!
//! Snagger's look is a command console: one pale line colour on near-black,
//! hairline modules, clipped-corner frames and keys that light up. Every
//! colour, size and duration the UI uses comes from [`Tokens`]; nothing else
//! hard-codes them. The dark set mirrors the design brief one-to-one. Text is
//! Saira, values are Fira Code (both SIL OFL, bundled in `assets/fonts`);
//! icons are Phosphor Thin (MIT, see `icons.rs`).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke, TextStyle,
    ThemePreference, Ui, vec2,
};
use eframe::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

use crate::settings::ThemePref;

/// The design tokens, by the names the brief uses.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    // type
    pub fs_xs: f32,
    pub fs_sm: f32,
    pub fs: f32,
    pub fs_lg: f32,
    pub fs_xl: f32,
    /// In the set for completeness; no text in Snagger is that large.
    #[allow(dead_code)]
    pub fs_2xl: f32,
    pub lh: f32,
    /// Headings are uppercase, so they're drawn a little smaller.
    pub heading_scale: f32,
    /// Extra space between heading letters, in em.
    pub heading_tracking: f32,
    // colour
    pub bg: Color32,
    pub surface: Color32,
    pub sunken: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub bw: f32,
    pub text: Color32,
    pub text2: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub pri_bg: Color32,
    pub pri_fg: Color32,
    pub pri_border: Color32,
    pub danger: Color32,
    pub danger_text: Color32,
    pub success_text: Color32,
    pub warning_text: Color32,
    // space (4 px base)
    pub ctrl: f32,
    pub padx: f32,
    pub gap: f32,
    pub row: f32,
    // shape and depth
    pub r_sm: f32,
    pub r_md: f32,
    pub r_lg: f32,
    pub r_btn: f32,
    pub shadow_overlay: Shadow,
    pub shadow_modal: Shadow,
    // Command console extras
    pub dk_line: Color32,
    pub dk_dim: Color32,
    pub dk_fill: Color32,
    pub dk_fill2: Color32,
    // icons
    pub icon_size: f32,
    // motion, in seconds
    pub dur_fast: f32,
    pub dur_base: f32,
    /// In the set for completeness; Snagger has no slow transitions.
    #[allow(dead_code)]
    pub dur_slow: f32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Tokens {
    /// The brief's tokens (the console is a dark style).
    pub const fn dark() -> Tokens {
        Tokens {
            fs_xs: 12.0,
            fs_sm: 12.5,
            fs: 13.0,
            fs_lg: 15.0,
            fs_xl: 19.0,
            fs_2xl: 24.0,
            lh: 1.4,
            heading_scale: 0.92,
            heading_tracking: 0.12,
            bg: hex(0x06090E),
            surface: hex(0x090E16),
            sunken: hex(0x0D1521),
            border: hex(0x2A3539),
            border_strong: hex(0x657C7F),
            bw: 1.0,
            text: hex(0xAACFD1),
            text2: hex(0x799497),
            accent: hex(0xAACFD1),
            accent_text: hex(0xAACFD1),
            on_accent: hex(0x111111),
            accent_soft: hex(0x29353B),
            pri_bg: hex(0xAACFD1),
            pri_fg: hex(0x111111),
            pri_border: hex(0xAACFD1),
            danger: hex(0xD92D20),
            danger_text: hex(0xFF8A80),
            success_text: hex(0x5FD08A),
            warning_text: hex(0xF0B24B),
            ctrl: 28.0,
            padx: 10.0,
            gap: 8.0,
            row: 36.0,
            r_sm: 2.0,
            r_md: 3.0,
            r_lg: 4.0,
            r_btn: 3.0,
            shadow_overlay: Shadow::NONE,
            shadow_modal: Shadow::NONE,
            dk_line: hex(0x415054),
            dk_dim: hex(0x222B2F),
            dk_fill: hex(0x11171C),
            dk_fill2: hex(0x222B2F),
            icon_size: 16.0,
            dur_fast: 0.09,
            dur_base: 0.14,
            dur_slow: 0.22,
        }
    }

    /// Derived for Snagger's Light setting with the same rules: one ink colour
    /// (the accent) on pale neutrals at hue 216. Not part of the brief.
    pub const fn light() -> Tokens {
        let d = Tokens::dark();
        Tokens {
            bg: hex(0xEEF1F4),
            surface: hex(0xF5F7F9),
            sunken: hex(0xE4E9EE),
            border: hex(0xC5CED6),
            border_strong: hex(0x6A7F86),
            text: hex(0x163D42),
            text2: hex(0x4B6266),
            accent: hex(0x163D42),
            accent_text: hex(0x163D42),
            on_accent: hex(0xF2F7F7),
            accent_soft: hex(0xCFDDE0),
            pri_bg: hex(0x163D42),
            pri_fg: hex(0xF2F7F7),
            pri_border: hex(0x163D42),
            danger: hex(0xB42318),
            danger_text: hex(0xA4201A),
            success_text: hex(0x146A3A),
            warning_text: hex(0x7E4C00),
            dk_line: hex(0xA9B8BE),
            dk_dim: hex(0xD5DEE3),
            dk_fill: hex(0xE6EBEF),
            dk_fill2: hex(0xD5DEE3),
            ..d
        }
    }

    /// 4 px spacing steps: s(1)=4, s(2)=8, s(3)=12, s(4)=16, s(6)=24, s(8)=32.
    pub const fn s(&self, steps: u8) -> f32 {
        4.0 * steps as f32
    }

    /// Heading letter spacing, in points, for a heading drawn at `size`.
    pub fn tracking(&self, size: f32) -> f32 {
        size * self.heading_tracking
    }
}

static LIGHT: Tokens = Tokens::light();
static DARK: Tokens = Tokens::dark();

/// The tokens for whichever theme is showing.
pub fn tk(ui: &Ui) -> &'static Tokens {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

pub fn tk_ctx(ctx: &egui::Context) -> &'static Tokens {
    if ctx.global_style().visuals.dark_mode { &DARK } else { &LIGHT }
}

/// Labels, keys and strong text: Saira 600 (--fw-label).
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

/// Headings: Saira 500 (--fw-head).
pub fn head() -> FontFamily {
    FontFamily::Name("head".into())
}

fn fonts_dir() -> PathBuf {
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    windir.join("Fonts")
}

fn read_font(names: &[&str]) -> Option<Vec<u8>> {
    let dir = fonts_dir();
    names.iter().find_map(|n| std::fs::read(dir.join(n)).ok())
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, data) in [
        ("saira", &include_bytes!("../../assets/fonts/Saira-Regular.ttf")[..]),
        ("saira-medium", &include_bytes!("../../assets/fonts/Saira-Medium.ttf")[..]),
        ("saira-semibold", &include_bytes!("../../assets/fonts/Saira-SemiBold.ttf")[..]),
        ("fira-code", &include_bytes!("../../assets/fonts/FiraCode-Regular.ttf")[..]),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(data)));
    }
    // Saira first; egui's emoji fonts and Windows' symbol font only fill in
    // glyphs Saira doesn't have (arrows, ≈, rare symbols).
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.contains("moji") || f.contains("Noto"))
        .collect();
    if let Some(data) = read_font(&["seguisym.ttf"]) {
        fonts.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(data)));
    }
    let family = |first: &str| {
        let mut v = vec![first.to_string()];
        v.extend(fallbacks.iter().cloned());
        v.push("symbols".into());
        v.retain(|f| fonts.font_data.contains_key(f));
        v
    };
    let (regular, medium, semibold, mono) =
        (family("saira"), family("saira-medium"), family("saira-semibold"), family("fira-code"));
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(head(), medium);
    fonts.families.insert(bold(), semibold);
    fonts.families.insert(FontFamily::Monospace, mono);
    ctx.set_fonts(fonts);
}

/// Loads script fonts the first time text needs them.
#[derive(Default)]
pub struct ScriptFonts {
    loaded: u32,
}

const CJK: u32 = 1;
const HANGUL: u32 = 2;
const EMOJI: u32 = 4;
const THAI: u32 = 8;
const INDIC: u32 = 16;

fn script_of(c: char) -> u32 {
    match c as u32 {
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF | 0x3000..=0x303F => {
            CJK
        }
        0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => HANGUL,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF => EMOJI,
        0x0E00..=0x0E7F => THAI,
        0x0900..=0x0DFF => INDIC,
        _ => 0,
    }
}

impl ScriptFonts {
    pub fn check<'a>(&mut self, ctx: &egui::Context, texts: impl Iterator<Item = &'a str>) {
        let mut need = 0;
        for t in texts {
            if t.is_ascii() {
                continue;
            }
            for c in t.chars() {
                need |= script_of(c);
            }
        }
        let missing = need & !self.loaded;
        if missing == 0 {
            return;
        }
        for (flag, name, files) in [
            (CJK, "cjk", &["msyh.ttc", "YuGothM.ttc", "msgothic.ttc"][..]),
            (HANGUL, "hangul", &["malgun.ttf"][..]),
            (EMOJI, "emoji", &["seguiemj.ttf"][..]),
            (THAI, "thai", &["LeelawUI.ttf", "leelawad.ttf"][..]),
            (INDIC, "indic", &["Nirmala.ttc", "Nirmala.ttf"][..]),
        ] {
            if missing & flag == 0 {
                continue;
            }
            self.loaded |= flag;
            if let Some(data) = read_font(files) {
                ctx.add_font(FontInsert {
                    name: name.into(),
                    data: FontData::from_owned(data),
                    families: [FontFamily::Proportional, bold(), head(), FontFamily::Monospace]
                        .into_iter()
                        .map(|family| InsertFontFamily { family, priority: FontPriority::Lowest })
                        .collect(),
                });
            }
        }
    }
}

static REDUCED: AtomicBool = AtomicBool::new(false);

/// Windows "Animation effects" is off: no hover blends, no moving bars.
pub fn reduced_motion() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Maps the tokens onto egui's own widgets (combo boxes, sliders, menus,
/// windows, tooltips, scroll bars), so built-ins match Snagger's components.
pub fn apply_style(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_theme(match pref {
        ThemePref::System => ThemePreference::System,
        ThemePref::Light => ThemePreference::Light,
        ThemePref::Dark => ThemePreference::Dark,
    });
    let reduced = crate::platform::reduced_motion();
    REDUCED.store(reduced, Ordering::Relaxed);
    for (theme, t) in [(egui::Theme::Light, &LIGHT), (egui::Theme::Dark, &DARK)] {
        ctx.style_mut_of(theme, |s| {
            s.animation_time = if reduced { 0.0 } else { t.dur_base };
            s.spacing.item_spacing = vec2(t.gap, t.s(1) + 2.0);
            s.spacing.button_padding = vec2(t.padx, t.s(1));
            s.spacing.interact_size = vec2(t.ctrl, t.ctrl);
            s.spacing.combo_height = 280.0;
            s.spacing.menu_margin = egui::Margin::same(t.s(1) as i8);
            s.spacing.menu_width = 220.0;
            s.spacing.window_margin = egui::Margin::same(t.s(4) as i8);
            s.spacing.slider_width = 160.0;
            s.spacing.scroll.bar_width = 6.0;
            s.spacing.scroll.floating = true;
            // The handle is drawn in the line colour and never fully hides,
            // so the list always shows where it is.
            s.spacing.scroll.foreground_color = true;
            s.spacing.scroll.dormant_handle_opacity = 0.35;
            s.text_styles.insert(TextStyle::Small, FontId::proportional(t.fs_xs));
            s.text_styles.insert(TextStyle::Body, FontId::proportional(t.fs));
            s.text_styles.insert(TextStyle::Button, FontId::new(t.fs_sm, bold()));
            s.text_styles.insert(TextStyle::Heading, FontId::new(t.fs_lg * t.heading_scale, head()));
            s.text_styles.insert(TextStyle::Monospace, FontId::monospace(t.fs_sm));
            s.interaction.selectable_labels = false;

            let v = &mut s.visuals;
            v.dark_mode = theme == egui::Theme::Dark;
            v.override_text_color = None;
            v.panel_fill = t.bg;
            v.window_fill = t.surface;
            v.faint_bg_color = t.sunken;
            v.extreme_bg_color = t.sunken;
            v.code_bg_color = t.sunken;
            v.hyperlink_color = t.accent_text;
            v.warn_fg_color = t.warning_text;
            v.error_fg_color = t.danger_text;
            // No shadows and no glow; square-ish corners.
            v.window_corner_radius = CornerRadius::same(t.r_lg as u8);
            v.menu_corner_radius = CornerRadius::same(t.r_sm as u8);
            v.window_shadow = t.shadow_modal;
            v.popup_shadow = t.shadow_overlay;
            v.window_stroke = Stroke::new(t.bw, t.border_strong);
            v.selection.bg_fill = t.accent_soft;
            v.selection.stroke = Stroke::new(t.bw, t.accent_text);
            v.text_cursor.stroke = Stroke::new(2.0, t.accent);
            v.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.5 };
            v.slider_trailing_fill = false;
            let r = CornerRadius::same(t.r_sm as u8);
            let w = &mut v.widgets;
            w.noninteractive.bg_fill = t.surface;
            w.noninteractive.weak_bg_fill = t.surface;
            w.noninteractive.bg_stroke = Stroke::new(t.bw, t.border);
            w.noninteractive.fg_stroke = Stroke::new(t.bw, t.text);
            w.noninteractive.corner_radius = r;
            // Combo boxes, sliders and drag values read as inputs: a 3:1 edge.
            w.inactive.bg_fill = t.dk_dim;
            w.inactive.weak_bg_fill = t.sunken;
            w.inactive.bg_stroke = Stroke::new(t.bw, t.border_strong);
            w.inactive.fg_stroke = Stroke::new(t.bw, t.text);
            w.inactive.corner_radius = r;
            w.inactive.expansion = 0.0;
            w.hovered.bg_fill = t.text2;
            w.hovered.weak_bg_fill = t.dk_fill;
            w.hovered.bg_stroke = Stroke::new(t.bw, t.text);
            w.hovered.fg_stroke = Stroke::new(t.bw, t.text);
            w.hovered.corner_radius = r;
            w.hovered.expansion = 0.0;
            w.active.bg_fill = t.accent;
            w.active.weak_bg_fill = t.dk_fill2;
            w.active.bg_stroke = Stroke::new(t.bw, t.accent);
            w.active.fg_stroke = Stroke::new(t.bw, t.text);
            w.active.corner_radius = r;
            w.active.expansion = 0.0;
            w.open.bg_fill = t.dk_fill2;
            w.open.weak_bg_fill = t.dk_fill;
            w.open.bg_stroke = Stroke::new(t.bw, t.accent);
            w.open.fg_stroke = Stroke::new(t.bw, t.text);
            w.open.corner_radius = r;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: Color32) -> f32 {
        let f = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f32 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Pale lines fade fast: text stays at 4.5:1 and interface edges at 3:1
    /// on every surface they sit on, in both themes.
    #[test]
    fn tokens_keep_their_contrast() {
        for (name, t) in [("dark", Tokens::dark()), ("light", Tokens::light())] {
            for (surface, bg) in [("bg", t.bg), ("surface", t.surface), ("sunken", t.sunken), ("dk_fill", t.dk_fill)] {
                for (role, fg, min) in [
                    ("text", t.text, 4.5),
                    ("text2", t.text2, 4.5),
                    ("danger_text", t.danger_text, 4.5),
                    ("success_text", t.success_text, 4.5),
                    ("warning_text", t.warning_text, 4.5),
                    ("border_strong", t.border_strong, 3.0),
                ] {
                    let c = contrast(fg, bg);
                    assert!(c >= min, "{name}: {role} on {surface} is {c:.2}:1");
                }
            }
            // Chosen rows switch their secondary text to --text (see widgets::sub_color).
            assert!(contrast(t.text, t.accent_soft) >= 4.5, "{name}: text on accent_soft");
            assert!(contrast(t.pri_fg, t.pri_bg) >= 4.5, "{name}: primary key label");
            assert!(contrast(t.on_accent, t.accent) >= 4.5, "{name}: chosen key label");
            assert!(contrast(t.on_accent, t.danger_text) >= 4.5, "{name}: pressed danger key label");
        }
    }
}
