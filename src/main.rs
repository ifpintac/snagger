//! Snagger: a private video and audio downloader for the desktop.
//!
//! The window is native (egui). Downloads are done by yt-dlp, which Snagger
//! installs, updates and drives for you; nothing needs Python.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod background;
mod clipwatch;
mod direct;
mod engine;
mod extension;
mod hub;
mod ipc;
mod manager;
mod model;
mod platform;
mod settings;
mod tools;
mod transcript;
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
        if util::from_snag_scheme(&arg).is_some() {
            // From the browser button or extension: kept whole (see SnagApp).
            links.push(arg.clone());
        } else if path.is_file() {
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

/// The browser button (`snag:` links) was set up for Snag.exe, which this
/// renamed app replaces: point it here. Development builds never take it.
fn relink_browser_button() {
    if cfg!(debug_assertions) {
        return;
    }
    if let (Some(cmd), Ok(exe)) = (platform::protocol_command(), std::env::current_exe())
        && util::button_needs_relink(&cmd, &exe)
    {
        let _ = platform::register_protocol(&exe);
    }
}

fn main() -> eframe::Result {
    settings::migrate_from_snag();
    relink_browser_button();
    let links = links_from_args();
    // Started with Windows: run in the tray without showing the window.
    let start_hidden = std::env::args().any(|a| a == "--background");
    // Already open? Hand the links over and bring that window forward.
    if ipc::forward_to_running(&links) {
        return Ok(());
    }

    let hub = hub::Hub::load();
    background::register_notifications();
    extension::refresh_if_set_up();
    ipc::listen(&hub);
    engine::start(&hub);
    manager::start(&hub);
    clipwatch::start(&hub);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Snagger")
            .with_app_id("snag")
            .with_inner_size([1060.0, 760.0])
            .with_min_inner_size([760.0, 520.0])
            .with_icon(app_icon())
            .with_drag_and_drop(true),
        centered: true,
        persist_window: true,
        ..Default::default()
    };
    eframe::run_native(
        "Snagger",
        options,
        Box::new(move |cc| Ok(Box::new(ui::SnagApp::new(cc, hub, links, start_hidden)))),
    )
}
