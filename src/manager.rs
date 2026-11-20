//! The queue manager. Runs on its own thread so downloads keep flowing while
//! the window is minimised: starts waiting jobs when a slot frees up, saves
//! state, keeps the PC awake, drives the taskbar progress and runs scheduled
//! subscription checks.

use std::collections::HashSet;
use std::fs;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::hub::{Core, Hub, POWER_COUNTDOWN, WhenDone};
use crate::model::{Live, Stage, Status};
use crate::platform::{self, Bar, Taskbar};
use crate::settings::{self, save_text};
use crate::util;
use crate::worker;

/// Below this much free space, waiting downloads don't start.
pub const DISK_FLOOR: u64 = 256 * 1024 * 1024;

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
        let mut notify: Option<String> = None;
        let mut open_folder = None;
        let mut power = None;
        {
            let mut c = hub.lock();
            let now = util::now();
            let engine_ok = c.engine.ready && c.engine.busy.is_none() && c.engine.ytdlp.is_some();
            let running = c.jobs.iter().filter(|j| j.status == Status::Running).count();
            let in_window = c.window_override || c.settings.in_window(settings::local_minute());
            c.disk_free = platform::free_space(&c.settings.download_dir);
            let startable = c.jobs.iter().any(|j| j.status == Status::Queued && !j.retry_pending(now));
            // Starting more downloads on a nearly full drive only makes them all fail.
            c.disk_hold = startable && c.disk_free.is_some_and(|f| f < DISK_FLOOR);
            if engine_ok && !c.queue_paused && in_window && !c.disk_hold {
                let mut free = (c.settings.parallel as usize).saturating_sub(running);
                for j in c.jobs.iter_mut() {
                    if free == 0 {
                        break;
                    }
                    if j.status == Status::Queued && !j.retry_pending(now) {
                        j.status = Status::Running;
                        j.retry_at = 0;
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

            // One notification (and the "when done" action) when a batch finishes.
            if !active.is_empty() && !self.was_active {
                self.batch_started = now;
            }
            let batch_over = self.was_active && active.is_empty();
            if batch_over && c.settings.notify_done {
                let done: Vec<&crate::model::Job> =
                    c.jobs.iter().filter(|j| j.status == Status::Done && j.finished >= self.batch_started).collect();
                let failed =
                    c.jobs.iter().filter(|j| j.status == Status::Failed && j.finished >= self.batch_started).count();
                if !done.is_empty() {
                    let body = match (done.len(), failed) {
                        (1, 0) => format!("“{}” finished.", util::ellipsize(&done[0].display_title(), 80)),
                        (n, 0) => format!("{n} downloads finished."),
                        (n, f) => format!("{n} finished, {f} failed."),
                    };
                    notify = Some(body);
                }
            }
            self.was_active = !active.is_empty();
            if batch_over && !c.queue_paused {
                match std::mem::take(&mut c.when_done) {
                    WhenDone::Nothing => {}
                    WhenDone::OpenFolder => open_folder = Some(c.settings.download_dir.clone()),
                    wd => {
                        c.power_at = Some((wd, now + POWER_COUNTDOWN));
                        c.toast(format!("All done. {} in a minute unless you cancel.", wd.label()));
                    }
                }
            }
            if self.was_active && c.power_at.is_some() {
                // Something new started downloading: forget the sleep/shut-down.
                c.power_at = None;
                c.toast("New downloads started, so Snagger won't sleep or shut down the PC.");
            }
            if let Some((wd, at)) = c.power_at
                && now >= at
            {
                c.power_at = None;
                power = Some(wd);
            }

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
        if let Some(body) = notify
            && !crate::background::notify("Downloads finished", &body)
        {
            platform::notify_attention(true);
        }
        if let Some(dir) = open_folder {
            let _ = std::fs::create_dir_all(&dir);
            platform::open_path(&dir);
        }
        if let Some(wd) = power {
            save_all(hub);
            match wd {
                WhenDone::Sleep => platform::sleep_pc(),
                WhenDone::ShutDown => platform::shut_down(),
                _ => {}
            }
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
