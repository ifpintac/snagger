//! The Settings tab. Changes apply immediately and are saved automatically.

use eframe::egui::{self, RichText, Ui};
use egui_phosphor::regular as icon;

use super::Confirm;
use super::downloads::preset_picker;
use super::theme::Pal;
use super::widgets::{self, section, setting_row, small_muted};
use crate::engine::{self, Part};
use crate::hub::Hub;
use crate::platform;
use crate::settings::{self, BROWSERS, ClipMode, NameStyle, Organize, Settings, SubsMode, ThemePref};
use crate::util;

pub fn show(ui: &mut Ui, hub: &Hub, confirm: &mut Option<Confirm>) {
    let (mut s, engine_state, running) = {
        let c = hub.lock();
        (c.settings.clone(), c.engine.clone(), c.jobs.iter().any(|j| j.status == crate::model::Status::Running))
    };
    let before = s.clone();

    egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(860.0);
        downloads_section(ui, &mut s);
        quality_section(ui, &mut s);
        extras_section(ui, &mut s);
        signin_section(ui, &mut s);
        behaviour_section(ui, &mut s, confirm);
        engine_section(ui, hub, &mut s, &engine_state, running);
        advanced_section(ui, &mut s);
        about_section(ui);
    });

    if s != before {
        let mut c = hub.lock();
        c.settings = s.sanitized();
        c.dirty.settings = true;
        drop(c);
        hub.wake();
    }
}

fn downloads_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, &format!("{}  Downloads", icon::FOLDER_SIMPLE), |ui| {
        setting_row(ui, "Save to", "", |ui| {
            ui.label(RichText::new(s.download_dir.display().to_string()).monospace());
            if ui.button("Change…").clicked()
                && let Some(dir) = rfd::FileDialog::new()
                    .set_title("Where should Snag save downloads?")
                    .set_directory(&s.download_dir)
                    .pick_folder()
            {
                s.download_dir = dir;
            }
            if ui.button("Open").clicked() {
                let _ = std::fs::create_dir_all(&s.download_dir);
                platform::open_path(&s.download_dir);
            }
            if s.download_dir != settings::default_download_dir() && ui.button("Reset").clicked() {
                s.download_dir = settings::default_download_dir();
            }
        });
        setting_row(
            ui,
            "Downloads at once",
            "More at once finishes a big batch sooner, until your connection is the limit.",
            |ui| {
                ui.add(egui::Slider::new(&mut s.parallel, 1..=10));
            },
        );
        setting_row(ui, "Sort into folders", "", |ui| {
            egui::ComboBox::from_id_salt("organize")
                .selected_text(match s.organize {
                    Organize::None => "No, all in one folder",
                    Organize::BySite => "By site (youtube.com, tiktok.com…)",
                    Organize::ByUploader => "By channel or uploader",
                })
                .width(280.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.organize, Organize::None, "No, all in one folder");
                    ui.selectable_value(&mut s.organize, Organize::BySite, "By site (youtube.com, tiktok.com…)");
                    ui.selectable_value(&mut s.organize, Organize::ByUploader, "By channel or uploader");
                });
        });
        ui.label(small_muted(ui, "Playlists always get their own folder, with numbered files."));
        ui.add_space(4.0);
        setting_row(ui, "File names", s.name_style.example(), |ui| {
            egui::ComboBox::from_id_salt("names").selected_text(s.name_style.label()).width(220.0).show_ui(ui, |ui| {
                for n in NameStyle::ALL {
                    ui.selectable_value(&mut s.name_style, n, n.label());
                }
            });
        });
        if s.name_style == NameStyle::Custom {
            setting_row(
                ui,
                "Template",
                "yt-dlp output template without the extension. Fields: %(title)s %(id)s %(uploader)s %(upload_date)s %(channel)s… Slashes make folders.",
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut s.custom_template)
                            .desired_width(420.0)
                            .font(egui::TextStyle::Monospace),
                    );
                },
            );
        }
        setting_row(ui, "", "", |ui| {
            ui.checkbox(&mut s.restrict_names, "Plain file names (ASCII only, no spaces)");
        });
    });
}

