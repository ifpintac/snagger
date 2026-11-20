//! The state shared by the window, the queue manager and the download
//! threads, and the operations the UI performs on it.

use std::collections::HashSet;
use std::fs;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use crate::engine::EngineState;
use crate::model::{Control, HistoryItem, Job, JobOptions, STOP_CANCEL, STOP_PAUSE, Status, Subscription};
use crate::settings::{self, Settings, load_json};
use crate::util;

#[derive(Default, Clone, Copy)]
pub struct Dirty {
    pub queue: bool,
    pub history: bool,
    pub subs: bool,
    pub settings: bool,
}

/// What to do once the queue runs dry. Picked per session, used once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WhenDone {
    #[default]
    Nothing,
    OpenFolder,
    Sleep,
    ShutDown,
}

impl WhenDone {
    pub const ALL: [WhenDone; 4] = [WhenDone::Nothing, WhenDone::OpenFolder, WhenDone::Sleep, WhenDone::ShutDown];

    pub fn label(self) -> &'static str {
        match self {
            WhenDone::Nothing => "Do nothing",
            WhenDone::OpenFolder => "Open the download folder",
            WhenDone::Sleep => "Put the PC to sleep",
            WhenDone::ShutDown => "Shut down the PC",
        }
    }

    pub fn is_power(self) -> bool {
        matches!(self, WhenDone::Sleep | WhenDone::ShutDown)
    }
}

/// Seconds of warning before Snag sleeps or shuts down the PC.
pub const POWER_COUNTDOWN: u64 = 60;

pub struct Core {
    pub settings: Settings,
    pub jobs: Vec<Job>,
    pub history: Vec<HistoryItem>,
    pub subs: Vec<Subscription>,
    pub engine: EngineState,
    pub queue_paused: bool,
    /// Links handed over by a second launch of Snag (command line, "Open with").
    pub inbox: Vec<String>,
    /// Links noticed on the clipboard, waiting for the user to decide.
    pub clip_offer: Vec<String>,
    /// Text Snag itself just copied, so the watcher ignores it.
    pub clip_ignore: String,
    /// Links the window already took (pasted, previewed), so the clipboard
    /// watcher doesn't offer them again.
    pub handled: Vec<String>,
    pub toasts: Vec<String>,
    pub dirty: Dirty,
    pub when_done: WhenDone,
    /// A sleep or shut-down that fires at this time unless cancelled.
    pub power_at: Option<(WhenDone, u64)>,
    /// "Start now" pressed while outside the download hours.
    pub window_override: bool,
    /// Free space on the download folder's drive, refreshed by the queue.
    pub disk_free: Option<u64>,
    /// Waiting downloads are held because the drive is nearly full.
    pub disk_hold: bool,
}

impl Core {
    fn new(settings: Settings, jobs: Vec<Job>, history: Vec<HistoryItem>, subs: Vec<Subscription>) -> Core {
        Core {
            settings,
            jobs,
            history,
            subs,
            engine: EngineState::default(),
            queue_paused: false,
            inbox: Vec::new(),
            clip_offer: Vec::new(),
            clip_ignore: String::new(),
            handled: Vec::new(),
            toasts: Vec::new(),
            dirty: Dirty::default(),
            when_done: WhenDone::Nothing,
            power_at: None,
            window_override: false,
            disk_free: None,
            disk_hold: false,
        }
    }

    fn load() -> Core {
        let settings: Settings = load_json::<Option<Settings>>("settings.json").unwrap_or_default().sanitized();
        let mut jobs: Vec<Job> = load_json("queue.json");
        for j in &mut jobs {
            // Downloads that were running when Snag closed pick up where they left off.
            if j.status == Status::Running {
                j.status = if settings.resume_on_start { Status::Queued } else { Status::Paused };
            } else if j.status == Status::Queued && !settings.resume_on_start {
                j.status = Status::Paused;
            }
            if !j.status.is_finished() {
                j.progress = j.progress.min(0.999);
            }
        }
        Core::new(settings, jobs, load_json("history.json"), load_json("subscriptions.json"))
    }

    pub fn job(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn job_mut(&mut self, id: &str) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push(msg.into());
    }

