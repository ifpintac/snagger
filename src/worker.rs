//! Runs one download: starts yt-dlp, follows its progress, moves finished
//! files into the download folder and records how it went.
//!
//! yt-dlp works inside a private staging folder (`<downloads>\.snag-temp\<job>`)
//! so two downloads never trip over each other, and an existing file is never
//! overwritten or deleted: a name clash becomes "name (2).ext".

use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::hub::Hub;
use crate::model::{Control, Live, STOP_CANCEL, STOP_NONE, STOP_PAUSE, Stage, Status};
use crate::platform::{self, ProcGroup};
use crate::settings;
use crate::transcript;
use crate::util;
use crate::ytdlp::{self, Line};

const MEDIA_EXTS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "m4v", "avi", "flv", "ts", "3gp", "mp3", "m4a", "opus", "ogg", "oga", "flac", "wav",
    "aac", "alac", "aiff", "wma", "weba",
];

pub fn spawn(hub: Hub, id: String, ctl: Arc<Control>) {
    let direct = hub.lock().job(&id).is_some_and(|j| j.opts.direct);
    let _ = thread::Builder::new()
        .name(format!("download-{id}"))
        .spawn(move || if direct { crate::direct::run(&hub, &id, &ctl) } else { run(&hub, &id, &ctl) });
}

struct Part {
    format_id: String,
    weight: f64,
    audio: bool,
}

struct Tracker {
    n: u32,
    item: u32,
    parts: Vec<Part>,
    flatten: bool,
    subscription: bool,
    /// Files this run finished, with the page each came from.
    made: Vec<(PathBuf, String)>,
    last_repaint: Instant,
}

impl Tracker {
    fn repaint(&mut self, hub: &Hub, force: bool) {
        if force || self.last_repaint.elapsed() > Duration::from_millis(120) {
            hub.repaint();
            self.last_repaint = Instant::now();
        }
    }
}

