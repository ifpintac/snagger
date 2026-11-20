//! Embeds the app icon (Microsoft Fluent Emoji "Inbox tray", MIT) and version
//! details into Snagger.exe.

use std::fs::File;
use std::path::PathBuf;

use image::ExtendedColorType;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::imageops::FilterType;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let ico_path = out.join("snag.ico");

    let src = image::open("assets/icon.png").expect("assets/icon.png").to_rgba8();
    let pngs: Vec<(u32, Vec<u8>)> = [16u32, 20, 24, 32, 40, 48, 64, 128, 256]
        .iter()
        .map(|&s| (s, image::imageops::resize(&src, s, s, FilterType::Lanczos3).into_raw()))
        .collect();
    let frames: Vec<IcoFrame> = pngs
        .iter()
        .map(|(s, px)| IcoFrame::as_png(px, *s, *s, ExtendedColorType::Rgba8).expect("encode icon frame"))
        .collect();
    IcoEncoder::new(File::create(&ico_path).expect("create icon")).encode_images(&frames).expect("write icon");

    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path.to_str().expect("utf-8 path"))
        .set("ProductName", "Snagger")
        .set("FileDescription", "Snagger video downloader")
        .set("LegalCopyright", "MIT licensed");
    res.compile().expect("embed resources (needs the Windows SDK's rc.exe)");
}
