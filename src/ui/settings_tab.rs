//! The Settings tab. Changes apply immediately and are saved automatically.

use eframe::egui::{self, RichText, Ui};

use super::Confirm;
use super::downloads::preset_picker;
use super::icons::{self, Icon};
use super::theme::tk;
use super::widgets::{self, Button, checkbox, muted, section, segmented, setting_row};
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

    let t = tk(ui);
    egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        // Room inside the scroll clip for the cards' shadows.
        egui::Frame::new().inner_margin(egui::Margin::symmetric(t.s(4) as i8, t.s(2) as i8)).show(ui, |ui| {
            ui.set_max_width(860.0);
            ui.horizontal(|ui| {
                widgets::title(ui, "Settings");
                ui.label(muted(ui, "Changes save as you make them."));
            });
            ui.add_space(t.s(4));
            sections(ui, hub, &mut s, &engine_state, running, confirm);
        });
    });

    if s != before {
        let mut c = hub.lock();
        c.settings = s.sanitized();
        c.dirty.settings = true;
        drop(c);
        hub.wake();
    }
}

fn sections(
    ui: &mut Ui,
    hub: &Hub,
    s: &mut Settings,
    engine_state: &engine::EngineState,
    running: bool,
    confirm: &mut Option<Confirm>,
) {
    downloads_section(ui, s);
    quality_section(ui, s);
    extras_section(ui, s);
    signin_section(ui, s);
    behaviour_section(ui, s, confirm);
    browser_section(ui, hub);
    engine_section(ui, hub, s, engine_state, running);
    advanced_section(ui, s);
    about_section(ui);
}

fn downloads_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, Icon::Folder, "Downloads", |ui| {
        setting_row(ui, "Save to", "", |ui| {
            ui.label(RichText::new(s.download_dir.display().to_string()).monospace());
            if Button::secondary("Change…").icon(Icon::Folder).show(ui).clicked()
                && let Some(dir) = rfd::FileDialog::new()
                    .set_title("Where should Snagger save downloads?")
                    .set_directory(&s.download_dir)
                    .pick_folder()
            {
                s.download_dir = dir;
            }
            if widgets::button(ui, "Open").clicked() {
                let _ = std::fs::create_dir_all(&s.download_dir);
                platform::open_path(&s.download_dir);
            }
            if s.download_dir != settings::default_download_dir() && Button::quiet("Reset").show(ui).clicked() {
                s.download_dir = settings::default_download_dir();
            }
        });
        setting_row(
            ui,
            "Downloads at once",
            "More at once finishes a big batch sooner, until your connection is the limit.",
            |ui| {
                widgets::slider(ui, egui::Slider::new(&mut s.parallel, 1..=10));
            },
        );
        setting_row(
            ui,
            "Connections per file",
            "For plain file links (.zip, .pdf, .psd…), which Snagger downloads with its own engine.",
            |ui| {
                widgets::slider(ui, egui::Slider::new(&mut s.connections, 1..=16));
            },
        );
        setting_row(
            ui,
            "Download hours",
            "Downloads you add outside these hours wait in the queue, e.g. overnight on a metered connection.",
            |ui| {
                checkbox(ui, &mut s.window_on, "Only start downloads between");
                ui.add_enabled_ui(s.window_on, |ui| {
                    time_picker(ui, "window-start", &mut s.window_start);
                    ui.label("and");
                    time_picker(ui, "window-end", &mut s.window_end);
                });
            },
        );
        setting_row(ui, "Sort into folders", "Playlists always get their own folder, with numbered files.", |ui| {
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
                    let edit = egui::TextEdit::singleline(&mut s.custom_template).font(egui::TextStyle::Monospace);
                    widgets::input(ui, edit, None, 420.0);
                },
            );
        }
        setting_row(ui, "", "", |ui| {
            checkbox(ui, &mut s.restrict_names, "Plain file names (ASCII only, no spaces)");
        });
        ui.add_space(widgets::SECTION_GAP);
        widgets::well(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.label(widgets::field_label(ui, "How files will be named"));
                for (what, playlist) in [("A video", false), ("From a playlist", true)] {
                    ui.horizontal(|ui| {
                        ui.label(muted(ui, format!("{what}:")));
                        ui.label(muted(ui, "(download folder)\\").monospace());
                        let name = crate::ytdlp::sample_name(s, playlist);
                        ui.add(egui::Label::new(RichText::new(name).monospace()).truncate());
                    });
                }
            });
        });
    });
}

