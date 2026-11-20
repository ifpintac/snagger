//! Snag's own downloader for plain file links: a .zip, a .pdf, an .mp4 or a
//! .psd sitting on a server. Pure Rust, no yt-dlp.
//!
//! Files whose server allows byte ranges are cut into chunks that several
//! connections fetch at once; whichever connection is free takes the next
//! chunk, so a slow one never holds the rest up. Finished chunks are recorded
//! in `state.json`, so pausing, quitting or a dropped connection costs at most
//! the chunks that were in flight. Every chunk request has its own time limit,
//! so a stalled connection is retried instead of hanging forever.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use ureq::ResponseExt;

use crate::hub::Hub;
use crate::model::{Control, Live, STOP_CANCEL, STOP_NONE, STOP_PAUSE, Stage, Status};
use crate::util;

const UA: &str = concat!("Snag/", env!("CARGO_PKG_VERSION"));
const MAX_CHUNK: u64 = 8 * 1024 * 1024;
const MIN_CHUNK: u64 = 256 * 1024;
/// Below this, one connection is as fast as several.
const SPLIT_ABOVE: u64 = 2 * 1024 * 1024;

/// What a file link points at, from one tiny ranged request.
#[derive(Clone, Debug)]
pub struct Probe {
    /// Where the link ends up after redirects.
    pub final_url: String,
    pub name: String,
    pub size: Option<u64>,
    pub ranges: bool,
    pub mime: String,
}

impl Probe {
    /// HTML means a web page, not a file.
    pub fn is_page(&self) -> bool {
        let m = self.mime.to_ascii_lowercase();
        m.starts_with("text/html") || m.starts_with("application/xhtml")
    }
}

fn agent(body_timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .timeout_recv_body(body_timeout)
        .http_status_as_error(false)
        .user_agent(UA)
        .build()
        .into()
}

pub fn inspect(url: &str) -> Result<Probe, String> {
    let resp = agent(Some(Duration::from_secs(15)))
        .get(&util::encode_url(url))
        .header("Range", "bytes=0-0")
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(net_err)?;
    let status = resp.status().as_u16();
    if status >= 400 {
        return Err(match status {
            401 | 403 => "The server refused the download (it may need you to be signed in).".into(),
            404 | 410 => "That file isn't there any more (404).".into(),
            _ => format!("The server answered HTTP {status}."),
        });
    }
    let final_url = resp.get_uri().to_string();
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mime = header("content-type");
    let (ranges, size) = if status == 206 {
        (true, header("content-range").rsplit('/').next().and_then(|t| t.trim().parse().ok()))
    } else {
        (false, header("content-length").trim().parse().ok())
    };
    let name = file_name(&header("content-disposition"), &final_url, &mime);
    Ok(Probe { final_url, name, size, ranges, mime })
}

/// The file's name: from Content-Disposition, else the link's last path part.
fn file_name(disposition: &str, url: &str, mime: &str) -> String {
    let from_header = disposition.split(';').map(str::trim).find_map(|part| {
        if let Some(v) = part.strip_prefix("filename*=") {
            let v = v.trim_matches('"');
            let v = v.split_once("''").map(|(_, rest)| rest).unwrap_or(v);
            Some(util::percent_decode(v))
        } else {
            part.strip_prefix("filename=").map(|v| v.trim_matches('"').to_string())
        }
    });
    let from_url = || {
        let path = url.split(['?', '#']).next().unwrap_or("");
        let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        let decoded = util::percent_decode(last);
        (!decoded.is_empty() && !decoded.contains("://")).then_some(decoded)
    };
    let mut name = from_header.filter(|n| !n.trim().is_empty()).or_else(from_url).unwrap_or_else(|| "download".into());
    name = util::safe_name(&name, 180);
    if Path::new(&name).extension().is_none()
        && let Some(ext) = ext_for_mime(mime)
    {
        name.push('.');
        name.push_str(ext);
    }
    name
}

fn ext_for_mime(mime: &str) -> Option<&'static str> {
    let m = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    Some(match m.as_str() {
        "application/pdf" => "pdf",
        "application/zip" | "application/x-zip-compressed" => "zip",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "audio/mpeg" => "mp3",
        "audio/mp4" => "m4a",
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "application/x-7z-compressed" => "7z",
        "application/vnd.rar" | "application/x-rar-compressed" => "rar",
        _ => return None,
    })
}

