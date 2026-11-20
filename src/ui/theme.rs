//! Design tokens, fonts and the base egui style.
//!
//! Snag's look is Neumorphism ("soft UI"): page and panels share one colour
//! and shape comes from paired light and dark shadows. Every colour, size and
//! duration the UI uses comes from [`Tokens`]; nothing else hard-codes them.
//! The light set mirrors the design brief one-to-one. Text is DM Sans (SIL OFL,
//! bundled in `assets/fonts`); icons are Iconoir (MIT, see `icons.rs`).

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
    /// In the set for completeness; no text in Snag is that large.
    #[allow(dead_code)]
    pub fs_2xl: f32,
    pub lh: f32,
    pub heading_tracking: f32,
    // colour
    pub bg: Color32,
    pub surface: Color32,
    pub sunken: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub on_accent: Color32,
    pub accent_soft: Color32,
    pub pri_bg: Color32,
    pub pri_fg: Color32,
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
    pub nm_d: Color32,
    pub nm_l: Color32,
    // icons
    pub icon_size: f32,
    // motion, in seconds
    pub dur_fast: f32,
    pub dur_base: f32,
    /// In the set for completeness; Snag has no slow transitions.
    #[allow(dead_code)]
    pub dur_slow: f32,
}

const fn hex(rgb: u32) -> Color32 {
    Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

impl Tokens {
    /// The brief's light tokens.
    pub const fn light() -> Tokens {
        Tokens {
            fs_xs: 12.0,
            fs_sm: 13.0,
            fs: 14.0,
            fs_lg: 17.0,
            fs_xl: 22.0,
            fs_2xl: 28.0,
            lh: 1.5,
            heading_tracking: -0.01,
            bg: hex(0xE5E7EB),
            surface: hex(0xE5E7EB),
            sunken: hex(0xD9DCE2),
            border: hex(0xBCC2CD),
            border_strong: hex(0x737F96),
            text: hex(0x181B20),
            text2: hex(0x4F5869),
            accent: hex(0x1F4FBF),
            accent_text: hex(0x1F4FBF),
            on_accent: hex(0xFFFFFF),
            accent_soft: hex(0xCDD5E6),
            pri_bg: hex(0x1F4FBF),
            pri_fg: hex(0xFFFFFF),
            danger: hex(0xB42318),
            danger_text: hex(0xAF2217),
            success_text: hex(0x16693B),
            warning_text: hex(0x865100),
            ctrl: 36.0,
            padx: 14.0,
            gap: 12.0,
            row: 48.0,
            r_sm: 10.0,
            r_md: 14.0,
            r_lg: 22.0,
            r_btn: 999.0,
            shadow_overlay: Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(20) },
            shadow_modal: Shadow { offset: [0, 16], blur: 40, spread: 0, color: Color32::from_black_alpha(46) },
            nm_d: hex(0xBEC0C3),
            nm_l: hex(0xFFFFFF),
            icon_size: 20.0,
            dur_fast: 0.16,
            dur_base: 0.26,
            dur_slow: 0.42,
        }
    }

    /// Derived for Snag's dark setting with the same rules (hue 220, paired
    /// shadows). Not part of the brief.
    pub const fn dark() -> Tokens {
        let l = Tokens::light();
        Tokens {
            bg: hex(0x2A2F37),
            surface: hex(0x2A2F37),
            sunken: hex(0x22262D),
            border: hex(0x3E4551),
            border_strong: hex(0x7D879A),
            text: hex(0xE7EAEF),
            text2: hex(0xA9B1BF),
            accent: hex(0x3D6BE0),
            accent_text: hex(0x8FB0FF),
            on_accent: hex(0xFFFFFF),
            accent_soft: hex(0x34405A),
            pri_bg: hex(0x3D6BE0),
            pri_fg: hex(0xFFFFFF),
            danger: hex(0xE5534B),
            danger_text: hex(0xFF8F87),
            success_text: hex(0x6FD49A),
            warning_text: hex(0xF0B65A),
            shadow_overlay: Shadow { offset: [0, 4], blur: 12, spread: 0, color: Color32::from_black_alpha(90) },
            shadow_modal: Shadow { offset: [0, 16], blur: 40, spread: 0, color: Color32::from_black_alpha(140) },
            nm_d: hex(0x1D2026),
            nm_l: hex(0x373D47),
            ..l
        }
    }

    /// 4 px spacing steps: s(1)=4, s(2)=8, s(3)=12, s(4)=16, s(6)=24, s(8)=32.
    pub const fn s(&self, steps: u8) -> f32 {
        4.0 * steps as f32
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

/// Headings, labels and buttons: DM Sans 600.
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
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
    fonts.font_data.insert(
        "dm-sans".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/DMSans-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "dm-sans-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/DMSans-SemiBold.ttf"))),
    );
    // DM Sans first; egui's emoji fonts and Windows' symbol font only fill in
    // glyphs DM Sans doesn't have (arrows, ≈, rare symbols).
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
    let mut regular = vec!["dm-sans".to_string()];
    regular.extend(fallbacks.iter().cloned());
    regular.push("symbols".into());
    let mut semibold = vec!["dm-sans-semibold".to_string()];
    semibold.extend(fallbacks);
    semibold.push("symbols".into());
    regular.retain(|f| fonts.font_data.contains_key(f));
    semibold.retain(|f| fonts.font_data.contains_key(f));
    fonts.families.insert(FontFamily::Proportional, regular);
    fonts.families.insert(bold(), semibold);
    // The log and the template field show code, so they stay monospaced.
    if let Some(data) = read_font(&["consola.ttf"]) {
        fonts.font_data.insert("consolas".into(), Arc::new(FontData::from_owned(data)));
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "consolas".into());
    }
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
                    families: vec![
                        InsertFontFamily { family: FontFamily::Proportional, priority: FontPriority::Lowest },
                        InsertFontFamily { family: bold(), priority: FontPriority::Lowest },
                    ],
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
/// dialogs, tooltips, scroll bars), so built-ins match Snag's components.
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
            s.spacing.item_spacing = vec2(t.s(2), t.s(2));
            s.spacing.button_padding = vec2(t.padx, t.s(1) + 2.0);
            s.spacing.interact_size = vec2(t.ctrl, t.ctrl);
            s.spacing.combo_height = 280.0;
            s.spacing.menu_margin = egui::Margin::same(t.s(2) as i8);
            s.spacing.menu_width = 240.0;
            s.spacing.window_margin = egui::Margin::same(t.s(6) as i8);
            s.spacing.slider_width = 160.0;
            s.spacing.scroll.bar_width = 8.0;
            s.spacing.scroll.floating = true;
            s.text_styles.insert(TextStyle::Small, FontId::proportional(t.fs_xs));
            s.text_styles.insert(TextStyle::Body, FontId::proportional(t.fs));
            s.text_styles.insert(TextStyle::Button, FontId::new(t.fs, bold()));
            s.text_styles.insert(TextStyle::Heading, FontId::new(t.fs_xl, bold()));
            s.text_styles.insert(TextStyle::Monospace, FontId::monospace(t.fs_sm));
            s.interaction.selectable_labels = false;

            let v = &mut s.visuals;
            v.panel_fill = t.bg;
            v.window_fill = t.surface;
            v.faint_bg_color = t.sunken;
            v.extreme_bg_color = t.sunken;
            v.code_bg_color = t.sunken;
            v.hyperlink_color = t.accent_text;
            v.warn_fg_color = t.warning_text;
            v.error_fg_color = t.danger_text;
            v.window_corner_radius = CornerRadius::same(t.r_lg as u8);
            v.menu_corner_radius = CornerRadius::same(t.r_md as u8);
            v.window_shadow = t.shadow_modal;
            v.popup_shadow = t.shadow_overlay;
            v.window_stroke = Stroke::new(1.0, t.border);
            v.selection.bg_fill = t.accent_soft;
            v.selection.stroke = Stroke::new(1.0, t.accent_text);
            v.text_cursor.stroke = Stroke::new(2.0, t.accent);
            let r = CornerRadius::same(t.r_sm as u8);
            let w = &mut v.widgets;
            w.noninteractive.bg_fill = t.surface;
            w.noninteractive.weak_bg_fill = t.surface;
            w.noninteractive.bg_stroke = Stroke::new(1.0, t.border);
            w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
            w.noninteractive.corner_radius = r;
            // Combo boxes and drag values read as inputs: a 3:1 edge.
            w.inactive.bg_fill = t.sunken;
            w.inactive.weak_bg_fill = t.sunken;
            w.inactive.bg_stroke = Stroke::new(1.0, t.border_strong);
            w.inactive.fg_stroke = Stroke::new(1.0, t.text);
            w.inactive.corner_radius = r;
            w.inactive.expansion = 0.0;
            w.hovered.bg_fill = t.sunken;
            w.hovered.weak_bg_fill = t.sunken;
            w.hovered.bg_stroke = Stroke::new(1.0, t.text2);
            w.hovered.fg_stroke = Stroke::new(1.5, t.text);
            w.hovered.corner_radius = r;
            w.hovered.expansion = 0.0;
            w.active.bg_fill = t.sunken;
            w.active.weak_bg_fill = t.sunken;
            w.active.bg_stroke = Stroke::new(2.0, t.accent);
            w.active.fg_stroke = Stroke::new(1.5, t.text);
            w.active.corner_radius = r;
            w.active.expansion = 0.0;
            w.open.bg_fill = t.sunken;
            w.open.weak_bg_fill = t.sunken;
            w.open.bg_stroke = Stroke::new(2.0, t.accent);
            w.open.fg_stroke = Stroke::new(1.5, t.text);
            w.open.corner_radius = r;
        });
    }
}