    // -- adding --------------------------------------------------------------

    pub fn add_job(&mut self, job: Job) -> String {
        let id = job.id.clone();
        self.jobs.push(job);
        self.dirty.queue = true;
        id
    }

    /// Adds many links with one set of options. Skips links that are already
    /// waiting or downloading. Returns (added, skipped).
    pub fn add_links(&mut self, links: &[String], opts: &JobOptions) -> (usize, usize) {
        let pending: HashSet<String> = self
            .jobs
            .iter()
            .filter(|j| j.status.is_active() || j.status == Status::Paused)
            .map(|j| j.url.clone())
            .collect();
        let mut added = 0;
        let mut skipped = 0;
        for url in links {
            if pending.contains(url) {
                skipped += 1;
                continue;
            }
            self.add_job(Job::new(url, opts.clone()));
            added += 1;
        }
        (added, skipped)
    }

    pub fn is_queued(&self, url: &str) -> bool {
        self.jobs.iter().any(|j| j.url == url && (j.status.is_active() || j.status == Status::Paused))
    }

    // -- controlling jobs ----------------------------------------------------

    pub fn pause(&mut self, id: &str) {
        if let Some(j) = self.job_mut(id) {
            match j.status {
                Status::Running => {
                    if let Some(c) = &j.control {
                        c.request(STOP_PAUSE);
                    }
                }
                Status::Queued => {
                    // A pending automatic retry is called off with it.
                    j.status = Status::Paused;
                    j.note.clear();
                    j.retry_at = 0;
                }
                _ => {}
            }
            self.dirty.queue = true;
        }
    }

    pub fn resume(&mut self, id: &str) {
        if let Some(j) = self.job_mut(id)
            && matches!(j.status, Status::Paused | Status::Failed | Status::Cancelled)
        {
            if j.status == Status::Cancelled {
                j.progress = 0.0;
                j.items_done = 0;
                j.items_failed = 0;
            }
            j.requeue();
            self.dirty.queue = true;
        }
    }

    /// Downloads again from scratch (keeps the same row).
    pub fn redo(&mut self, id: &str) {
        if let Some(j) = self.job_mut(id)
            && j.status.is_finished()
        {
            let url = j.url.clone();
            let opts = j.opts.clone();
            let mut fresh = Job::new(&url, opts);
            fresh.title = j.title.clone();
            fresh.thumb = j.thumb.clone();
            fresh.site = j.site.clone();
            *j = fresh;
            self.dirty.queue = true;
        }
    }

    pub fn cancel(&mut self, id: &str) {
        if let Some(j) = self.job_mut(id) {
            match j.status {
                Status::Running => {
                    if let Some(c) = &j.control {
                        c.request(STOP_CANCEL);
                    }
                }
                Status::Queued | Status::Paused => {
                    j.status = Status::Cancelled;
                    j.live = Default::default();
                    j.finished = util::now();
                    if let Some(s) = j.staging.take() {
                        let _ = fs::remove_dir_all(s);
                    }
                }
                _ => {}
            }
            self.dirty.queue = true;
        }
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(pos) = self.jobs.iter().position(|j| j.id == id) {
            let j = self.jobs.remove(pos);
            if j.status == Status::Running {
                // The download thread cleans up its own folder when it ends.
                if let Some(c) = &j.control {
                    c.request(STOP_CANCEL);
                }
            } else {
                discard_job_files(&j);
            }
            self.dirty.queue = true;
        }
    }

    pub fn pause_all(&mut self) {
        self.queue_paused = true;
        let ids: Vec<String> = self.jobs.iter().filter(|j| j.status.is_active()).map(|j| j.id.clone()).collect();
        for id in ids {
            self.pause(&id);
        }
    }

    pub fn resume_all(&mut self) {
        self.queue_paused = false;
        for j in self.jobs.iter_mut().filter(|j| j.status == Status::Paused) {
            j.requeue();
        }
        self.dirty.queue = true;
    }

    pub fn retry_failed(&mut self) -> usize {
        let mut n = 0;
        for j in self.jobs.iter_mut() {
            let partial_playlist = j.status == Status::Done && j.items_failed > 0;
            if j.status == Status::Failed || partial_playlist {
                j.requeue();
                j.items_failed = 0;
                n += 1;
            }
        }
        self.dirty.queue = true;
        n
    }