// --------------------------------------------------------------------------
// Running a download
// --------------------------------------------------------------------------

/// What's been fetched so far, saved next to the partial file.
#[derive(Serialize, Deserialize, Default)]
struct State {
    url: String,
    name: String,
    size: Option<u64>,
    ranges: bool,
    chunk: u64,
    done: Vec<bool>,
}

enum Fail {
    Stopped,
    Error(String),
}

/// A shared byte budget per second (0 = no limit).
struct Limiter {
    rate: f64,
    state: Mutex<(Instant, f64)>,
}

impl Limiter {
    fn new(bytes_per_sec: u64) -> Limiter {
        Limiter { rate: bytes_per_sec as f64, state: Mutex::new((Instant::now(), bytes_per_sec as f64)) }
    }

    fn take(&self, n: usize) {
        if self.rate <= 0.0 {
            return;
        }
        let wait = {
            let mut s = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            s.1 = (s.1 + now.duration_since(s.0).as_secs_f64() * self.rate).min(self.rate);
            s.0 = now;
            s.1 -= n as f64;
            if s.1 < 0.0 { -s.1 / self.rate } else { 0.0 }
        };
        if wait > 0.0 {
            thread::sleep(Duration::from_secs_f64(wait));
        }
    }
}

struct Shared {
    url: String,
    file: File,
    received: AtomicU64,
    failed: AtomicBool,
    error: Mutex<String>,
    limiter: Limiter,
}

pub fn run(hub: &Hub, id: &str, ctl: &Arc<Control>) {
    let prep = {
        let mut c = hub.lock();
        let s = c.settings.clone();
        c.job_mut(id).map(|job| {
            let dest = job.dest.get_or_insert_with(|| s.download_dir.clone()).clone();
            let staging = job.staging.get_or_insert_with(|| dest.join(".snag-temp").join(&job.id)).clone();
            job.live = Live { stage: Stage::Starting, ..Default::default() };
            job.error.clear();
            job.note.clear();
            job.push_log(format!("Starting: {}", job.url));
            (job.url.clone(), dest, staging, s.connections.max(1), s.speed_limit_kb as u64 * 1024)
        })
    };
    let Some((url, dest, staging, connections, limit)) = prep else { return };

    let result = download(hub, id, ctl, &url, &staging, connections, limit);
    finish(hub, id, ctl, result, &dest, &staging);
}

