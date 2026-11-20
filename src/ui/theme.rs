//! Fonts, spacing and colours.
//!
//! Text uses Windows' own Segoe UI. Icons come from the Phosphor icon font
//! (MIT, via egui-phosphor). Fonts for Chinese, Japanese, Korean, Thai, Indic
//! scripts and emoji are loaded from Windows only when a title needs them.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, TextStyle, ThemePreference, vec2,
};
use eframe::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

use crate::settings::ThemePref;

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
    if let Some(data) = read_font(&["segoeui.ttf"]) {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(data)));
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "segoe".into());
    }
    // Icons go second so they never replace ordinary letters.
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    if let Some(data) = read_font(&["seguisym.ttf"]) {
        fonts.font_data.insert("symbols".into(), Arc::new(FontData::from_owned(data)));
        fonts.families.entry(FontFamily::Proportional).or_default().push("symbols".into());
    }
    if let Some(data) = read_font(&["consola.ttf"]) {
        fonts.font_data.insert("consolas".into(), Arc::new(FontData::from_owned(data)));
        fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "consolas".into());
    }
    // A semibold family for titles, falling back to the regular stack.
    let mut bold_stack = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    if let Some(data) = read_font(&["seguisb.ttf"]) {
        fonts.font_data.insert("segoe-semibold".into(), Arc::new(FontData::from_owned(data)));
        bold_stack.insert(0, "segoe-semibold".into());
    }
    fonts.families.insert(bold(), bold_stack);
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

pub fn apply_style(ctx: &egui::Context, pref: ThemePref) {
    ctx.set_theme(match pref {
        ThemePref::System => ThemePreference::System,
        ThemePref::Light => ThemePreference::Light,
        ThemePref::Dark => ThemePreference::Dark,
    });
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(10.0, 4.0);
        s.spacing.interact_size = vec2(36.0, 28.0);
        s.spacing.scroll.bar_width = 8.0;
        s.text_styles.insert(TextStyle::Body, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(14.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
        s.text_styles.insert(TextStyle::Heading, FontId::new(20.0, bold()));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(12.5));
        let r = CornerRadius::same(6);
        s.visuals.widgets.noninteractive.corner_radius = r;
        s.visuals.widgets.inactive.corner_radius = r;
        s.visuals.widgets.hovered.corner_radius = r;
        s.visuals.widgets.active.corner_radius = r;
        s.visuals.widgets.open.corner_radius = r;
        s.visuals.window_corner_radius = CornerRadius::same(10);
        s.visuals.menu_corner_radius = CornerRadius::same(8);
        s.visuals.selection.bg_fill =
            if s.visuals.dark_mode { Color32::from_rgb(38, 99, 196) } else { Color32::from_rgb(59, 130, 246) };
    });
}

/// Colours that work on both light and dark backgrounds.
#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub accent: Color32,
    pub green: Color32,
    pub red: Color32,
    pub amber: Color32,
    pub track: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub border: Color32,
    pub muted: Color32,
    pub text: Color32,
}

impl Pal {
    pub fn of(ui: &egui::Ui) -> Pal {
        let v = ui.visuals();
        if v.dark_mode {
            Pal {
                dark: true,
                accent: Color32::from_rgb(88, 160, 255),
                green: Color32::from_rgb(74, 201, 126),
                red: Color32::from_rgb(255, 112, 104),
                amber: Color32::from_rgb(246, 180, 72),
                track: Color32::from_gray(52),
                card: Color32::from_gray(32),
                card_hover: Color32::from_gray(38),
                border: Color32::from_gray(48),
                muted: Color32::from_gray(150),
                text: v.text_color(),
            }
        } else {
            Pal {
                dark: false,
                accent: Color32::from_rgb(37, 99, 235),
                green: Color32::from_rgb(22, 150, 76),
                red: Color32::from_rgb(200, 40, 34),
                amber: Color32::from_rgb(196, 122, 0),
                track: Color32::from_gray(222),
                card: Color32::from_gray(252),
                card_hover: Color32::from_gray(246),
                border: Color32::from_gray(222),
                muted: Color32::from_gray(105),
                text: v.text_color(),
            }
        }
    }
}