fn run(hub: &Hub, id: &str, ctl: &Arc<Control>) {
    // Everything this run needs, read under one lock.
    let prep = {
        let mut c = hub.lock();
        let settings = c.settings.clone();
        let paths = c.engine.paths();
        match (c.job_mut(id), paths) {
            (Some(job), Some(paths)) => {
                let dest = job.dest.get_or_insert_with(|| settings.download_dir.clone()).clone();
                let staging = job.staging.get_or_insert_with(|| dest.join(".snag-temp").join(&job.id)).clone();
                let archive = if !job.opts.sub_id.is_empty() {
                    settings::subs_archive_path()
                } else if settings.use_archive {
                    settings::archive_path()
                } else {
                    settings::job_archive_path(&job.id)
                };
                job.live = Live { stage: Stage::Starting, ..Default::default() };
                job.error.clear();
                job.note.clear();
                let args = ytdlp::download_args(job, &settings, &paths, &staging, &archive);
                job.push_log(format!("Starting: {}", job.url));
                let is_sub = !job.opts.sub_id.is_empty();
                Some((args, dest, staging, archive, paths, settings.cookies_browser.clone(), is_sub))
            }
            (Some(job), None) => {
                job.status = Status::Failed;
                job.error = "The download engine isn't installed. Open Settings → Engine.".into();
                job.control = None;
                None
            }
            (None, _) => None,
        }
    };
    let Some((args, dest, staging, archive, paths, browser, is_sub)) = prep else {
        hub.wake();
        hub.repaint();
        return;
    };

    let out_dir = staging.join("out");
    let setup = fs::create_dir_all(&out_dir)
        .and_then(|_| fs::create_dir_all(staging.join("work")))
        .and_then(|_| archive.parent().map(fs::create_dir_all).unwrap_or(Ok(())));
    if let Some(temp_root) = staging.parent() {
        platform::hide_path(temp_root);
    }
    if let Err(e) = setup {
        let msg = format!("Couldn't use the download folder {} ({e}). Choose another one in Settings.", dest.display());
        return finish(hub, id, ctl, None, Vec::new(), &staging, &browser, Some(msg));
    }

    let mut cmd = ytdlp::command(&paths);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Couldn't start yt-dlp ({e}). Reinstall it from Settings → Engine.");
            return finish(hub, id, ctl, None, Vec::new(), &staging, &browser, Some(msg));
        }
    };
    let group = ProcGroup::new();
    if let Some(g) = &group {
        g.assign(&child);
    }
    *ctl.group.lock().unwrap_or_else(|p| p.into_inner()) = group;
    if ctl.stop() != STOP_NONE {
        ctl.request(ctl.stop());
    }

    // Errors and warnings arrive on stderr.
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    let stderr = child.stderr.take().expect("piped");
    let err_thread = {
        let (hub, id, errors) = (hub.clone(), id.to_string(), errors.clone());
        thread::spawn(move || {
            for line in BufReader::new(stderr).split(b'\n').map_while(Result::ok) {
                let line = String::from_utf8_lossy(&line).trim_end().to_string();
                if line.is_empty() {
                    continue;
                }
                if line.starts_with("ERROR") {
                    errors.lock().unwrap_or_else(|p| p.into_inner()).push(line.clone());
                }
                if let Some(j) = hub.lock().job_mut(&id) {
                    j.push_log(line);
                }
            }
        })
    };

    let mut t = Tracker {
        n: 1,
        item: 1,
        parts: Vec::new(),
        flatten: false,
        subscription: is_sub,
        made: Vec::new(),
        last_repaint: Instant::now(),
    };
    let stdout = child.stdout.take().expect("piped");
    for line in BufReader::new(stdout).split(b'\n').map_while(Result::ok) {
        let line = String::from_utf8_lossy(&line).trim_end().to_string();
        if line.is_empty() {
            continue;
        }
        match ytdlp::parse_line(&line) {
            Line::Item(info, fmts) => {
                on_item(hub, id, &mut t, &info, &fmts);
                if let Some(msg) = space_problem(&staging, &fmts) {
                    if let Some(j) = hub.lock().job_mut(id) {
                        j.push_log(msg.clone());
                        j.note = msg;
                    }
                    ctl.request(STOP_PAUSE);
                }
            }
            Line::Progress(p, info) => on_progress(hub, id, &mut t, &p, &info),
            Line::Post(p) => {
                if p["status"].as_str() == Some("started") {
                    let label = ytdlp::phase_label(p["postprocessor"].as_str().unwrap_or(""));
                    if let Some(j) = hub.lock().job_mut(id) {
                        j.live.stage = Stage::Processing;
                        j.live.phase = label.to_string();
                        j.live.speed = None;
                        j.live.eta = None;
                    }
                    t.repaint(hub, true);
                }
            }
            Line::File(f) => {
                let media = f["filepath"].as_str().map(PathBuf::from);
                let moved = move_out(&out_dir, &dest, media.as_deref(), t.flatten);
                if let Some(page) = f["webpage_url"].as_str() {
                    t.made.extend(moved.media.iter().map(|m| (m.clone(), page.to_string())));
                }
                let n = t.n.max(1);
                if let Some(j) = hub.lock().job_mut(id) {
                    j.items_done += 1;
                    j.progress = j.progress.max((t.item as f32 / n as f32).min(0.999));
                    apply_moved(j, moved);
                }
                t.repaint(hub, true);
            }
            Line::Other(text) => {
                if let Some(j) = hub.lock().job_mut(id) {
                    j.push_log(text);
                }
            }
        }
    }
    let status = child.wait().ok();
    let _ = err_thread.join();
    let code = status.and_then(|s| s.code());
    if matches!(code, Some(0) | Some(101)) && ctl.stop() == STOP_NONE {
        make_transcripts(hub, id, ctl, &t.made);
    }

    // Anything yt-dlp finished but didn't report (it shouldn't happen).
    let moved = move_out(&out_dir, &dest, None, t.flatten);
    if let Some(j) = hub.lock().job_mut(id) {
        apply_moved(j, moved);
    }
    let errs = errors.lock().unwrap_or_else(|p| p.into_inner()).clone();
    finish(hub, id, ctl, code, errs, &staging, &browser, None);
}