/// Returns the finished file inside the staging folder, and its name.
fn download(
    hub: &Hub,
    id: &str,
    ctl: &Arc<Control>,
    url: &str,
    staging: &Path,
    connections: u32,
    limit: u64,
) -> Result<(PathBuf, String), Fail> {
    fs::create_dir_all(staging).map_err(|e| Fail::Error(format!("Couldn't use the download folder ({e}).")))?;
    if let Some(root) = staging.parent() {
        crate::platform::hide_path(root);
    }
    let state_path = staging.join("state.json");
    let data_path = staging.join("data.part");

    // Always look again: signed links expire, and the file may have changed.
    let probe = inspect(url).map_err(Fail::Error)?;
    if probe.is_page() {
        return Err(Fail::Error("That link opens a web page, not a file.".into()));
    }
    let mut st: State =
        fs::read_to_string(&state_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if st.url != url || st.size != probe.size || st.ranges != probe.ranges {
        st = State {
            url: url.into(),
            name: probe.name.clone(),
            size: probe.size,
            ranges: probe.ranges,
            ..Default::default()
        };
        let _ = fs::remove_file(&data_path);
    }
    if let Some(j) = hub.lock().job_mut(id) {
        if j.title.is_empty() || j.title == util::short_url(url) {
            j.title = st.name.clone();
        }
        j.live.stage = Stage::Downloading;
        j.live.total = st.size;
        j.push_log(format!(
            "{} · {} · {}",
            st.name,
            st.size.map_or("size unknown".into(), |s| util::fmt_bytes(s as f64)),
            if st.ranges { "resumable" } else { "no resume support" }
        ));
    }
    hub.repaint();

    match st.size {
        Some(size) if st.ranges && size > SPLIT_ABOVE => {
            chunked(hub, id, ctl, &probe.final_url, &mut st, &state_path, &data_path, size, connections, limit)?
        }
        _ => single(hub, id, ctl, &probe.final_url, &st, &data_path, limit)?,
    }
    let _ = fs::remove_file(&state_path);
    Ok((data_path, st.name))
}

#[allow(clippy::too_many_arguments)]
fn chunked(
    hub: &Hub,
    id: &str,
    ctl: &Arc<Control>,
    url: &str,
    st: &mut State,
    state_path: &Path,
    data_path: &Path,
    size: u64,
    connections: u32,
    limit: u64,
) -> Result<(), Fail> {
    // Smaller chunks under a speed limit, so each still finishes well inside
    // its time limit.
    let per_conn = if limit > 0 { limit / connections as u64 } else { u64::MAX };
    let chunk = if st.chunk > 0 { st.chunk } else { (per_conn.saturating_mul(60)).clamp(MIN_CHUNK, MAX_CHUNK) };
    let n = size.div_ceil(chunk) as usize;
    if st.chunk != chunk || st.done.len() != n {
        st.chunk = chunk;
        st.done = vec![false; n];
    }
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(data_path)
        .map_err(|e| Fail::Error(format!("Couldn't write the file ({e}).")))?;
    if file.metadata().map(|m| m.len()).unwrap_or(0) != size {
        file.set_len(size).map_err(|_| Fail::Error("Not enough disk space for this file.".into()))?;
    }
    let already: u64 = (0..n).filter(|&i| st.done[i]).map(|i| chunk_len(i, chunk, size)).sum();
    let shared = Arc::new(Shared {
        url: url.into(),
        file,
        received: AtomicU64::new(already),
        failed: AtomicBool::new(false),
        error: Mutex::new(String::new()),
        limiter: Limiter::new(limit),
    });
    let todo: VecDeque<usize> = (0..n).filter(|&i| !st.done[i]).collect();
    let workers = (connections as usize).min(todo.len()).max(1);
    let queue = Arc::new(Mutex::new(todo));
    let done = Arc::new(Mutex::new(std::mem::take(&mut st.done)));
    let agent = agent(Some(Duration::from_secs(180)));

    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let (shared, queue, done, ctl, agent) =
                (shared.clone(), queue.clone(), done.clone(), ctl.clone(), agent.clone());
            thread::spawn(move || chunk_worker(&agent, &shared, &queue, &done, &ctl, chunk, size))
        })
        .collect();

    let mut meter = Meter::new(already);
    let mut last_save = Instant::now();
    while handles.iter().any(|h| !h.is_finished()) {
        thread::sleep(Duration::from_millis(250));
        meter.report(hub, id, shared.received.load(Ordering::Relaxed), Some(size));
        if last_save.elapsed() > Duration::from_secs(2) {
            st.done = done.lock().unwrap_or_else(|p| p.into_inner()).clone();
            save_state(st, state_path);
            last_save = Instant::now();
        }
    }
    for h in handles {
        let _ = h.join();
    }
    st.done = done.lock().unwrap_or_else(|p| p.into_inner()).clone();
    save_state(st, state_path);
    if ctl.stop() != STOP_NONE {
        return Err(Fail::Stopped);
    }
    if shared.failed.load(Ordering::SeqCst) || st.done.iter().any(|d| !d) {
        let e = shared.error.lock().unwrap_or_else(|p| p.into_inner()).clone();
        return Err(Fail::Error(if e.is_empty() { "The download didn't finish.".into() } else { e }));
    }
    shared.file.sync_all().map_err(|e| Fail::Error(format!("Couldn't finish writing the file ({e}).")))?;
    Ok(())
}

fn chunk_len(i: usize, chunk: u64, size: u64) -> u64 {
    let start = i as u64 * chunk;
    (start + chunk).min(size) - start
}