fn quality_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, &format!("{}  Default quality", icon::FILM_STRIP), |ui| {
        ui.label(small_muted(
            ui,
            "Used for Add many, the clipboard and subscriptions. The preview also starts here and remembers your last pick.",
        ));
        ui.add_space(4.0);
        preset_picker(ui, "default", &mut s.preset);
        ui.add_space(4.0);
        ui.checkbox(&mut s.batch_playlists, "When a link is a video inside a playlist, download the whole playlist");
    });
}

fn extras_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, &format!("{}  Subtitles, artwork and tags", icon::SUBTITLES), |ui| {
        setting_row(ui, "Subtitles", "", |ui| {
            ui.selectable_value(&mut s.subtitles, SubsMode::Off, "Off");
            ui.selectable_value(&mut s.subtitles, SubsMode::Embed, "Inside the video");
            ui.selectable_value(&mut s.subtitles, SubsMode::Files, "Separate .srt files");
        });
        if s.subtitles != SubsMode::Off {
            setting_row(
                ui,
                "Languages",
                "Comma-separated codes. en.* matches every English variant; all gets everything.",
                |ui| {
                    ui.add(egui::TextEdit::singleline(&mut s.sub_langs).desired_width(200.0));
                    ui.checkbox(&mut s.auto_subs, "Include auto-generated captions");
                },
            );
        }
        setting_row(ui, "", "", |ui| {
            ui.vertical(|ui| {
                ui.checkbox(&mut s.embed_thumbnail, "Add the thumbnail as cover art (MP4, MKV, MP3, M4A, Opus, FLAC)");
                ui.checkbox(&mut s.write_thumbnail, "Also save the thumbnail as a .jpg next to the file");
                ui.checkbox(&mut s.embed_metadata, "Write title, artist, date and chapters into the file");
                ui.checkbox(
                    &mut s.sponsorblock,
                    "Cut sponsor segments and self-promotion out of YouTube videos (SponsorBlock)",
                );
            });
        });
    });
}

fn signin_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, &format!("{}  Sign-in and network", icon::COOKIE), |ui| {
        setting_row(
            ui,
            "Signed-in browser",
            "For private, age-restricted or members-only videos, Snag borrows that browser's sign-in. Firefox works most reliably; close Chrome or Edge first if they refuse.",
            |ui| {
                let label = BROWSERS.iter().find(|(id, _)| *id == s.cookies_browser).map(|(_, n)| *n).unwrap_or("None");
                egui::ComboBox::from_id_salt("browser").selected_text(label).show_ui(ui, |ui| {
                    for (id, name) in BROWSERS {
                        ui.selectable_value(&mut s.cookies_browser, id.to_string(), *name);
                    }
                });
            },
        );
        setting_row(
            ui,
            "Cookies file",
            "Instead of a browser: a cookies.txt exported with a browser extension.",
            |ui| {
                ui.add(egui::TextEdit::singleline(&mut s.cookies_file).desired_width(360.0).hint_text("None"));
                if ui.button("Choose…").clicked()
                    && let Some(f) = rfd::FileDialog::new().add_filter("Cookies", &["txt"]).pick_file()
                {
                    s.cookies_file = f.display().to_string();
                }
            },
        );
        setting_row(ui, "Proxy", "e.g. socks5://127.0.0.1:1080 or http://user:pass@host:port", |ui| {
            ui.add(egui::TextEdit::singleline(&mut s.proxy).desired_width(360.0).hint_text("None"));
        });
        setting_row(ui, "Speed limit", "Per download. 0 means no limit.", |ui| {
            let mut mb = s.speed_limit_kb as f32 / 1024.0;
            if ui
                .add(egui::DragValue::new(&mut mb).range(0.0..=1000.0).speed(0.1).max_decimals(1).suffix(" MB/s"))
                .changed()
            {
                s.speed_limit_kb = (mb * 1024.0).round() as u32;
            }
        });
        setting_row(ui, "Retries", "How often a dropped connection is retried before giving up.", |ui| {
            ui.add(egui::DragValue::new(&mut s.retries).range(0..=100));
            ui.label("Parallel pieces per download");
            ui.add(egui::DragValue::new(&mut s.fragments).range(1..=16));
        });
    });
}