/// A time of day in half-hour steps.
fn time_picker(ui: &mut Ui, id: &str, minute: &mut u16) {
    egui::ComboBox::from_id_salt(id).selected_text(settings::fmt_minute(*minute)).width(80.0).show_ui(ui, |ui| {
        for m in (0..24 * 60).step_by(30) {
            ui.selectable_value(minute, m as u16, settings::fmt_minute(m as u16));
        }
    });
}

fn quality_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, Icon::Video, "Default quality", |ui| {
        widgets::hint(
            ui,
            "Used for Add many, the clipboard and subscriptions. The preview also starts here and remembers your last pick.",
        );
        ui.add_space(widgets::SECTION_GAP);
        preset_picker(ui, "default", &mut s.preset);
        ui.add_space(widgets::SECTION_GAP);
        checkbox(ui, &mut s.batch_playlists, "When a link is a video inside a playlist, download the whole playlist");
    });
}

fn extras_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, Icon::Captions, "Subtitles, artwork and tags", |ui| {
        setting_row(ui, "Subtitles", "", |ui| {
            segmented(
                ui,
                &mut s.subtitles,
                &[
                    (SubsMode::Off, "Off"),
                    (SubsMode::Embed, "Inside the video"),
                    (SubsMode::Files, "Separate .srt files"),
                ],
            );
        });
        if s.subtitles != SubsMode::Off {
            setting_row(
                ui,
                "Languages",
                "Comma-separated codes. en.* matches every English variant; all gets everything.",
                |ui| {
                    widgets::input(ui, egui::TextEdit::singleline(&mut s.sub_langs), None, 200.0);
                    checkbox(ui, &mut s.auto_subs, "Include auto-generated captions");
                },
            );
        }
        setting_row(ui, "", "", |ui| {
            ui.vertical(|ui| {
                checkbox(ui, &mut s.embed_thumbnail, "Add the thumbnail as cover art (MP4, MKV, MP3, M4A, Opus, FLAC)");
                checkbox(ui, &mut s.write_thumbnail, "Also save the thumbnail as a .jpg next to the file");
                checkbox(ui, &mut s.embed_metadata, "Write title, artist, date and chapters into the file");
                checkbox(
                    ui,
                    &mut s.sponsorblock,
                    "Cut sponsor segments and self-promotion out of YouTube videos (SponsorBlock)",
                );
            });
        });
    });
}

fn signin_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, Icon::Cookie, "Sign-in and network", |ui| {
        setting_row(
            ui,
            "Signed-in browser",
            "For private, age-restricted or members-only videos, Snagger borrows that browser's sign-in. Firefox works most reliably; close Chrome or Edge first if they refuse.",
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
                widgets::input(ui, egui::TextEdit::singleline(&mut s.cookies_file).hint_text("None"), None, 360.0);
                if widgets::button(ui, "Choose…").clicked()
                    && let Some(f) = rfd::FileDialog::new().add_filter("Cookies", &["txt"]).pick_file()
                {
                    s.cookies_file = f.display().to_string();
                }
            },
        );
        setting_row(ui, "Proxy", "e.g. socks5://127.0.0.1:1080 or http://user:pass@host:port", |ui| {
            widgets::input(ui, egui::TextEdit::singleline(&mut s.proxy).hint_text("None"), None, 360.0);
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
            ui.add_space(widgets::SECTION_GAP);
            ui.label(widgets::field_label(ui, "Parallel pieces per download"));
            ui.add(egui::DragValue::new(&mut s.fragments).range(1..=16));
        });
    });
}

