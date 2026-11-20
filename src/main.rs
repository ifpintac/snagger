//! Snag: a private video and audio downloader for the desktop.
//!
//! The window is native (egui). Downloads are done by yt-dlp, which Snag
//! installs, updates and drives for you; nothing needs Python.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipwatch;
mod engine;
mod hub;
mod ipc;
mod manager;
mod model;
mod platform;
mod settings;
mod ui;
mod util;
mod worker;
mod ytdlp;

use std::path::Path;

use eframe::egui;

/// Links from the command line: URLs, or files full of them.
fn links_from_args() -> Vec<String> {
    let mut links = Vec::new();
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        if path.is_file() {
            if let Ok(bytes) = std::fs::read(path) {
                let text = String::from_utf8_lossy(&bytes);
                let is_url_file = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("url"));
                links.extend(if is_url_file { util::links_from_url_file(&text) } else { util::extract_links(&text) });
            }
        } else if let Some(one) = util::parse_single_link(&arg) {
            links.push(one);
        }
    }
    let mut seen = std::collections::HashSet::new();
    links.retain(|l| seen.insert(l.clone()));
    links
}

fn app_icon() -> egui::IconData {
    match image::load_from_memory(include_bytes!("../assets/icon.png")) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
        }
        Err(_) => egui::IconData::default(),
    }
}

fn main() -> eframe::Result {
    let links = links_from_args();
    // Already open? Hand the links over and bring that window forward.
    if ipc::forward_to_running(&links) {
        return Ok(());
    }

    let hub = hub::Hub::load();
    ipc::listen(&hub);
    engine::start(&hub);
    manager::start(&hub);
    clipwatch::start(&hub);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Snag")
            .with_app_id("snag")
            .with_inner_size([1060.0, 760.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(app_icon())
            .with_drag_and_drop(true),
        centered: true,
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native("Snag", options, Box::new(move |cc| Ok(Box::new(ui::SnagApp::new(cc, hub, links)))))
}