    pub fn clear_finished(&mut self) {
        let (keep, drop): (Vec<Job>, Vec<Job>) = std::mem::take(&mut self.jobs)
            .into_iter()
            .partition(|j| !(j.status == Status::Done || j.status == Status::Cancelled));
        for j in &drop {
            discard_job_files(j);
        }
        self.jobs = keep;
        self.dirty.queue = true;
    }

    pub fn clear_failed(&mut self) {
        let (keep, drop): (Vec<Job>, Vec<Job>) =
            std::mem::take(&mut self.jobs).into_iter().partition(|j| j.status != Status::Failed);
        for j in &drop {
            discard_job_files(j);
        }
        self.jobs = keep;
        self.dirty.queue = true;
    }

    /// Moves a waiting job to the front of the queue.
    pub fn move_to_top(&mut self, id: &str) {
        if let Some(pos) = self.jobs.iter().position(|j| j.id == id) {
            let j = self.jobs.remove(pos);
            let first_waiting = self.jobs.iter().position(|j| j.status == Status::Queued).unwrap_or(0);
            self.jobs.insert(first_waiting, j);
            self.dirty.queue = true;
        }
    }

    pub fn move_to_bottom(&mut self, id: &str) {
        if let Some(pos) = self.jobs.iter().position(|j| j.id == id) {
            let j = self.jobs.remove(pos);
            self.jobs.push(j);
            self.dirty.queue = true;
        }
    }

    // -- subscriptions -------------------------------------------------------

    /// Queues a download of whatever is new in a subscription.
    pub fn check_subscription(&mut self, sub_id: &str) -> bool {
        let Some(s) = self.subs.iter().find(|s| s.id == sub_id).cloned() else { return false };
        let busy =
            self.jobs.iter().any(|j| j.opts.sub_id == s.id && (j.status.is_active() || j.status == Status::Paused));
        if busy {
            return false;
        }
        let opts = JobOptions {
            preset: s.preset,
            folder: s.folder.clone(),
            sub_id: s.id.clone(),
            break_on_existing: util::looks_like_channel(&s.url),
            ..Default::default()
        };
        let mut job = Job::new(&s.url, opts);
        job.title = s.title.clone();
        self.add_job(job);
        true
    }

    pub fn remove_subscription(&mut self, sub_id: &str) {
        self.subs.retain(|s| s.id != sub_id);
        self.dirty.subs = true;
    }

    // -- history -------------------------------------------------------------

    pub fn record_history(&mut self, job: &Job) {
        if job.files.is_empty() {
            return;
        }
        self.history.insert(
            0,
            HistoryItem {
                id: util::new_id(),
                url: job.url.clone(),
                vid: job.vid.clone(),
                title: job.display_title(),
                site: job.site.clone(),
                uploader: job.uploader.clone(),
                thumb: job.thumb.clone(),
                preset: job.opts.preset,
                files: job.files.clone(),
                folder: job.folder.clone(),
                bytes: job.bytes,
                items: job.items_done,
                finished: util::now(),
                opts: job.opts.clone(),
            },
        );
        self.history.truncate(5000);
        self.dirty.history = true;
    }

    /// A previous download of the same video whose file still exists.
    pub fn already_downloaded(&self, vid: &str, url: &str) -> Option<&HistoryItem> {
        self.history
            .iter()
            .find(|h| ((!vid.is_empty() && h.vid == vid) || h.url == url) && h.files.iter().any(|f| f.exists()))
    }
}