/// After the download: a transcript next to each new file, from its captions.
/// A missing transcript never fails the download; the log says why.
fn make_transcripts(hub: &Hub, id: &str, ctl: &Arc<Control>, made: &[(PathBuf, String)]) {
    let (settings, paths, clip, title) = {
        let c = hub.lock();
        let clip = c.job(id).and_then(|j| j.opts.clip);
        // File names can be shortened; one video's transcript gets its full title.
        let title = c.job(id).filter(|_| made.len() == 1).map(|j| j.display_title()).unwrap_or_default();
        (c.settings.clone(), c.engine.paths(), clip, title)
    };
    let (true, Some(paths)) = (settings.transcripts, paths) else { return };
    for (i, (media, url)) in made.iter().enumerate() {
        if ctl.stop() != STOP_NONE {
            return;
        }
        if let Some(j) = hub.lock().job_mut(id) {
            j.live.stage = Stage::Processing;
            j.live.speed = None;
            j.live.eta = None;
            j.live.phase = if made.len() > 1 {
                format!("Getting transcripts ({} of {})", i + 1, made.len())
            } else {
                "Getting the transcript".into()
            };
        }
        hub.repaint();
        let req = transcript::Request { url, title: &title, media, clip: if made.len() == 1 { clip } else { None } };
        let outcome = transcript::from_captions(&req, &settings, &paths, ctl);
        if let Some(j) = hub.lock().job_mut(id) {
            match outcome {
                Ok(Some(path)) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    j.push_log(format!("Transcript saved: {name}"));
                    j.extras.push(path);
                }
                Ok(None) => j.push_log("No transcript: this video has no captions.".into()),
                Err(e) => j.push_log(format!("No transcript: {e}")),
            }
        }
    }
}

/// Before a video downloads: does it fit? Only when yt-dlp knows the sizes.
fn space_problem(staging: &Path, fmts: &[Value]) -> Option<String> {
    let sizes: Option<Vec<f64>> =
        fmts.iter().map(|f| f["filesize"].as_f64().or(f["filesize_approx"].as_f64())).collect();
    let total = sizes.filter(|s| !s.is_empty())?.iter().sum::<f64>() as u64;
    let needed = util::space_needed(total, fmts.len() > 1);
    let free = platform::free_space(staging)?;
    (free < needed).then(|| util::not_enough_space(needed, free, staging))
}

fn on_item(hub: &Hub, id: &str, t: &mut Tracker, info: &Value, fmts: &[Value]) {
    let n_entries = info["n_entries"].as_u64().map(|n| n as u32);
    t.n = n_entries.unwrap_or(1).max(1);
    t.item = info["playlist_autonumber"].as_u64().map(|n| n as u32).unwrap_or(1).max(1);
    // A "playlist" of one (a post with a single video, or one picked item)
    // is saved like a single video, without a folder. Subscriptions save
    // straight into their own folder, so there's nothing to flatten.
    t.flatten = n_entries == Some(1) && !t.subscription;

    let size = |f: &Value| f["filesize"].as_f64().or(f["filesize_approx"].as_f64());
    let sizes: Vec<Option<f64>> = fmts.iter().map(size).collect();
    let audio: Vec<bool> = fmts.iter().map(|f| f["vcodec"].as_str() == Some("none")).collect();
    let total: f64 = sizes.iter().flatten().sum();
    let weights: Vec<f64> = if !fmts.is_empty() && sizes.iter().all(Option::is_some) && total > 0.0 {
        sizes.iter().map(|s| s.unwrap_or(0.0) / total).collect()
    } else if fmts.len() == 2 {
        // Unknown sizes: video is usually most of the bytes.
        audio.iter().map(|&a| if a { 0.15 } else { 0.85 }).collect()
    } else {
        vec![1.0 / fmts.len().max(1) as f64; fmts.len()]
    };
    t.parts = fmts
        .iter()
        .zip(weights)
        .zip(audio)
        .map(|((f, weight), audio)| Part {
            format_id: f["format_id"].as_str().unwrap_or("").to_string(),
            weight,
            audio,
        })
        .collect();

    let s = |k: &str| info[k].as_str().unwrap_or("").to_string();
    let title = s("title");
    {
        let mut c = hub.lock();
        let Some(j) = c.job_mut(id) else { return };
        let playlist_title = s("playlist_title");
        if t.subscription {
            // Keeps the subscription's name.
        } else if t.n > 1 {
            if j.title.is_empty() {
                j.title = if playlist_title.is_empty() { title.clone() } else { playlist_title };
            }
        } else if j.title.is_empty() || j.n_items <= 1 {
            j.title = title.clone();
        }
        if j.thumb.is_empty() {
            j.thumb = s("thumbnail");
        }
        let extractor = s("extractor_key");
        if !extractor.is_empty() {
            j.site = util::site_name(&extractor, &s("webpage_url"));
        }
        if j.uploader.is_empty() {
            j.uploader = if s("uploader").is_empty() { s("channel") } else { s("uploader") };
        }
        if t.n <= 1 {
            j.duration = info["duration"].as_f64().unwrap_or(0.0) as u64;
            j.vid = format!("{}:{}", extractor.to_ascii_lowercase(), s("id"));
        }
        j.n_items = j.n_items.max(t.n);
        j.live.item = t.item;
        j.live.item_title = title.clone();
        j.live.stage = Stage::Downloading;
        j.live.phase =
            if info["is_live"].as_bool() == Some(true) { "Recording live stream".into() } else { String::new() };
        if t.n > 1 {
            j.push_log(format!("Item {} of {}: {title}", t.item, t.n));
        } else {
            j.push_log(format!("Downloading: {title}"));
        }
    }
    t.repaint(hub, true);
}