fn behaviour_section(ui: &mut Ui, s: &mut Settings, confirm: &mut Option<Confirm>) {
    section(ui, Icon::Settings, "Behaviour", |ui| {
        setting_row(
            ui,
            "Copied links",
            "Reacts to links from YouTube, TikTok, Instagram, X, Reddit, Facebook, Vimeo, Twitch, SoundCloud and other video sites.",
            |ui| {
                segmented(
                    ui,
                    &mut s.clipboard,
                    &[
                        (ClipMode::Ask, "Offer to download"),
                        (ClipMode::Auto, "Download right away"),
                        (ClipMode::Off, "Ignore"),
                    ],
                );
            },
        );
        setting_row(ui, "", "", |ui| {
            ui.vertical(|ui| {
                checkbox(ui, &mut s.resume_on_start, "Continue unfinished downloads when Snagger opens");
                checkbox(ui, &mut s.keep_awake, "Keep the PC awake while downloading");
                checkbox(ui, &mut s.notify_done, "Flash the taskbar and chime when a batch finishes");
                checkbox(
                    ui,
                    &mut s.auto_retry,
                    "Try again by itself after connection problems or \"too many requests\"",
                );
                checkbox(
                    ui,
                    &mut s.gentle_playlists,
                    "Pause a few seconds between playlist videos, so sites don't block you",
                );
                checkbox(
                    ui,
                    &mut s.use_archive,
                    "Skip videos Snagger has downloaded before (even if you deleted the file)",
                );
            });
        });
        if s.use_archive || settings::archive_path().exists() {
            let n = std::fs::read_to_string(settings::archive_path()).map(|t| t.lines().count()).unwrap_or(0);
            setting_row(ui, "", "", |ui| {
                ui.label(muted(ui, format!("{n} videos remembered")));
                if n > 0 && widgets::button(ui, "Forget them").clicked() {
                    *confirm = Some(Confirm::ClearArchive);
                }
            });
        }
        setting_row(ui, "Theme", "", |ui| {
            segmented(
                ui,
                &mut s.theme,
                &[(ThemePref::System, "Match Windows"), (ThemePref::Light, "Light"), (ThemePref::Dark, "Dark")],
            );
        });
    });
}

fn engine_section(ui: &mut Ui, hub: &Hub, s: &mut Settings, e: &engine::EngineState, running: bool) {
    section(ui, Icon::Cpu, "Engine", |ui| {
        let t = tk(ui);
        widgets::hint(
            ui,
            "Sites change how they serve video all the time; a fresh yt-dlp fixes most failures within a day or two.",
        );
        ui.add_space(widgets::SECTION_GAP);
        let busy = e.busy.is_some();
        let status = |ok: bool, text: String| RichText::new(text).color(if ok { t.text } else { t.danger_text });
        setting_row(ui, "yt-dlp", "Reads the sites and downloads. Updated from GitHub.", |ui| {
            let v = if e.ytdlp.is_some() { format!("Version {}", e.ytdlp_version) } else { "Not installed".into() };
            ui.label(status(e.ytdlp.is_some(), v));
            if !e.latest.is_empty() && e.latest != e.ytdlp_version {
                ui.label(RichText::new(format!("{} is available", e.latest)).color(t.warning_text));
            }
            if Button::secondary("Check for updates").icon(Icon::Refresh).enabled(!busy && !running).show(ui).clicked()
            {
                engine::check_updates(hub);
            }
            if Button::secondary("Reinstall").enabled(!busy && !running).show(ui).clicked() {
                engine::reinstall(hub, Part::YtDlp);
            }
        });
        setting_row(ui, "", "", |ui| {
            checkbox(ui, &mut s.auto_update, "Update automatically when Snagger starts");
            checkbox(ui, &mut s.nightly, "Use nightly builds (fixes arrive sooner)");
        });
        setting_row(ui, "ffmpeg", "Joins video and audio, converts formats, trims.", |ui| {
            let text = match (&e.ffmpeg, e.ffmpeg_system) {
                (Some(p), true) => format!("Using the copy on this PC ({})", p.display()),
                (Some(_), false) => "Installed".into(),
                (None, _) => "Not installed".into(),
            };
            ui.label(status(e.ffmpeg.is_some(), text));
            let label = if e.ffmpeg.is_some() && !e.ffmpeg_system { "Reinstall" } else { "Install" };
            if Button::secondary(label).enabled(!busy && !running).show(ui).clicked() {
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
            if Button::secondary(label).enabled(!busy && !running).show(ui).clicked() {
                engine::reinstall(hub, Part::Deno);
            }
        });
        if let Some(task) = &e.busy {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(t.icon_size).color(t.text2));
                ui.label(task.title.as_str());
                if task.total > 0 {
                    ui.label(muted(
                        ui,
                        format!("{} of {}", util::fmt_bytes(task.done as f64), util::fmt_bytes(task.total as f64)),
                    ));
                }
            });
            if task.total > 0 {
                let w = ui.available_width().min(500.0);
                widgets::progress(ui, w, task.done as f32 / task.total as f32, widgets::Tone::Busy, false);
            }
        }
        if running && !busy {
            ui.label(muted(ui, "Engine changes wait until running downloads finish or are paused."));
        }
        if !e.error.is_empty() {
            ui.horizontal_wrapped(|ui| {
                icons::show(ui, Icon::Alert, t.icon_size, t.danger_text);
                ui.label(widgets::colored(ui, &e.error, t.danger_text));
            });
        } else if !e.note.is_empty() {
            ui.horizontal_wrapped(|ui| {
                icons::show(ui, Icon::Done, t.icon_size, t.success_text);
                ui.label(widgets::colored(ui, &e.note, t.success_text));
            });
        }
        ui.add_space(widgets::SECTION_GAP);
        ui.horizontal(|ui| {
            if Button::secondary("Open engine folder").icon(Icon::Folder).show(ui).clicked() {
                let _ = std::fs::create_dir_all(settings::engine_dir());
                platform::open_path(&settings::engine_dir());
            }
            if Button::secondary("Open data folder")
                .icon(Icon::Folder)
                .tip("Settings, queue, history")
                .show(ui)
                .clicked()
            {
                let _ = std::fs::create_dir_all(settings::data_dir());
                platform::open_path(&settings::data_dir());
            }
        });
    });
}