/// A download that went wrong: back to the queue for another try later when
/// it looks like a passing problem (a dropped connection, a busy server, too
/// many requests), otherwise marked failed.
pub fn fail_or_retry(j: &mut Job, error: String, auto_retry: bool) {
    // A full disk isn't a failure of the download: keep the part and pause.
    if util::disk_full(&error) {
        j.status = Status::Paused;
        j.push_log(error.clone());
        j.note = error;
        j.error.clear();
        j.retry_at = 0;
        return;
    }
    // "403 Forbidden" is usually one stream link gone bad; a new run gets
    // fresh links. Quick, so it doesn't wait for the auto-retry setting.
    if util::refused(&error)
        && let Some(secs) = util::refusal_delay(j.refusals)
    {
        j.refusals += 1;
        j.status = Status::Queued;
        j.retry_at = util::now() + secs;
        j.push_log(format!("The site refused the download (HTTP 403). Fetching fresh links in {secs} s."));
        j.note = "The site refused the download: fetching fresh links".into();
        j.error.clear();
        return;
    }
    let delay = if auto_retry { util::retry_delay(&error, j.auto_retries) } else { None };
    match delay {
        Some(secs) => {
            j.auto_retries += 1;
            j.status = Status::Queued;
            j.retry_at = util::now() + secs;
            j.push_log(format!("Trying again in {secs} s: {error}"));
            j.note = error;
            j.error.clear();
        }
        None => {
            j.status = Status::Failed;
            j.finished = util::now();
            j.error = error;
            j.retry_at = 0;
        }
    }
}

/// Removes a job's partial data and its per-job archive.
pub fn discard_job_files(j: &Job) {
    if let Some(s) = &j.staging
        && j.status != Status::Done
    {
        let _ = fs::remove_dir_all(s);
    }
    let _ = fs::remove_file(settings::job_archive_path(&j.id));
}

// --------------------------------------------------------------------------
// The handle everything shares
// --------------------------------------------------------------------------

struct Inner {
    core: Mutex<Core>,
    wake: Mutex<Option<Sender<()>>>,
    ctx: OnceLock<eframe::egui::Context>,
}

#[derive(Clone)]
pub struct Hub(Arc<Inner>);

impl Hub {
    pub fn load() -> Hub {
        Hub(Arc::new(Inner { core: Mutex::new(Core::load()), wake: Mutex::new(None), ctx: OnceLock::new() }))
    }

    /// A hub that reads and writes nothing, for tests.
    #[cfg(test)]
    pub fn load_empty() -> Hub {
        let core = Core::new(Settings::default(), Vec::new(), Vec::new(), Vec::new());
        Hub(Arc::new(Inner { core: Mutex::new(core), wake: Mutex::new(None), ctx: OnceLock::new() }))
    }

    pub fn lock(&self) -> MutexGuard<'_, Core> {
        self.0.core.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_waker(&self, tx: Sender<()>) {
        *self.0.wake.lock().unwrap_or_else(|p| p.into_inner()) = Some(tx);
    }

    /// Nudges the queue manager to look for work now.
    pub fn wake(&self) {
        if let Some(tx) = self.0.wake.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            let _ = tx.send(());
        }
    }

    pub fn set_ctx(&self, ctx: eframe::egui::Context) {
        let _ = self.0.ctx.set(ctx);
    }

    pub fn repaint(&self) {
        if let Some(ctx) = self.0.ctx.get() {
            ctx.request_repaint();
        }
    }

    pub fn new_control() -> Arc<Control> {
        Arc::new(Control::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_retried_twice_with_fresh_links_then_fails() {
        let mut j = Job::new("https://www.youtube.com/watch?v=x", Default::default());
        j.status = Status::Running;
        for expected in [1, 2] {
            fail_or_retry(&mut j, util::REFUSED.into(), false);
            assert_eq!(j.status, Status::Queued, "even with auto-retry off");
            assert_eq!(j.refusals, expected);
            assert!(j.retry_at > 0 && j.error.is_empty());
            j.status = Status::Running;
        }
        fail_or_retry(&mut j, util::REFUSED.into(), false);
        assert_eq!(j.status, Status::Failed);
        assert_eq!(j.error, util::REFUSED);
        j.requeue();
        assert_eq!(j.refusals, 0, "a manual retry gets fresh tries again");
    }

    #[test]
    fn a_full_disk_pauses_and_keeps_the_part() {
        let mut j = Job::new("https://example.com/v", Default::default());
        j.status = Status::Running;
        let msg = util::friendly_error("ERROR: [Errno 28] No space left on device", "");
        fail_or_retry(&mut j, msg.clone(), true);
        assert_eq!(j.status, Status::Paused);
        assert_eq!(j.note, msg);
        assert!(j.error.is_empty());
    }
}