fn on_progress(hub: &Hub, id: &str, t: &mut Tracker, p: &Value, info: &Value) {
    let status = p["status"].as_str().unwrap_or("");
    let got = p["downloaded_bytes"].as_f64().unwrap_or(0.0);
    let size = p["total_bytes"].as_f64().or(p["total_bytes_estimate"].as_f64());
    let frac = if status == "finished" {
        1.0
    } else if let Some(sz) = size.filter(|s| *s > 0.0) {
        got / sz
    } else if let (Some(i), Some(n)) = (p["fragment_index"].as_f64(), p["fragment_count"].as_f64()) {
        if n > 0.0 { i / n } else { 0.0 }
    } else {
        0.0
    }
    .clamp(0.0, 1.0);

    let fid = info["format_id"].as_str().unwrap_or("");
    let idx = t.parts.iter().position(|part| part.format_id == fid).unwrap_or(0);
    let item_frac = if t.parts.is_empty() {
        frac
    } else {
        let before: f64 = t.parts[..idx].iter().map(|p| p.weight).sum();
        before + t.parts[idx].weight * frac
    };
    if let Some(auto) = info["playlist_autonumber"].as_u64() {
        t.item = auto as u32;
    }
    let overall = (((t.item.max(1) - 1) as f64 + item_frac) / t.n.max(1) as f64) as f32;
    let audio_part = t.parts.len() > 1 && t.parts.get(idx).is_some_and(|p| p.audio);

    if let Some(j) = hub.lock().job_mut(id) {
        j.progress = j.progress.max(overall.min(0.999));
        let l = &mut j.live;
        if l.stage != Stage::Processing || status == "downloading" {
            l.stage = Stage::Downloading;
        }
        l.speed = p["speed"].as_f64();
        l.eta = p["eta"].as_f64().map(|e| e as u64);
        l.downloaded = got as u64;
        l.total = size.map(|s| s as u64);
        l.item = t.item;
        if l.phase != "Recording live stream" {
            l.phase = if audio_part { "Downloading audio".into() } else { String::new() };
        }
    }
    t.repaint(hub, false);
}

#[derive(Default)]
struct Moved {
    media: Vec<PathBuf>,
    extras: Vec<PathBuf>,
    bytes: u64,
    folder: Option<PathBuf>,
}

fn apply_moved(j: &mut crate::model::Job, m: Moved) {
    if m.media.is_empty() && m.extras.is_empty() {
        return;
    }
    j.files.extend(m.media);
    j.extras.extend(m.extras);
    j.bytes += m.bytes;
    if m.folder.is_some() && j.n_items > 1 {
        j.folder = m.folder;
    }
}

/// Moves everything yt-dlp has finished from the staging folder into the
/// download folder, keeping sub-folders (playlist, site, uploader).
fn move_out(out: &Path, dest: &Path, media: Option<&Path>, flatten: bool) -> Moved {
    let mut moved = Moved::default();
    let mut files = Vec::new();
    collect_files(out, &mut files);
    let media_str = media.map(|m| m.to_string_lossy().to_ascii_lowercase());
    for file in files {
        let Ok(rel) = file.strip_prefix(out) else { continue };
        let mut rel = rel.to_path_buf();
        if flatten
            && let Some(name) = rel.file_name().map(|n| n.to_string_lossy().into_owned())
            && let Some(stripped) = strip_number_prefix(&name)
        {
            let parent = rel.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default();
            rel = parent.join(stripped);
        }
        let target = util::unique_path(&dest.join(&rel));
        if let Some(parent) = target.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if !move_file(&file, &target) {
            continue;
        }
        let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        let is_media = match &media_str {
            Some(m) => file.to_string_lossy().to_ascii_lowercase() == *m,
            None => target
                .extension()
                .map(|e| MEDIA_EXTS.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
                .unwrap_or(false),
        };
        if is_media {
            moved.media.push(target.clone());
            moved.bytes += size;
            if let Some(parent) = target.parent().filter(|p| *p != dest) {
                moved.folder = Some(parent.to_path_buf());
            }
        } else {
            moved.extras.push(target);
        }
    }
    remove_empty_dirs(out);
    moved
}

/// "001 - Title.mp4" → "Title.mp4"
fn strip_number_prefix(name: &str) -> Option<String> {
    let digits = name.chars().take_while(char::is_ascii_digit).count();
    if digits >= 3 { name[digits..].strip_prefix(" - ").map(str::to_string) } else { None }
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn remove_empty_dirs(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            remove_empty_dirs(&p);
            let _ = fs::remove_dir(&p);
        }
    }
}

