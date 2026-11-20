//! The first-run screen while Snag fetches its engine.

use eframe::egui::{RichText, Ui};
use egui_phosphor::regular as icon;

use super::theme::{Pal, bold};
use super::widgets::{self, small_muted};
use crate::engine::{self, Part};
use crate::hub::Hub;
use crate::util;

pub fn show(ui: &mut Ui, hub: &Hub) {
    let e = hub.lock().engine.clone();
    let pal = Pal::of(ui);
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(560.0);
        widgets::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Setting up Snag").family(bold()).size(20.0));
            ui.add_space(4.0);
            ui.label(small_muted(
                ui,
                "Snag fetches its download engine once, from each project's official GitHub releases, into your user folder. Nothing is installed system-wide.",
            ));
            ui.add_space(10.0);
            for part in [Part::YtDlp, Part::Ffmpeg, Part::Deno] {
                let (done, note) = match part {
                    Part::YtDlp => (e.ytdlp.is_some(), String::new()),
                    Part::Ffmpeg => (e.ffmpeg.is_some(), if e.ffmpeg_system { "found on this PC".into() } else { String::new() }),
                    Part::Deno => (
                        e.js.is_some(),
                        match &e.js {
                            Some(j) if j.kind == "node" => "using Node.js on this PC".into(),
                            Some(j) if j.system => "found on this PC".into(),
                            _ => String::new(),
                        },
                    ),
                };
                ui.horizontal(|ui| {
                    let mark = if done {
                        RichText::new(icon::CHECK_CIRCLE).color(pal.green)
                    } else {
                        RichText::new(icon::CIRCLE_NOTCH).color(pal.muted)
                    };
                    ui.label(mark.size(18.0));
                    ui.label(RichText::new(part.name()).family(bold()));
                    ui.label(small_muted(ui, format!("{} · {}", part.purpose(), if note.is_empty() { part.approx_size().to_string() } else { note })));
                });
            }
            ui.add_space(10.0);
            if let Some(t) = &e.busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(&t.title);
                    if t.total > 0 {
                        ui.label(small_muted(ui, format!("{} of {}", util::fmt_bytes(t.done as f64), util::fmt_bytes(t.total as f64))));
                    }
                });
                if t.total > 0 {
                    widgets::bar(ui, ui.available_width(), t.done as f32 / t.total as f32, pal.accent, false);
                } else {
                    widgets::bar(ui, ui.available_width(), 0.0, pal.accent, true);
                }
            } else if !e.error.is_empty() {
                ui.label(RichText::new(&e.error).color(pal.red));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if widgets::primary_button(ui, "Try again", true).clicked() {
                        engine::install_missing(hub);
                    }
                    if e.ytdlp.is_some() && ui.button("Continue without it").clicked() {
                        hub.lock().engine.setting_up = false;
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking what's already here…");
                });
            }
        });
    });
}