fn chunk_worker(
    agent: &ureq::Agent,
    sh: &Shared,
    queue: &Mutex<VecDeque<usize>>,
    done: &Mutex<Vec<bool>>,
    ctl: &Control,
    chunk: u64,
    size: u64,
) {
    loop {
        if ctl.stop() != STOP_NONE || sh.failed.load(Ordering::SeqCst) {
            return;
        }
        let Some(i) = queue.lock().unwrap_or_else(|p| p.into_inner()).pop_front() else { return };
        let start = i as u64 * chunk;
        let end = start + chunk_len(i, chunk, size) - 1;
        let mut attempt = 0;
        loop {
            let mut got = 0u64;
            match fetch_range(agent, sh, ctl, start, end, &mut got) {
                Ok(()) => {
                    done.lock().unwrap_or_else(|p| p.into_inner())[i] = true;
                    break;
                }
                Err(fail) => {
                    // Bytes of an unfinished chunk don't count.
                    sh.received.fetch_sub(got, Ordering::Relaxed);
                    let give_up = match fail {
                        Fail::Stopped => true,
                        Fail::Error(e) => {
                            attempt += 1;
                            if attempt >= 6 {
                                *sh.error.lock().unwrap_or_else(|p| p.into_inner()) = e;
                                sh.failed.store(true, Ordering::SeqCst);
                                true
                            } else {
                                thread::sleep(Duration::from_secs(1 << attempt.min(4)));
                                false
                            }
                        }
                    };
                    if give_up {
                        queue.lock().unwrap_or_else(|p| p.into_inner()).push_front(i);
                        return;
                    }
                }
            }
        }
    }
}

fn fetch_range(
    agent: &ureq::Agent,
    sh: &Shared,
    ctl: &Control,
    start: u64,
    end: u64,
    got: &mut u64,
) -> Result<(), Fail> {
    let resp = agent
        .get(&sh.url)
        .header("Range", &format!("bytes={start}-{end}"))
        .header("Accept-Encoding", "identity")
        .call()
        .map_err(|e| Fail::Error(net_err(e)))?;
    if resp.status().as_u16() != 206 {
        return Err(Fail::Error(format!(
            "The server stopped allowing resumed downloads (HTTP {}).",
            resp.status().as_u16()
        )));
    }
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let want = end - start + 1;
    while *got < want {
        if ctl.stop() != STOP_NONE {
            return Err(Fail::Stopped);
        }
        let n = reader.read(&mut buf).map_err(|e| Fail::Error(format!("The connection dropped ({e}).")))?;
        if n == 0 {
            break;
        }
        let n = n.min((want - *got) as usize);
        write_at(&sh.file, &buf[..n], start + *got)
            .map_err(|e| Fail::Error(format!("Couldn't write the file ({e}).")))?;
        *got += n as u64;
        sh.received.fetch_add(n as u64, Ordering::Relaxed);
        sh.limiter.take(n);
    }
    if *got < want {
        return Err(Fail::Error("The connection was cut short.".into()));
    }
    Ok(())
}

/// One connection, start to end. Resumes when the server allows ranges.
fn single(
    hub: &Hub,
    id: &str,
    ctl: &Arc<Control>,
    url: &str,
    st: &State,
    data_path: &Path,
    limit: u64,
) -> Result<(), Fail> {
    let have = if st.ranges { fs::metadata(data_path).map(|m| m.len()).unwrap_or(0) } else { 0 };
    let mut req = agent(None).get(url).header("Accept-Encoding", "identity");
    if have > 0 {
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let resp = req.call().map_err(|e| Fail::Error(net_err(e)))?;
    let status = resp.status().as_u16();
    if status >= 400 {
        return Err(Fail::Error(format!("The server answered HTTP {status}.")));
    }
    let resumed = have > 0 && status == 206;
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(data_path)
        .map_err(|e| Fail::Error(format!("Couldn't write the file ({e}).")))?;
    let mut received = if resumed { have } else { 0 };
    let limiter = Limiter::new(limit);
    let mut meter = Meter::new(received);
    let mut reader = resp.into_body().into_reader();
    let mut buf = vec![0u8; 64 * 1024];
    let mut last = Instant::now();
    loop {
        if ctl.stop() != STOP_NONE {
            return Err(Fail::Stopped);
        }
        let n = reader.read(&mut buf).map_err(|e| Fail::Error(format!("The connection dropped ({e}).")))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| Fail::Error(format!("Couldn't write the file ({e}).")))?;
        received += n as u64;
        limiter.take(n);
        if last.elapsed() > Duration::from_millis(250) {
            meter.report(hub, id, received, st.size);
            last = Instant::now();
        }
    }
    file.flush().map_err(|e| Fail::Error(format!("Couldn't finish writing the file ({e}).")))?;
    if st.size.is_some_and(|s| received < s) {
        return Err(Fail::Error("The connection was cut short.".into()));
    }
    Ok(())
}