fn behaviour_section(ui: &mut Ui, s: &mut Settings, confirm: &mut Option<Confirm>) {
    section(ui, &format!("{}  Behaviour", icon::GEAR), |ui| {
        setting_row(
            ui,
            "Copied links",
            "Reacts to links from YouTube, TikTok, Instagram, X, Reddit, Facebook, Vimeo, Twitch, SoundCloud and other video sites.",
            |ui| {
                ui.selectable_value(&mut s.clipboard, ClipMode::Ask, "Offer to download");
                ui.selectable_value(&mut s.clipboard, ClipMode::Auto, "Download right away");
                ui.selectable_value(&mut s.clipboard, ClipMode::Off, "Ignore");
            },
        );
        setting_row(ui, "", "", |ui| {
            ui.vertical(|ui| {
                ui.checkbox(&mut s.resume_on_start, "Continue unfinished downloads when Snag opens");
                ui.checkbox(&mut s.keep_awake, "Keep the PC awake while downloading");
                ui.checkbox(&mut s.notify_done, "Flash the taskbar and chime when a batch finishes");
                ui.checkbox(
                    &mut s.use_archive,
                    "Skip videos Snag has downloaded before (even if you deleted the file)",
                );
            });
        });
        if s.use_archive || settings::archive_path().exists() {
            let n = std::fs::read_to_string(settings::archive_path()).map(|t| t.lines().count()).unwrap_or(0);
            setting_row(ui, "", "", |ui| {
                ui.label(small_muted(ui, format!("{n} videos remembered")));
                if n > 0 && ui.button("Forget them").clicked() {
                    *confirm = Some(Confirm::ClearArchive);
                }
            });
        }
        setting_row(ui, "Theme", "", |ui| {
            ui.selectable_value(&mut s.theme, ThemePref::System, "Match Windows");
            ui.selectable_value(&mut s.theme, ThemePref::Light, "Light");
            ui.selectable_value(&mut s.theme, ThemePref::Dark, "Dark");
        });
    });
}

