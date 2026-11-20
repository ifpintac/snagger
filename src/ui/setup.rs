//! The first-run screen while Snag fetches its engine.

use eframe::egui::{self, Ui};

use super::icons::{self, Icon};
use super::theme::tk;
use super::widgets::{self, Button, Tone, muted};
use crate::engine::{self, Part};
use crate::hub::Hub;
use crate::util;

pub fn show(ui: &mut Ui, hub: &Hub) {
    let e = hub.lock().engine.clone();
    let t = tk(ui);
    ui.add_space(t.s(8));
    ui.vertical_centered(|ui| {
        ui.set_max_width(560.0);
        widgets::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                widgets::title(ui, "Setting up Snag");
                ui.add_space(t.s(1));
                widgets::hint(
                    ui,
                    "Snag fetches its download engine once, from each project's official GitHub releases, into your user folder. Nothing is installed system-wide.",
                );
                ui.add_space(t.s(4));
                for part in [Part::YtDlp, Part::Ffmpeg, Part::Deno] {
                    let (done, note) = match part {
                        Part::YtDlp => (e.ytdlp.is_some(), String::new()),
                        Part::Ffmpeg => {
                            (e.ffmpeg.is_some(), if e.ffmpeg_system { "found on this PC".into() } else { String::new() })
                        }
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
                        if done {
                            icons::show(ui, Icon::Done, t.icon_size, t.success_text);
                        } else {
                            icons::show(ui, Icon::Hourglass, t.icon_size, t.text2);
                        }
                        ui.label(widgets::strong(ui, part.name()));
                        let size = if note.is_empty() { part.approx_size().to_string() } else { note };
                        let state = if done { "ready" } else { "to fetch" };
                        ui.label(muted(ui, format!("{} · {size} · {state}", part.purpose())));
                    });
                }
                ui.add_space(t.s(4));
                if let Some(task) = &e.busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(t.icon_size).color(t.text2));
                        ui.label(task.title.as_str());
                        if task.total > 0 {
                            ui.label(muted(
                                ui,
                                format!(
                                    "{} of {}",
                                    util::fmt_bytes(task.done as f64),
                                    util::fmt_bytes(task.total as f64)
                                ),
                            ));
                        }
                    });
                    ui.add_space(t.s(1));
                    let w = ui.available_width();
                    if task.total > 0 {
                        widgets::progress(ui, w, task.done as f32 / task.total as f32, Tone::Busy, false);
                    } else {
                        widgets::progress(ui, w, 0.0, Tone::Busy, true);
                    }
                } else if !e.error.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        icons::show(ui, Icon::Alert, t.icon_size, t.danger_text);
                        ui.label(widgets::colored(ui, &e.error, t.danger_text));
                    });
                    ui.add_space(t.s(3));
                    ui.horizontal(|ui| {
                        if Button::primary("Try again").icon(Icon::Refresh).show(ui).clicked() {
                            engine::install_missing(hub);
                        }
                        if e.ytdlp.is_some() && widgets::button(ui, "Continue without it").clicked() {
                            hub.lock().engine.setting_up = false;
                        }
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(t.icon_size).color(t.text2));
                        ui.label("Checking what's already here…");
                    });
                }
            });
        });
    });
}
