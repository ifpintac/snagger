//! Menus and dialogs for the after-download tools (see `crate::tools`).

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Id, Label, Layout, Ui};

use super::icons::Icon;
use super::theme::tk;
use super::widgets::{self, Button, chip, menu_item};
use crate::hub::Hub;
use crate::tools::{GIF_MAX_SECONDS, Tool};
use crate::util;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolKind {
    Shrink,
    Gif,
    Split,
}

const SIZES: [(u32, &str); 4] = [(10, "Discord"), (25, "Email"), (50, ""), (100, "")];

enum Dialog {
    Shrink { media: PathBuf, title: String, mb: u32, other: String },
    Gif { media: PathBuf, title: String, from: String, to: String, width: u32, fps: u32 },
}

#[derive(Default)]
pub struct ToolsUi {
    dialog: Option<Dialog>,
}

/// Files with a picture (a GIF can be made from them).
pub fn is_video(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "mp4" | "mkv" | "webm" | "mov" | "m4v" | "avi" | "flv")
}

/// The tool items for a finished file's menu.
pub fn menu(ui: &mut Ui, media: &Path, picked: &mut Option<ToolKind>) {
    if menu_item(ui, Some(Icon::Package), "Make smaller…").clicked() {
        *picked = Some(ToolKind::Shrink);
    }
    if is_video(media) && menu_item(ui, Some(Icon::Image), "Make a GIF…").clicked() {
        *picked = Some(ToolKind::Gif);
    }
    if menu_item(ui, Some(Icon::Scissor), "Split into tracks by chapters").clicked() {
        *picked = Some(ToolKind::Split);
    }
}

impl ToolsUi {
    /// Opens the tool's dialog, or runs it straight away when it has no options.
    pub fn open(&mut self, hub: &Hub, media: PathBuf, title: String, kind: ToolKind, duration: u64) {
        match kind {
            ToolKind::Split => crate::tools::run_in_background(hub, media, title, Tool::SplitChapters),
            ToolKind::Shrink => self.dialog = Some(Dialog::Shrink { media, title, mb: 10, other: String::new() }),
            ToolKind::Gif => {
                let end = if duration > 0 { (duration as f64).min(10.0) } else { 10.0 };
                self.dialog = Some(Dialog::Gif {
                    media,
                    title,
                    from: "0:00".into(),
                    to: util::fmt_timestamp(end),
                    width: 480,
                    fps: 15,
                });
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, hub: &Hub) {
        let Some(dialog) = self.dialog.as_mut() else { return };
        let mut done: Option<Option<(PathBuf, String, Tool)>> = None;
        let modal = widgets::modal(ctx, Id::new("tool-dialog"), |ui| {
            let t = tk(ui);
            ui.set_width(480.0);
            match dialog {
                Dialog::Shrink { media, title, mb, other } => {
                    widgets::icon_heading(ui, Icon::Package, "Make smaller");
                    ui.add_space(t.s(1));
                    ui.add(Label::new(widgets::muted(ui, title.as_str())).truncate());
                    ui.add_space(t.s(4));
                    ui.label(widgets::field_label(ui, "Fit in"));
                    ui.add_space(t.s(1));
                    ui.horizontal_wrapped(|ui| {
                        for (size, note) in SIZES {
                            if chip(ui, *mb == size && other.trim().is_empty(), &format!("{size} MB"), note).clicked() {
                                *mb = size;
                                other.clear();
                            }
                        }
                        ui.add_space(t.s(2));
                        let edit = egui::TextEdit::singleline(other).hint_text("Other");
                        widgets::input(ui, edit, None, 70.0);
                        ui.label(widgets::muted(ui, "MB"));
                    });
                    let custom = other.trim().parse::<u32>().ok().filter(|v| (1..=100_000).contains(v));
                    let bad = !other.trim().is_empty() && custom.is_none();
                    ui.add_space(t.s(3));
                    if bad {
                        ui.label(widgets::colored(ui, "Type a whole number of megabytes, like 8.", t.danger_text));
                    } else {
                        widgets::hint(
                            ui,
                            "Saves a new copy next to the original, which stays as it is. The longer the file, the lower its quality at a small size.",
                        );
                    }
                    ui.add_space(t.s(6));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if Button::primary("Make it smaller").icon(Icon::Package).enabled(!bad).show(ui).clicked() {
                            let megabytes = custom.unwrap_or(*mb);
                            done = Some(Some((media.clone(), title.clone(), Tool::Shrink { megabytes })));
                        }
                        if widgets::button(ui, "Cancel").clicked() {
                            done = Some(None);
                        }
                    });
                }
                Dialog::Gif { media, title, from, to, width, fps } => {
                    widgets::icon_heading(ui, Icon::Image, "Make a GIF");
                    ui.add_space(t.s(1));
                    ui.add(Label::new(widgets::muted(ui, title.as_str())).truncate());
                    ui.add_space(t.s(4));
                    ui.horizontal(|ui| {
                        ui.label(widgets::field_label(ui, "From"));
                        widgets::input(ui, egui::TextEdit::singleline(from), None, 80.0);
                        ui.add_space(t.s(2));
                        ui.label(widgets::field_label(ui, "To"));
                        widgets::input(ui, egui::TextEdit::singleline(to), None, 80.0);
                    });
                    ui.add_space(t.s(3));
                    ui.horizontal(|ui| {
                        ui.label(widgets::field_label(ui, "Width"));
                        widgets::segmented(ui, width, &[(320, "320 px"), (480, "480 px"), (640, "640 px")]);
                        ui.add_space(t.s(3));
                        ui.label(widgets::field_label(ui, "Frames"));
                        widgets::segmented(ui, fps, &[(10, "10 / s"), (15, "15 / s"), (24, "24 / s")]);
                    });
                    let range = match (util::parse_timestamp(from), util::parse_timestamp(to)) {
                        (Some(a), Some(b)) if b > a && b - a <= GIF_MAX_SECONDS => Ok((a, b)),
                        (Some(a), Some(b)) if b > a => {
                            Err(format!("Up to {} seconds; pick a shorter part.", GIF_MAX_SECONDS as u32))
                        }
                        (Some(_), Some(_)) => Err("The end has to come after the start.".to_string()),
                        _ => Err("Type times like 0:05 or 1:30.".to_string()),
                    };
                    ui.add_space(t.s(3));
                    match &range {
                        Ok((a, b)) => {
                            widgets::hint(
                                ui,
                                &format!("{:.0} seconds, saved as a looping GIF next to the video.", (b - a).max(1.0)),
                            );
                        }
                        Err(e) => {
                            ui.label(widgets::colored(ui, e.as_str(), t.danger_text));
                        }
                    }
                    ui.add_space(t.s(6));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if Button::primary("Make the GIF").icon(Icon::Image).enabled(range.is_ok()).show(ui).clicked()
                            && let Ok((a, b)) = range
                        {
                            let tool = Tool::Gif { from: a, to: b, width: *width, fps: *fps };
                            done = Some(Some((media.clone(), title.clone(), tool)));
                        }
                        if widgets::button(ui, "Cancel").clicked() {
                            done = Some(None);
                        }
                    });
                }
            }
        });
        if modal.should_close() && done.is_none() {
            done = Some(None);
        }
        if let Some(choice) = done {
            self.dialog = None;
            if let Some((media, title, tool)) = choice {
                crate::tools::run_in_background(hub, media, title, tool);
            }
        }
    }
}