/// Rename, retried briefly (antivirus scanners hold new files for a moment),
/// with copy-and-delete as the last resort.
fn move_file(from: &Path, to: &Path) -> bool {
    for _ in 0..15 {
        if fs::rename(from, to).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(200));
    }
    if fs::copy(from, to).is_ok() {
        let _ = fs::remove_file(from);
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn finish(
    hub: &Hub,
    id: &str,
    ctl: &Arc<Control>,
    code: Option<i32>,
    errors: Vec<String>,
    staging: &Path,
    browser: &str,
    setup_error: Option<String>,
) {
    let stop = ctl.stop();
    let mut drop_staging = false;
    {
        let mut c = hub.lock();
        let auto_retry = c.settings.auto_retry;
        let mut done_job = None;
        let mut sub_update: Option<(String, Result<u32, String>)> = None;
        match c.job_mut(id) {
            // Removed from the list while it ran.
            None => {
                drop_staging = true;
                let _ = fs::remove_file(settings::job_archive_path(id));
            }
            Some(j) => {
                j.control = None;
                j.live = Live::default();
                let unique: BTreeSet<&String> = errors.iter().collect();
                let downloaded = matches!(code, Some(0) | Some(101));
                if stop == STOP_PAUSE && !downloaded {
                    j.status = Status::Paused;
                    j.push_log("Paused.".into());
                } else if stop == STOP_CANCEL && !downloaded {
                    j.status = Status::Cancelled;
                    j.finished = util::now();
                    j.push_log("Cancelled.".into());
                    drop_staging = true;
                } else if let Some(msg) = setup_error {
                    j.status = Status::Failed;
                    j.error = msg;
                    j.finished = util::now();
                } else if matches!(code, Some(0) | Some(101)) || !j.files.is_empty() {
                    j.status = Status::Done;
                    j.progress = 1.0;
                    j.finished = util::now();
                    j.items_failed = if j.n_items > 1 { unique.len() as u32 } else { 0 };
                    if j.files.is_empty() {
                        j.note = if j.opts.sub_id.is_empty() {
                            "Already downloaded before (skipped by the download archive)".into()
                        } else {
                            "No new videos".into()
                        };
                    }
                    if j.items_failed > 0
                        && let Some(first) = unique.iter().next()
                    {
                        j.error = util::friendly_error(first, browser);
                    }
                    drop_staging = true;
                    done_job = Some(j.clone());
                    if !j.opts.sub_id.is_empty() {
                        sub_update = Some((j.opts.sub_id.clone(), Ok(j.items_done)));
                    }
                } else {
                    let error = match errors.last() {
                        Some(e) => util::friendly_error(e, browser),
                        None => format!(
                            "yt-dlp stopped unexpectedly (exit code {}).",
                            code.map_or("none".into(), |c| c.to_string())
                        ),
                    };
                    crate::hub::fail_or_retry(j, error, auto_retry);
                    if j.status == Status::Failed && !j.opts.sub_id.is_empty() {
                        sub_update = Some((j.opts.sub_id.clone(), Err(j.error.clone())));
                    }
                }
                c.dirty.queue = true;
            }
        }
        if let Some(job) = done_job {
            c.record_history(&job);
        }
        if let Some((sub_id, result)) = sub_update
            && let Some(s) = c.subs.iter_mut().find(|s| s.id == sub_id)
        {
            s.last_checked = util::now();
            match result {
                Ok(n) => {
                    s.last_new = n;
                    s.total_new += n;
                    s.error.clear();
                }
                Err(e) => s.error = e,
            }
            c.dirty.subs = true;
        }
    }
    if drop_staging {
        let _ = fs::remove_dir_all(staging);
        if let Some(root) = staging.parent() {
            let _ = fs::remove_dir(root); // only when empty
        }
    }
    hub.wake();
    hub.repaint();
}
