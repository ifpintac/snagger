//! The queue manager. Runs on its own thread so downloads keep flowing while
//! the window is minimised: starts waiting jobs when a slot frees up, saves
//! state, keeps the PC awake, drives the taskbar progress and runs scheduled
//! subscription checks.

use std::collections::HashSet;
use std::fs;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::hub::{Core, Hub};
use crate::model::{Live, Stage, Status};
use crate::platform::{self, Bar, Taskbar};
use crate::settings::{self, save_text};
use crate::util;
use crate::worker;

pub fn start(hub: &Hub) {
    let (tx, rx) = mpsc::channel::<()>();
    hub.set_waker(tx);
    let hub = hub.clone();
    let _ = thread::Builder::new().name("queue".into()).spawn(move || {
        cleanup_leftovers(&hub);
        let mut m = Manager::new();
        loop {
            match rx.recv_timeout(Duration::from_millis(700)) {
                Err(RecvTimeoutError::Disconnected) => break,
                _ => while rx.try_recv().is_ok() {},
            }
            m.tick(&hub);
        }
    });
}

struct Manager {
    taskbar: Taskbar,
    awake: bool,
    was_active: bool,
    batch_started: u64,
    last_save: Instant,
    last_progress_save: Instant,
    last_sub_scan: Instant,
}

impl Manager {
    fn new() -> Manager {
        Manager {
            taskbar: Taskbar::new(),
            awake: false,
            was_active: false,
            batch_started: 0,
            last_save: Instant::now(),
            last_progress_save: Instant::now(),
            last_sub_scan: Instant::now() - Duration::from_secs(3600),
        }
    }

    fn tick(&mut self, hub: &Hub) {
        let mut to_start = Vec::new();
        let saves;
        let bar;
        let keep_awake;
        let mut notify = false;
        {
            let mut c = hub.lock();
            let engine_ok = c.engine.ready && c.engine.busy.is_none() && c.engine.ytdlp.is_some();
            let running = c.jobs.iter().filter(|j| j.status == Status::Running).count();
            if engine_ok && !c.queue_paused {
                let mut free = (c.settings.parallel as usize).saturating_sub(running);
                for j in c.jobs.iter_mut() {
                    if free == 0 {
                        break;
                    }
                    if j.status == Status::Queued {
                        j.status = Status::Running;
                        j.live = Live { stage: Stage::Starting, ..Default::default() };
                        let ctl = Hub::new_control();
                        j.control = Some(ctl.clone());
                        to_start.push((j.id.clone(), ctl));
                        free -= 1;
                    }
                }
                if !to_start.is_empty() {
                    c.dirty.queue = true;
                }
            }
            if engine_ok {
                self.scan_subscriptions(&mut c);
            }

            let active: Vec<_> = c.jobs.iter().filter(|j| j.status.is_active()).collect();
            let running_now = active.iter().filter(|j| j.status == Status::Running).count();
            let failed_in_batch = self.was_active
                && c.jobs.iter().any(|j| j.status == Status::Failed && j.finished >= self.batch_started);
            bar = if active.is_empty() {
                Bar::Off
            } else {
                let p = active.iter().map(|j| j.progress).sum::<f32>() / active.len() as f32;
                if running_now == 0 && c.queue_paused {
                    Bar::Paused(p)
                } else if failed_in_batch {
                    // Red, so a failure in a long batch is noticed from the taskbar.
                    Bar::Error(p)
                } else if p <= 0.001 {
                    Bar::Busy
                } else {
                    Bar::Normal(p)
                }
            };
            keep_awake = c.settings.keep_awake && running_now > 0;

            // One notification when a batch of downloads finishes.
            let now = util::now();
            if !active.is_empty() && !self.was_active {
                self.batch_started = now;
            }
            if self.was_active && active.is_empty() && c.settings.notify_done {
                notify = c.jobs.iter().any(|j| j.status == Status::Done && j.finished >= self.batch_started);
            }
            self.was_active = !active.is_empty();

            if running_now > 0 && self.last_progress_save.elapsed() > Duration::from_secs(15) {
                c.dirty.queue = true;
                self.last_progress_save = Instant::now();
            }
            saves = if self.last_save.elapsed() > Duration::from_millis(800) { take_saves(&mut c) } else { Vec::new() };
        }

        for (id, ctl) in to_start {
            worker::spawn(hub.clone(), id, ctl);
        }
        if !saves.is_empty() {
            write_saves(&saves);
            self.last_save = Instant::now();
        }
        if keep_awake != self.awake {
            platform::keep_awake(keep_awake);
            self.awake = keep_awake;
        }
        self.taskbar.set(bar);
        if notify {
            platform::notify_attention(true);
        }
    }

    /// Queues a check for every subscription that's due.
    fn scan_subscriptions(&mut self, c: &mut Core) {
        let Some(interval) = c.settings.sub_check.seconds() else { return };
        if self.last_sub_scan.elapsed() < Duration::from_secs(60) {
            return;
        }
        self.last_sub_scan = Instant::now();
        let now = util::now();
        let due: Vec<String> = c
            .subs
            .iter()
            .filter(|s| s.auto && s.state == crate::model::SubState::Idle)
            .filter(|s| now.saturating_sub(s.last_checked) >= interval)
            .map(|s| s.id.clone())
            .collect();
        for id in due {
            c.check_subscription(&id);
        }
    }
}

type Save = (&'static str, String);

fn take_saves(c: &mut Core) -> Vec<Save> {
    let mut out = Vec::new();
    let d = std::mem::take(&mut c.dirty);
    if d.queue {
        out.push(("queue.json", serde_json::to_string(&c.jobs).unwrap_or_default()));
    }
    if d.history {
        out.push(("history.json", serde_json::to_string(&c.history).unwrap_or_default()));
    }
    if d.subs {
        out.push(("subscriptions.json", serde_json::to_string_pretty(&c.subs).unwrap_or_default()));
    }
    if d.settings {
        out.push(("settings.json", serde_json::to_string_pretty(&c.settings).unwrap_or_default()));
    }
    out
}

fn write_saves(saves: &[Save]) {
    for (name, text) in saves {
        if !text.is_empty() {
            let _ = save_text(name, text);
        }
    }
}

/// Writes everything now. Called when the window closes.
pub fn save_all(hub: &Hub) {
    let saves = {
        let mut c = hub.lock();
        c.dirty.queue = true;
        c.dirty.history = true;
        c.dirty.subs = true;
        c.dirty.settings = true;
        take_saves(&mut c)
    };
    write_saves(&saves);
}

/// Staging folders and per-job archives from jobs that are gone.
fn cleanup_leftovers(hub: &Hub) {
    let (known, dirs): (HashSet<String>, Vec<std::path::PathBuf>) = {
        let c = hub.lock();
        let known = c.jobs.iter().map(|j| j.id.clone()).collect();
        let mut dirs: Vec<_> = c.jobs.iter().filter_map(|j| j.dest.clone()).collect();
        dirs.push(c.settings.download_dir.clone());
        dirs.sort();
        dirs.dedup();
        (known, dirs)
    };
    for d in dirs {
        let root = d.join(".snag-temp");
        if let Ok(rd) = fs::read_dir(&root) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !known.contains(&name) {
                    let _ = fs::remove_dir_all(e.path());
                }
            }
        }
        let _ = fs::remove_dir(&root);
    }
    if let Ok(rd) = fs::read_dir(settings::data_dir().join("job-archives")) {
        for e in rd.flatten() {
            let name = e.path().file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if !known.contains(&name) {
                let _ = fs::remove_file(e.path());
            }
        }
    }
}
