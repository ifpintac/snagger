//! Running in the background: the tray icon (closing the window keeps
//! Snagger downloading), the copy-and-download hotkey, and Windows
//! notifications.
//!
//! Tray and hotkey events arrive on the main thread from their own hidden
//! windows, even while Snagger's window is hidden (when egui doesn't run),
//! so they act directly on the shared state instead of waiting for a frame.

use std::sync::atomic::{AtomicBool, Ordering};

use crate::hub::Hub;
use crate::settings::Hotkey;

/// Set by the tray's "Quit": the next close really quits.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Notifications are shown under this name (registered per user).
pub const AUMID: &str = "Snagger.Downloader";

pub fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

#[cfg(windows)]
mod imp {
    use super::*;
    use eframe::egui;
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use crate::platform;
    use crate::util;

    pub struct Background {
        tray: Option<TrayIcon>,
        hotkeys: Option<GlobalHotKeyManager>,
        hotkey: Option<HotKey>,
        /// Why the hotkey couldn't be set (another app has it), for Settings.
        pub hotkey_error: String,
    }

    fn tray_icon_image() -> Option<tray_icon::Icon> {
        let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
        let img = img.resize_exact(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
        tray_icon::Icon::from_rgba(img.into_raw(), 32, 32).ok()
    }

    fn hotkey_of(choice: Hotkey) -> Option<HotKey> {
        match choice {
            Hotkey::Off => None,
            Hotkey::CtrlAltD => Some(HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyD)),
            Hotkey::CtrlShiftD => Some(HotKey::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyD)),
        }
    }

    impl Background {
        /// Sets up the event handlers once; the tray and hotkey follow Settings
        /// through [`Background::apply`].
        pub fn start(hub: &Hub, ctx: &egui::Context) -> Background {
            let show = MenuItem::new("Open Snagger", true, None);
            let pause = MenuItem::new("Pause all downloads", true, None);
            let resume = MenuItem::new("Resume all downloads", true, None);
            let quit = MenuItem::new("Quit Snagger", true, None);
            let ids = (show.id().clone(), pause.id().clone(), resume.id().clone(), quit.id().clone());
            let menu = Menu::new();
            let _ = menu.append_items(&[&show, &pause, &resume, &PredefinedMenuItem::separator(), &quit]);
            MENU.with(|m| *m.borrow_mut() = Some(menu));

            {
                let ctx = ctx.clone();
                TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left, button_state: MouseButtonState::Up, ..
                    } = e
                    {
                        platform::bring_to_front();
                        ctx.request_repaint();
                    }
                }));
            }
            {
                let (hub, ctx) = (hub.clone(), ctx.clone());
                MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                    let (show, pause, resume, quit) = &ids;
                    if e.id == *show {
                        platform::bring_to_front();
                    } else if e.id == *pause {
                        hub.lock().pause_all();
                        hub.wake();
                    } else if e.id == *resume {
                        hub.lock().resume_all();
                        hub.wake();
                    } else if e.id == *quit {
                        QUITTING.store(true, Ordering::SeqCst);
                        platform::bring_to_front();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ctx.request_repaint();
                }));
            }
            {
                let (hub, ctx) = (hub.clone(), ctx.clone());
                GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                    if e.state == HotKeyState::Pressed {
                        download_clipboard(&hub);
                        ctx.request_repaint();
                    }
                }));
            }
            Background {
                tray: None,
                hotkeys: GlobalHotKeyManager::new().ok(),
                hotkey: None,
                hotkey_error: String::new(),
            }
        }

        /// Shows or hides the tray icon and (re)registers the hotkey.
        pub fn apply(&mut self, tray: bool, hotkey: Hotkey) {
            if tray && self.tray.is_none() {
                let menu = MENU.with(|m| m.borrow().clone());
                let mut b = TrayIconBuilder::new().with_tooltip("Snagger").with_menu_on_left_click(false);
                if let Some(icon) = tray_icon_image() {
                    b = b.with_icon(icon);
                }
                if let Some(menu) = menu {
                    b = b.with_menu(Box::new(menu));
                }
                self.tray = b.build().ok();
            } else if !tray {
                self.tray = None;
            }
            let wanted = hotkey_of(hotkey);
            if wanted != self.hotkey
                && let Some(m) = &self.hotkeys
            {
                if let Some(old) = self.hotkey.take() {
                    let _ = m.unregister(old);
                }
                self.hotkey_error.clear();
                if let Some(new) = wanted {
                    match m.register(new) {
                        Ok(()) => self.hotkey = Some(new),
                        Err(_) => {
                            self.hotkey_error = "Another app already uses that shortcut. Pick the other one.".into();
                        }
                    }
                }
            }
        }

        pub fn has_tray(&self) -> bool {
            self.tray.is_some()
        }
    }

    thread_local! {
        static MENU: std::cell::RefCell<Option<Menu>> = const { std::cell::RefCell::new(None) };
    }

    /// The hotkey: every link on the clipboard goes into the queue with the
    /// default quality, and a notification says what happened.
    fn download_clipboard(hub: &Hub) {
        let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
        let mut links = util::extract_links(&text);
        if links.is_empty()
            && let Some(one) = util::parse_single_link(&text)
        {
            links.push(one);
        }
        if links.is_empty() {
            super::notify("No link to download", "Copy a video link first, then press the shortcut again.");
            return;
        }
        let (added, queued) = {
            let mut c = hub.lock();
            let opts = c.settings.job_options();
            let r = c.add_links(&links, &opts);
            c.handled.extend(links.iter().cloned());
            r
        };
        hub.wake();
        let body = match (added, queued) {
            (0, _) => "That link is already in the queue.".to_string(),
            (1, _) => format!("Downloading {}", util::short_url(&links[0])),
            (n, _) => format!("Downloading {n} links"),
        };
        super::notify("Added to Snagger", &body);
    }
}

#[cfg(windows)]
pub use imp::Background;

/// Shows a Windows notification. False when Windows didn't take it (then
/// the caller can flash the taskbar instead).
#[cfg(windows)]
pub fn notify(title: &str, body: &str) -> bool {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    let esc = |s: &str| {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    };
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        esc(title),
        esc(body)
    );
    let show = || -> windows::core::Result<()> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml.as_str()))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))?.Show(&toast)
    };
    show().is_ok()
}

#[cfg(not(windows))]
pub fn notify(_title: &str, _body: &str) -> bool {
    false
}

/// Registers Snagger's name and icon with Windows notifications, once a run.
pub fn register_notifications() {
    let icon = crate::settings::data_dir().join("snagger.png");
    if !icon.is_file() {
        let _ = std::fs::write(&icon, include_bytes!("../assets/icon.png"));
    }
    let _ = crate::platform::register_notifications(AUMID, &icon);
}