fn engine_section(ui: &mut Ui, hub: &Hub, s: &mut Settings, e: &engine::EngineState, running: bool) {
    section(ui, &format!("{}  Engine", icon::HARD_DRIVES), |ui| {
        let pal = Pal::of(ui);
        ui.label(small_muted(
            ui,
            "Sites change how they serve video all the time; a fresh yt-dlp fixes most failures within a day or two.",
        ));
        ui.add_space(4.0);
        let busy = e.busy.is_some();
        let status = |ok: bool, text: String| RichText::new(text).color(if ok { pal.text } else { pal.red });
        setting_row(ui, "yt-dlp", "Reads the sites and downloads. Updated from GitHub.", |ui| {
            let v = if e.ytdlp.is_some() { format!("Version {}", e.ytdlp_version) } else { "Not installed".into() };
            ui.label(status(e.ytdlp.is_some(), v));
            if !e.latest.is_empty() && e.latest != e.ytdlp_version {
                ui.label(RichText::new(format!("{} is available", e.latest)).color(pal.amber));
            }
            if ui.add_enabled(!busy && !running, egui::Button::new("Check for updates")).clicked() {
                engine::check_updates(hub);
            }
            if ui.add_enabled(!busy && !running, egui::Button::new("Reinstall")).clicked() {
                engine::reinstall(hub, Part::YtDlp);
            }
        });
        setting_row(ui, "", "", |ui| {
            ui.checkbox(&mut s.auto_update, "Update automatically when Snag starts");
            ui.checkbox(&mut s.nightly, "Use nightly builds (fixes arrive sooner)");
        });
        setting_row(ui, "ffmpeg", "Joins video and audio, converts formats, trims.", |ui| {
            let text = match (&e.ffmpeg, e.ffmpeg_system) {
                (Some(p), true) => format!("Using the copy on this PC ({})", p.display()),
                (Some(_), false) => "Installed".into(),
                (None, _) => "Not installed".into(),
            };
            ui.label(status(e.ffmpeg.is_some(), text));
            let label = if e.ffmpeg.is_some() && !e.ffmpeg_system { "Reinstall" } else { "Install" };
            if ui.add_enabled(!busy && !running, egui::Button::new(label)).clicked() {
                engine::reinstall(hub, Part::Ffmpeg);
            }
        });
        setting_row(ui, "JavaScript runtime", "YouTube needs one to unlock its video links.", |ui| {
            let text = match &e.js {
                Some(j) if j.system => {
                    format!("Using {} on this PC", if j.kind == "node" { "Node.js" } else { "Deno" })
                }
                Some(_) => "Deno installed".into(),
                None => "Not installed".into(),
            };
            ui.label(status(e.js.is_some(), text));
            let label = if e.js.as_ref().is_some_and(|j| !j.system) { "Reinstall Deno" } else { "Install Deno" };
            if ui.add_enabled(!busy && !running, egui::Button::new(label)).clicked() {
                engine::reinstall(hub, Part::Deno);
            }
        });
        if let Some(t) = &e.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(&t.title);
                if t.total > 0 {
                    ui.label(small_muted(
                        ui,
                        format!("{} of {}", util::fmt_bytes(t.done as f64), util::fmt_bytes(t.total as f64)),
                    ));
                }
            });
            if t.total > 0 {
                widgets::bar(ui, ui.available_width().min(500.0), t.done as f32 / t.total as f32, pal.accent, false);
            }
        }
        if running && !busy {
            ui.label(small_muted(ui, "Engine changes wait until running downloads finish or are paused."));
        }
        if !e.error.is_empty() {
            ui.label(RichText::new(&e.error).color(pal.red));
        } else if !e.note.is_empty() {
            ui.label(RichText::new(&e.note).color(pal.green));
        }
        ui.horizontal(|ui| {
            if ui.button(format!("{}  Open engine folder", icon::FOLDER_OPEN)).clicked() {
                let _ = std::fs::create_dir_all(settings::engine_dir());
                platform::open_path(&settings::engine_dir());
            }
            if ui
                .button(format!("{}  Open data folder", icon::FOLDER_OPEN))
                .on_hover_text("Settings, queue, history")
                .clicked()
            {
                let _ = std::fs::create_dir_all(settings::data_dir());
                platform::open_path(&settings::data_dir());
            }
        });
    });
}

fn advanced_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, &format!("{}  Advanced", icon::TERMINAL_WINDOW), |ui| {
        setting_row(
            ui,
            "Extra yt-dlp options",
            "Added to every download, e.g. --sleep-requests 1 or --geo-bypass. Quote values with spaces.",
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut s.extra_args).desired_width(460.0).font(egui::TextStyle::Monospace),
                );
            },
        );
    });
}

fn about_section(ui: &mut Ui) {
    section(ui, &format!("{}  About", icon::INFO), |ui| {
        ui.label(format!(
            "Snag {}. Everything runs on this PC; nothing is uploaded anywhere.",
            env!("CARGO_PKG_VERSION")
        ));
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(small_muted(ui, "Built on"));
            for (name, url) in [
                ("yt-dlp", "https://github.com/yt-dlp/yt-dlp"),
                ("ffmpeg", "https://github.com/yt-dlp/FFmpeg-Builds"),
                ("Deno", "https://deno.com"),
                ("egui", "https://github.com/emilk/egui"),
                ("Phosphor Icons", "https://phosphoricons.com"),
                ("Fluent Emoji (app icon)", "https://github.com/microsoft/fluentui-emoji"),
            ] {
                ui.hyperlink_to(name, url);
            }
        });
        ui.label(small_muted(
            ui,
            "Only download videos you have the right to keep: your own uploads, Creative Commons work, or videos the creator allows you to save.",
        ));
    });
}
