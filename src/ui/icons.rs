//! Phosphor icons, Thin weight (@phosphor-icons/core 2.1.1, MIT; licence in
//! `assets/phosphor/LICENSE`).
//!
//! The SVGs are bundled unchanged. Each one is rasterised once per pixel size
//! with resvg, in white, and tinted when painted, so one texture serves every
//! colour. Thin strokes are half a pixel wide at 16 px, so coverage gets a
//! gain ([`INK_GAIN`]) that keeps the lines at full colour without changing
//! their shape: the same idea as font gamma.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use eframe::egui::{
    self, Color32, ColorImage, Painter, Rect, Response, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2,
};
use resvg::{tiny_skia, usvg};

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Icon { $($name,)* }

        impl Icon {
            fn svg(self) -> &'static [u8] {
                match self {
                    $(Icon::$name => include_bytes!(concat!("../../assets/phosphor/", $file, "-thin.svg")),)*
                }
            }
        }

        #[cfg(test)]
        const ALL: &[Icon] = &[$(Icon::$name,)*];
    };
}

icons! {
    Download => "download-simple",
    History => "clock-counter-clockwise",
    Feed => "rss",
    Settings => "gear",
    Link => "link",
    Paste => "clipboard-text",
    AddMany => "list-plus",
    Import => "tray-arrow-down",
    Warning => "warning",
    Alert => "warning-circle",
    Close => "x",
    Check => "check",
    Done => "check-circle",
    Search => "magnifying-glass",
    Eye => "eye",
    Back => "arrow-left",
    Play => "play",
    Pause => "pause",
    Open => "arrow-square-out",
    Globe => "globe",
    Folder => "folder",
    Refresh => "arrow-clockwise",
    Retry => "arrows-clockwise",
    Terminal => "terminal-window",
    Trash => "trash",
    Erase => "eraser",
    Up => "arrow-up",
    Down => "arrow-down",
    Copy => "copy",
    More => "dots-three-vertical",
    Plus => "plus",
    Minus => "minus",
    Video => "film-strip",
    Music => "music-notes",
    Note => "music-note",
    Playlist => "playlist",
    Page => "file-text",
    Archive => "file-zip",
    Image => "image",
    Table => "table",
    Slides => "presentation",
    Package => "package",
    Cube => "cube",
    Power => "power",
    Clock => "clock",
    Window => "app-window",
    Scissor => "scissors",
    Captions => "closed-captioning",
    Cookie => "cookie",
    Cpu => "cpu",
    Info => "info",
    Hourglass => "hourglass",
}

type Cache = Mutex<HashMap<(Icon, u32), TextureHandle>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// How much a thin stroke's edge coverage is boosted (1.0 = none).
const INK_GAIN: f32 = 1.6;

/// White, premultiplied RGBA at `px` × `px`.
fn rasterise(icon: Icon, px: u32) -> ColorImage {
    let svg = String::from_utf8_lossy(icon.svg()).replace("currentColor", "#FFFFFF");
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("icon size");
    if let Ok(tree) = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()) {
        let s = px as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
    }
    // White premultiplied: every channel equals the coverage.
    let gain = if px <= 24 { INK_GAIN } else { 1.0 + (INK_GAIN - 1.0) * 24.0 / px as f32 };
    let data: Vec<u8> = pixmap.data().iter().map(|&c| (c as f32 * gain).min(255.0) as u8).collect();
    ColorImage::from_rgba_premultiplied([px as usize, px as usize], &data)
}

fn texture(ctx: &egui::Context, icon: Icon, px: u32) -> TextureHandle {
    let mut map = cache().lock().unwrap();
    map.entry((icon, px))
        .or_insert_with(|| {
            ctx.load_texture(format!("phosphor-{icon:?}-{px}"), rasterise(icon, px), TextureOptions::LINEAR)
        })
        .clone()
}

/// Paints `icon` filling `rect` (square), tinted `color`, snapped to pixels.
pub fn paint(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    let ctx = painter.ctx();
    let ppp = ctx.pixels_per_point();
    let px = (rect.width() * ppp).round().max(1.0) as u32;
    let tex = texture(ctx, icon, px);
    let min = pos2((rect.min.x * ppp).round() / ppp, (rect.min.y * ppp).round() / ppp);
    let r = Rect::from_min_size(min, vec2(px as f32 / ppp, px as f32 / ppp));
    painter.image(tex.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
}

/// Paints `icon` at `size`, centred on `center`.
pub fn paint_at(painter: &Painter, icon: Icon, center: egui::Pos2, size: f32, color: Color32) {
    paint(painter, icon, Rect::from_center_size(center, vec2(size, size)), color);
}

/// An icon laid out like a label.
pub fn show(ui: &mut Ui, icon: Icon, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    paint(ui.painter(), icon, rect, color);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_parses_and_draws() {
        for &icon in ALL {
            let img = rasterise(icon, 20);
            let ink = img.pixels.iter().filter(|p| p.a() > 0).count();
            assert!(ink > 10, "{icon:?} drew nothing");
        }
    }
}