fn browser_section(ui: &mut Ui, hub: &Hub) {
    section(ui, Icon::Window, "Browser button", |ui| {
        let t = tk(ui);
        widgets::hint(
            ui,
            "A “Snag it” button for your browser's bookmarks bar: on any video page, click it and that page opens in Snagger.",
        );
        ui.add_space(widgets::SECTION_GAP);
        let exe = std::env::current_exe().ok();
        let command = platform::protocol_command();
        let ours = match (&command, &exe) {
            (Some(cmd), Some(exe)) => cmd.to_lowercase().contains(&exe.display().to_string().to_lowercase()),
            _ => false,
        };
        if ours {
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Done, t.icon_size, t.success_text);
                ui.label(widgets::colored(ui, "Set up. The button opens this copy of Snagger.", t.success_text));
            });
        } else if command.is_some() {
            ui.horizontal(|ui| {
                icons::show(ui, Icon::Warning, t.icon_size, t.warning_text);
                ui.label(widgets::colored(
                    ui,
                    "Set up for another copy of Snagger. Set it up again to use this one.",
                    t.warning_text,
                ));
            });
        }
        ui.add_space(widgets::SECTION_GAP);
        ui.horizontal(|ui| {
            let label = if ours { "Show the button again" } else { "Set up the browser button" };
            if Button::primary(label).icon(Icon::Window).enabled(exe.is_some()).show(ui).clicked()
                && let Some(exe) = &exe
            {
                let result =
                    platform::register_protocol(exe).and_then(|_| write_button_page().map_err(|e| e.to_string()));
                match result {
                    Ok(page) => {
                        platform::open_path(&page);
                        hub.lock().toast("Drag the “Snag it” button from the page that opened to your bookmarks bar.");
                    }
                    Err(e) => hub.lock().toast(format!("Couldn't set up the browser button: {e}")),
                }
            }
            if command.is_some() && widgets::button(ui, "Remove").clicked() {
                platform::unregister_protocol();
            }
        });
    });
}

