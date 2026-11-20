//! Notices video links you copy anywhere (browser address bar, chat apps)
//! and either offers them or queues them straight away.

use std::thread;
use std::time::Duration;

use crate::hub::Hub;
use crate::platform;
use crate::settings::ClipMode;
use crate::util;

pub fn start(hub: &Hub) {
    let hub = hub.clone();
    let _ = thread::Builder::new().name("clipboard".into()).spawn(move || {
        // Whatever was on the clipboard before Snag opened is left alone.
        let mut last_seq = platform::clipboard_seq();
        let mut clipboard: Option<arboard::Clipboard> = None;
        let mut last_text = String::new();
        loop {
            thread::sleep(Duration::from_millis(600));
            let seq = platform::clipboard_seq();
            if seq == last_seq {
                continue;
            }
            last_seq = seq;
            let mode = hub.lock().settings.clipboard;
            if mode == ClipMode::Off {
                continue;
            }
            if clipboard.is_none() {
                clipboard = arboard::Clipboard::new().ok();
            }
            let Some(cb) = clipboard.as_mut() else { continue };
            let Ok(text) = cb.get_text() else { continue };
            if text == last_text || text.len() > 200_000 {
                continue;
            }
            last_text = text.clone();
            let links: Vec<String> =
                util::extract_links(&text).into_iter().filter(|l| util::is_media_link(l)).collect();
            if links.is_empty() {
                continue;
            }
            {
                let mut c = hub.lock();
                if !c.clip_ignore.is_empty() && text.trim() == c.clip_ignore.trim() {
                    continue;
                }
                let fresh: Vec<String> =
                    links.into_iter().filter(|l| !c.is_queued(l) && !c.handled.contains(l)).collect();
                if fresh.is_empty() {
                    continue;
                }
                match mode {
                    ClipMode::Ask => c.clip_offer = fresh,
                    ClipMode::Auto => {
                        let opts = c.settings.job_options();
                        let (added, _) = c.add_links(&fresh, &opts);
                        if added > 0 {
                            c.toast(if added == 1 {
                                "Added the link you copied".to_string()
                            } else {
                                format!("Added {added} links you copied")
                            });
                        }
                    }
                    ClipMode::Off => {}
                }
            }
            hub.wake();
            hub.repaint();
        }
    });
}