/// Turns byte counts into progress, speed and time left.
struct Meter {
    last_bytes: u64,
    last_time: Instant,
    speed: f64,
}

impl Meter {
    fn new(bytes: u64) -> Meter {
        Meter { last_bytes: bytes, last_time: Instant::now(), speed: 0.0 }
    }

    fn report(&mut self, hub: &Hub, id: &str, bytes: u64, total: Option<u64>) {
        let dt = self.last_time.elapsed().as_secs_f64();
        if dt > 0.0 {
            let now_speed = bytes.saturating_sub(self.last_bytes) as f64 / dt;
            self.speed = if self.speed == 0.0 { now_speed } else { self.speed * 0.8 + now_speed * 0.2 };
        }
        self.last_bytes = bytes;
        self.last_time = Instant::now();
        if let Some(j) = hub.lock().job_mut(id) {
            j.live.stage = Stage::Downloading;
            j.live.downloaded = bytes;
            j.live.total = total;
            j.live.speed = (self.speed > 0.0).then_some(self.speed);
            j.live.eta =
                total.filter(|_| self.speed > 1.0).map(|t| (t.saturating_sub(bytes) as f64 / self.speed) as u64);
            if let Some(t) = total.filter(|t| *t > 0) {
                j.progress = j.progress.max((bytes as f64 / t as f64).min(0.999) as f32);
            }
        }
        hub.repaint();
    }
}

fn save_state(st: &State, path: &Path) {
    if let Ok(text) = serde_json::to_string(st) {
        let _ = crate::settings::write_atomic(path, text.as_bytes());
    }
}

fn finish(
    hub: &Hub,
    id: &str,
    ctl: &Arc<Control>,
    result: Result<(PathBuf, String), Fail>,
    dest: &Path,
    staging: &Path,
) {
    let stop = ctl.stop();
    // Move the finished file out before touching shared state.
    let moved = match &result {
        Ok((data, name)) => {
            let target = util::unique_path(&dest.join(name));
            fs::create_dir_all(dest)
                .and_then(|_| fs::rename(data, &target))
                .map(|_| target)
                .map_err(|e| format!("Couldn't move the finished file into {} ({e}).", dest.display()))
        }
        Err(_) => Err(String::new()),
    };
    let mut drop_staging = false;
    {
        let mut c = hub.lock();
        let auto_retry = c.settings.auto_retry;
        let mut done_job = None;
        if let Some(j) = c.job_mut(id) {
            j.control = None;
            j.live = Live::default();
            match (&result, moved) {
                (Ok(_), Ok(path)) => {
                    j.bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    j.files = vec![path];
                    j.status = Status::Done;
                    j.progress = 1.0;
                    j.items_done = 1;
                    j.finished = util::now();
                    j.push_log("Saved.".into());
                    drop_staging = true;
                    done_job = Some(j.clone());
                }
                (Ok(_), Err(e)) => {
                    j.status = Status::Failed;
                    j.error = e;
                    j.finished = util::now();
                }
                (Err(_), _) if stop == STOP_PAUSE => {
                    j.status = Status::Paused;
                    j.push_log("Paused.".into());
                }
                (Err(_), _) if stop == STOP_CANCEL => {
                    j.status = Status::Cancelled;
                    j.finished = util::now();
                    drop_staging = true;
                }
                (Err(Fail::Error(e)), _) => {
                    j.push_log(format!("Failed: {e}"));
                    crate::hub::fail_or_retry(j, e.clone(), auto_retry);
                }
                (Err(Fail::Stopped), _) => j.status = Status::Paused,
            }
            c.dirty.queue = true;
        } else {
            drop_staging = true;
        }
        if let Some(job) = done_job {
            c.record_history(&job);
        }
    }
    if drop_staging {
        let _ = fs::remove_dir_all(staging);
        if let Some(root) = staging.parent() {
            let _ = fs::remove_dir(root);
        }
    }
    hub.wake();
    hub.repaint();
}