/// A small local page holding the draggable bookmarklet.
fn write_button_page() -> std::io::Result<std::path::PathBuf> {
    const PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Snag it: browser button</title>
<style>
:root{--bg:#06090E;--sunken:#0D1521;--border:#2A3539;--text:#AACFD1;--text2:#799497;--pri-bg:#AACFD1;--pri-fg:#111111;--r-btn:3px;--font-ui:"Saira",system-ui,sans-serif;--dk-mono:"Fira Code",ui-monospace,monospace}
@media (prefers-color-scheme:light){:root{--bg:#EEF1F4;--sunken:#E4E9EE;--border:#C5CED6;--text:#163D42;--text2:#4B6266;--pri-bg:#163D42;--pri-fg:#F2F7F7}}
body{font:13px/1.4 var(--font-ui);max-width:640px;margin:48px auto;padding:0 16px;color:var(--text);background:var(--bg)}
h1{font-weight:500;font-size:22px;text-transform:uppercase;letter-spacing:.12em;border-top:1px solid var(--border);padding-top:12px}
p,li{color:var(--text)} a{color:var(--text)}
a.btn{display:inline-block;padding:6px 14px;border-radius:var(--r-btn);background:var(--pri-bg);color:var(--pri-fg);font-weight:600;text-transform:uppercase;letter-spacing:.06em;text-decoration:none;cursor:grab}
li{margin:6px 0} code{font-family:var(--dk-mono);background:var(--sunken);padding:1px 6px;border-radius:2px}
</style></head><body>
<h1>Snag it</h1>
<p>Drag this button to your bookmarks bar:</p>
<p><a class="btn" href="javascript:(function(){location.href='snag:'+encodeURIComponent(location.href)})()">Snag it</a></p>
<ol>
<li>Show the bookmarks bar if it's hidden: <code>Ctrl+Shift+B</code> in Chrome and Edge, or View → Toolbars → Bookmarks Toolbar in Firefox.</li>
<li>Drag the blue button onto it.</li>
<li>On any video page, click <b>Snag it</b>. The first time, your browser asks whether to open Snagger: tick “Always allow” and choose Open.</li>
</ol>
<p>Try it here: <a href="snag:https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DjNQXAC9IVRw">send a test video to Snagger</a>.</p>
</body></html>
"#;
    let path = settings::data_dir().join("browser-button.html");
    std::fs::create_dir_all(settings::data_dir())?;
    std::fs::write(&path, PAGE)?;
    Ok(path)
}

fn advanced_section(ui: &mut Ui, s: &mut Settings) {
    section(ui, Icon::Terminal, "Advanced", |ui| {
        setting_row(
            ui,
            "Extra yt-dlp options",
            "Added to every download, e.g. --sleep-requests 1 or --geo-bypass. Quote values with spaces.",
            |ui| {
                let edit = egui::TextEdit::singleline(&mut s.extra_args).font(egui::TextStyle::Monospace);
                widgets::input(ui, edit, None, 460.0);
            },
        );
    });
}

fn about_section(ui: &mut Ui) {
    section(ui, Icon::Info, "About", |ui| {
        widgets::para(
            ui,
            format!("Snagger {}. Everything runs on this PC; nothing is uploaded anywhere.", env!("CARGO_PKG_VERSION")),
        );
        ui.add_space(widgets::SECTION_GAP);
        ui.horizontal_wrapped(|ui| {
            ui.label(muted(ui, "Built on"));
            for (name, url) in [
                ("yt-dlp", "https://github.com/yt-dlp/yt-dlp"),
                ("ffmpeg", "https://github.com/yt-dlp/FFmpeg-Builds"),
                ("Deno", "https://deno.com"),
                ("egui", "https://github.com/emilk/egui"),
                ("Phosphor icons", "https://phosphoricons.com"),
                ("Saira", "https://github.com/Omnibus-Type/Saira"),
                ("Fira Code", "https://github.com/tonsky/FiraCode"),
                ("Fluent Emoji (app icon)", "https://github.com/microsoft/fluentui-emoji"),
            ] {
                ui.hyperlink_to(name, url);
            }
        });
        ui.add_space(widgets::SECTION_GAP);
        widgets::hint(
            ui,
            "Only download videos you have the right to keep: your own uploads, Creative Commons work, or videos the creator allows you to save.",
        );
    });
}
