//! Iconoir icons (v7.12.2, MIT; licence in `assets/iconoir/LICENSE`).
//!
//! The SVGs are bundled unchanged. Each one is rasterised once per pixel size
//! with resvg, in white, and tinted when painted, so one texture serves every
//! colour.

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
                    $(Icon::$name => include_bytes!(concat!("../../assets/iconoir/", $file, ".svg")),)*
                }
            }
        }

        #[cfg(test)]
        const ALL: &[Icon] = &[$(Icon::$name,)*];
    };
}

icons! {
    Download => "download",
    History => "clock-rotate-right",
    Feed => "rss-feed",
    Settings => "settings",
    Link => "link",
    Paste => "paste-clipboard",
    AddMany => "playlist-plus",
    Import => "import",
    Warning => "warning-triangle",
    Alert => "warning-circle",
    Close => "xmark",
    Check => "check",
    Done => "check-circle",
    Search => "search",
    Eye => "eye",
    Back => "arrow-left",
    Play => "play",
    Pause => "pause",
    Open => "open-in-window",
    Globe => "internet",
    Folder => "folder",
    Refresh => "refresh",
    Retry => "refresh-double",
    Terminal => "terminal",
    Trash => "trash",
    Erase => "erase",
    Up => "arrow-up",
    Down => "arrow-down",
    Copy => "copy",
    More => "more-vert",
    Plus => "plus",
    Minus => "minus",
    Video => "media-video",
    Music => "music-double-note",
    Note => "music-note",
    Playlist => "playlist",
    Page => "page",
    Archive => "archive",
    Image => "media-image",
    Table => "table",
    Slides => "presentation",
    Package => "package",
    Cube => "cube",
    Power => "system-shut",
    Clock => "clock",
    Window => "web-window",
    Scissor => "scissor",
    Captions => "closed-captions-tag",
    Cookie => "cookie",
    Cpu => "cpu",
    Info => "info-circle",
    Hourglass => "hourglass",
}

type Cache = Mutex<HashMap<(Icon, u32), TextureHandle>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// White, premultiplied RGBA at `px` × `px`.
fn rasterise(icon: Icon, px: u32) -> ColorImage {
    let svg = String::from_utf8_lossy(icon.svg()).replace("currentColor", "#FFFFFF");
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("icon size");
    if let Ok(tree) = usvg::Tree::from_data(svg.as_bytes(), &usvg::Options::default()) {
        let s = px as f32 / tree.size().width();
        resvg::render(&tree, tiny_skia::Transform::from_scale(s, s), &mut pixmap.as_mut());
    }
    ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data())
}

fn texture(ctx: &egui::Context, icon: Icon, px: u32) -> TextureHandle {
    let mut map = cache().lock().unwrap();
    map.entry((icon, px))
        .or_insert_with(|| {
            ctx.load_texture(format!("iconoir-{icon:?}-{px}"), rasterise(icon, px), TextureOptions::LINEAR)
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