#[cfg(windows)]
fn write_at(f: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < buf.len() {
        done += f.seek_write(&buf[done..], offset + done as u64)?;
    }
    Ok(())
}

#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(buf, offset)
}

fn net_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "The server stopped responding (timed out).".into(),
        ureq::Error::HostNotFound => "Couldn't find that server. Check the link and your connection.".into(),
        ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            "Couldn't reach the server. Check your internet connection.".into()
        }
        other => format!("Network error: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// A tiny HTTP server that serves `data` with byte-range support.
    fn serve(data: Vec<u8>, name: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let data = Arc::new(data);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let data = data.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut range = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some(r) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            let (a, b) = r.trim().split_once('-').unwrap();
                            let a: usize = a.parse().unwrap();
                            let b: usize = b.parse().unwrap_or(data.len() - 1).min(data.len() - 1);
                            range = Some((a, b));
                        }
                    }
                    let mut out = stream;
                    let (status, body, extra) = match range {
                        Some((a, b)) => (
                            "206 Partial Content",
                            &data[a..=b],
                            format!("Content-Range: bytes {a}-{b}/{}\r\n", data.len()),
                        ),
                        None => ("200 OK", &data[..], String::new()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/zip\r\n\
                         Content-Disposition: attachment; filename=\"{name}\"\r\n{extra}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = out.write_all(head.as_bytes());
                    let _ = out.write_all(body);
                });
            }
        });
        format!("http://{addr}/files/{name}")
    }

    #[test]
    fn inspects_a_file_link() {
        let url = serve(vec![7u8; 5000], "plans v2.zip");
        let p = inspect(&url).unwrap();
        assert_eq!(p.size, Some(5000));
        assert!(p.ranges && !p.is_page());
        assert_eq!(p.name, "plans v2.zip");
    }

    #[test]
    fn downloads_in_chunks_and_resumes() {
        let data: Vec<u8> = (0..(3 * 1024 * 1024 + 12_345)).map(|i| (i % 251) as u8).collect();
        let url = serve(data.clone(), "big.bin");
        let dir = std::env::temp_dir().join(format!("snag-direct-test-{}", util::new_id()));
        fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state.json");
        let data_path = dir.join("data.part");
        let hub = Hub::load_empty();
        let ctl = Arc::new(Control::default());
        let size = data.len() as u64;
        // Pretend an earlier run finished every other chunk.
        let mut st = State {
            url: url.clone(),
            name: "big.bin".into(),
            size: Some(size),
            ranges: true,
            chunk: MIN_CHUNK,
            done: vec![],
        };
        let n = size.div_ceil(MIN_CHUNK) as usize;
        st.done = (0..n).map(|i| i % 2 == 0).collect();
        let file = OpenOptions::new().create(true).write(true).truncate(true).open(&data_path).unwrap();
        file.set_len(size).unwrap();
        for i in (0..n).step_by(2) {
            let a = i * MIN_CHUNK as usize;
            let b = (a + MIN_CHUNK as usize).min(data.len());
            write_at(&file, &data[a..b], a as u64).unwrap();
        }
        drop(file);
        chunked(&hub, "none", &ctl, &url, &mut st, &state_path, &data_path, size, 4, 0).ok().unwrap();
        assert_eq!(fs::read(&data_path).unwrap(), data);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_come_from_headers_or_links() {
        assert_eq!(
            file_name("attachment; filename*=UTF-8''r%C3%A9sum%C3%A9.pdf", "https://x.com/dl?id=1", ""),
            "résumé.pdf"
        );
        assert_eq!(file_name("", "https://x.com/a/My%20File.zip?token=1", ""), "My File.zip");
        assert_eq!(file_name("", "https://x.com/download", "application/pdf"), "download.pdf");
    }
}
