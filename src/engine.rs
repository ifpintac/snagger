//! The download engine Snagger drives: yt-dlp (the site extractors), ffmpeg
//! (joining and converting) and a JavaScript runtime that YouTube needs.
//! All three come from their official GitHub releases and live in
//! %LOCALAPPDATA%\Snagger\engine. Nothing is installed system-wide.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::hub::Hub;
use crate::model::Status;
use crate::platform;
use crate::settings::engine_dir;

const YTDLP_ASSET: &str = "yt-dlp_win.zip";
const FFMPEG_URL: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
const FFMPEG_SUMS: &str = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/checksums.sha256";
const FFMPEG_ASSET: &str = "ffmpeg-master-latest-win64-gpl-shared.zip";
const DENO_ASSET: &str = "deno-x86_64-pc-windows-msvc.zip";

#[derive(Clone, Debug, Default)]
pub struct Task {
    pub title: String,
    pub done: u64,
    pub total: u64,
}

#[derive(Clone, Debug)]
pub struct JsRuntime {
    pub kind: String,
    pub path: PathBuf,
    pub system: bool,
}

#[derive(Clone, Debug, Default)]
pub struct EngineState {
    pub ytdlp: Option<PathBuf>,
    pub ytdlp_version: String,
    pub ffmpeg: Option<PathBuf>,
    pub ffmpeg_system: bool,
    pub js: Option<JsRuntime>,
    pub busy: Option<Task>,
    pub error: String,
    pub note: String,
    pub latest: String,
    /// Detection (and the startup update check) finished.
    pub ready: bool,
    /// Something was missing at startup, so the setup screen is showing.
    pub setting_up: bool,
}

/// What a download needs to start yt-dlp.
#[derive(Clone, Debug)]
pub struct EnginePaths {
    pub ytdlp: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    pub js: Option<JsRuntime>,
}

impl EngineState {
    pub fn paths(&self) -> Option<EnginePaths> {
        Some(EnginePaths { ytdlp: self.ytdlp.clone()?, ffmpeg: self.ffmpeg.clone(), js: self.js.clone() })
    }

    pub fn missing(&self) -> Vec<Part> {
        let mut out = Vec::new();
        if self.ytdlp.is_none() {
            out.push(Part::YtDlp);
        }
        if self.ffmpeg.is_none() {
            out.push(Part::Ffmpeg);
        }
        if self.js.is_none() {
            out.push(Part::Deno);
        }
        out
    }

    pub fn warnings(&self) -> Vec<&'static str> {
        let mut w = Vec::new();
        if self.ffmpeg.is_none() {
            w.push("ffmpeg is missing, so high-quality YouTube video, audio conversion and trimming won't work.");
        }
        if self.js.is_none() {
            w.push("No JavaScript runtime (Deno), so YouTube downloads may fail.");
        }
        w
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    YtDlp,
    Ffmpeg,
    Deno,
}

impl Part {
    pub fn name(self) -> &'static str {
        match self {
            Part::YtDlp => "yt-dlp",
            Part::Ffmpeg => "ffmpeg",
            Part::Deno => "Deno",
        }
    }
    pub fn approx_size(self) -> &'static str {
        match self {
            Part::YtDlp => "18 MB",
            Part::Ffmpeg => "90 MB",
            Part::Deno => "45 MB",
        }
    }
    pub fn purpose(self) -> &'static str {
        match self {
            Part::YtDlp => "reads 1,800+ sites and downloads",
            Part::Ffmpeg => "joins video and audio, converts formats",
            Part::Deno => "unlocks YouTube's video links",
        }
    }
}

fn ytdlp_dir() -> PathBuf {
    engine_dir().join("yt-dlp")
}
fn ffmpeg_dir() -> PathBuf {
    engine_dir().join("ffmpeg")
}
fn deno_dir() -> PathBuf {
    engine_dir().join("deno")
}

// --------------------------------------------------------------------------
// Finding what's installed
// --------------------------------------------------------------------------

fn which(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

fn node_version(path: &Path) -> Option<(u32, u32)> {
    let mut cmd = Command::new(path);
    cmd.arg("--version").stdin(Stdio::null()).stderr(Stdio::null());
    platform::hide_window(&mut cmd);
    let out = cmd.output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout);
    let mut it = v.trim().trim_start_matches('v').split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

pub fn detect() -> EngineState {
    let mut st = EngineState::default();

    let yt = ytdlp_dir().join("yt-dlp.exe");
    if yt.is_file() {
        st.ytdlp_version = fs::read_to_string(ytdlp_dir().join("version.txt")).unwrap_or_default().trim().to_string();
        st.ytdlp = Some(yt);
    }

    if ffmpeg_dir().join("ffmpeg.exe").is_file() {
        st.ffmpeg = Some(ffmpeg_dir());
    } else if let (Some(f), Some(_)) = (which("ffmpeg"), which("ffprobe")) {
        st.ffmpeg = f.parent().map(Path::to_path_buf);
        st.ffmpeg_system = true;
    }

    // yt-dlp supports Deno (preferred) and Node.js 23.5 or newer.
    let bundled = deno_dir().join("deno.exe");
    st.js = if bundled.is_file() {
        Some(JsRuntime { kind: "deno".into(), path: bundled, system: false })
    } else if let Some(p) = which("deno") {
        Some(JsRuntime { kind: "deno".into(), path: p, system: true })
    } else {
        which("node").filter(|p| node_version(p).is_some_and(|v| v >= (23, 5))).map(|p| JsRuntime {
            kind: "node".into(),
            path: p,
            system: true,
        })
    };
    st
}

fn apply_detection(hub: &Hub) {
    let found = detect();
    let mut c = hub.lock();
    let e = &mut c.engine;
    e.ytdlp = found.ytdlp;
    e.ytdlp_version = found.ytdlp_version;
    e.ffmpeg = found.ffmpeg;
    e.ffmpeg_system = found.ffmpeg_system;
    e.js = found.js;
}

// --------------------------------------------------------------------------
// Background tasks
// --------------------------------------------------------------------------

/// Startup: find the engine, install what's missing, then check for a newer
/// yt-dlp. Downloads wait until this is done.
pub fn start(hub: &Hub) {
    let hub = hub.clone();
    thread::spawn(move || {
        apply_detection(&hub);
        let (missing, auto_update, nightly) = {
            let mut c = hub.lock();
            let missing = c.engine.missing();
            c.engine.setting_up = !missing.is_empty();
            (missing, c.settings.auto_update, c.settings.nightly)
        };
        if !missing.is_empty() {
            install_parts(&hub, &missing);
        } else if auto_update && let Err(e) = update_ytdlp(&hub, nightly, false) {
            hub.lock().engine.note = format!("Couldn't check for engine updates: {e}");
        }
        finish(&hub);
    });
}

/// From the setup screen or Settings: install whatever is missing.
pub fn install_missing(hub: &Hub) {
    run_task(hub, |hub| {
        apply_detection(hub);
        let missing = hub.lock().engine.missing();
        install_parts(hub, &missing);
    });
}

pub fn reinstall(hub: &Hub, part: Part) {
    run_task(hub, move |hub| {
        let nightly = hub.lock().settings.nightly;
        let r = match part {
            Part::YtDlp => install_ytdlp(hub, nightly).map(|_| ()),
            Part::Ffmpeg => install_ffmpeg(hub),
            Part::Deno => install_deno(hub),
        };
        apply_detection(hub);
        let mut c = hub.lock();
        match r {
            Ok(()) => {
                c.engine.error.clear();
                c.engine.note = format!("Reinstalled {}.", part.name());
            }
            Err(e) => c.engine.error = format!("Couldn't install {}: {e}", part.name()),
        }
    });
}

/// Settings → "Check for updates".
pub fn check_updates(hub: &Hub) {
    run_task(hub, |hub| {
        let nightly = hub.lock().settings.nightly;
        let r = update_ytdlp(hub, nightly, true);
        let mut c = hub.lock();
        match r {
            Ok(true) => {}
            Ok(false) => c.engine.note = format!("yt-dlp {} is the newest version.", c.engine.ytdlp_version),
            Err(e) => c.engine.error = format!("Couldn't check for updates: {e}"),
        }
    });
}

fn run_task(hub: &Hub, f: impl FnOnce(&Hub) + Send + 'static) {
    {
        let mut c = hub.lock();
        if c.engine.busy.is_some() {
            return;
        }
        if c.jobs.iter().any(|j| j.status == Status::Running) {
            c.engine.error = "Pause or finish the running downloads first.".into();
            return;
        }
        c.engine.error.clear();
        c.engine.note.clear();
        c.engine.busy = Some(Task { title: "Starting…".into(), ..Default::default() });
    }
    let hub = hub.clone();
    thread::spawn(move || {
        f(&hub);
        finish(&hub);
    });
}

fn finish(hub: &Hub) {
    {
        let mut c = hub.lock();
        c.engine.busy = None;
        c.engine.ready = true;
        if c.engine.missing().is_empty() || c.engine.ytdlp.is_some() && c.engine.error.is_empty() {
            c.engine.setting_up = false;
        }
    }
    hub.wake();
    hub.repaint();
}

fn install_parts(hub: &Hub, parts: &[Part]) {
    let nightly = hub.lock().settings.nightly;
    let mut errors = Vec::new();
    for &p in parts {
        let r = match p {
            Part::YtDlp => install_ytdlp(hub, nightly).map(|_| ()),
            Part::Ffmpeg => install_ffmpeg(hub),
            Part::Deno => install_deno(hub),
        };
        if let Err(e) = r {
            errors.push(format!("{}: {e}", p.name()));
        }
        apply_detection(hub);
    }
    let mut c = hub.lock();
    if errors.is_empty() {
        c.engine.error.clear();
        c.engine.setting_up = false;
    } else {
        c.engine.error = format!("Setup didn't finish. {}", errors.join(" · "));
    }
}

fn set_task(hub: &Hub, title: &str, done: u64, total: u64) {
    hub.lock().engine.busy = Some(Task { title: title.into(), done, total });
    hub.repaint();
}

// --------------------------------------------------------------------------
// Installers
// --------------------------------------------------------------------------

/// Returns Ok(true) when it installed a new version.
fn update_ytdlp(hub: &Hub, nightly: bool, user_asked: bool) -> Result<bool, String> {
    if user_asked {
        set_task(hub, "Checking for updates…", 0, 0);
    }
    let tag = latest_tag(repo(nightly))?;
    let current = hub.lock().engine.ytdlp_version.clone();
    hub.lock().engine.latest = tag.clone();
    if tag == current {
        return Ok(false);
    }
    let installed = install_ytdlp(hub, nightly)?;
    apply_detection(hub);
    let mut c = hub.lock();
    c.engine.note = if current.is_empty() {
        format!("Installed yt-dlp {installed}.")
    } else {
        format!("Updated yt-dlp from {current} to {installed}.")
    };
    Ok(true)
}

fn repo(nightly: bool) -> &'static str {
    if nightly { "yt-dlp/yt-dlp-nightly-builds" } else { "yt-dlp/yt-dlp" }
}

fn install_ytdlp(hub: &Hub, nightly: bool) -> Result<String, String> {
    set_task(hub, "Finding the latest yt-dlp…", 0, 0);
    let repo = repo(nightly);
    let tag = latest_tag(repo)?;
    let base = format!("https://github.com/{repo}/releases/download/{tag}");
    let zip = download_dir()?.join(YTDLP_ASSET);
    let digest = download(hub, &format!("{base}/{YTDLP_ASSET}"), &zip, &format!("Downloading yt-dlp {tag}"))?;
    verify(&digest, &format!("{base}/SHA2-256SUMS"), YTDLP_ASSET)?;

    set_task(hub, "Unpacking yt-dlp…", 0, 0);
    let staged = engine_dir().join("yt-dlp.new");
    let _ = fs::remove_dir_all(&staged);
    extract(&zip, &staged, |name| Some(PathBuf::from(name)))?;
    if !staged.join("yt-dlp.exe").is_file() {
        return Err("the download didn't contain yt-dlp.exe".into());
    }
    fs::write(staged.join("version.txt"), &tag).map_err(io_err)?;
    swap_into_place(&staged, &ytdlp_dir())?;
    let _ = fs::remove_file(&zip);
    Ok(tag)
}

fn install_ffmpeg(hub: &Hub) -> Result<(), String> {
    let zip = download_dir()?.join(FFMPEG_ASSET);
    let digest = download(hub, FFMPEG_URL, &zip, "Downloading ffmpeg")?;
    verify(&digest, FFMPEG_SUMS, FFMPEG_ASSET)?;
    set_task(hub, "Unpacking ffmpeg…", 0, 0);
    let staged = engine_dir().join("ffmpeg.new");
    let _ = fs::remove_dir_all(&staged);
    // Only bin\ matters: ffmpeg.exe, ffprobe.exe and their DLLs (not the ffplay player).
    extract(&zip, &staged, |name| {
        let lower = name.to_ascii_lowercase();
        let file = name.rsplit('/').next()?;
        let wanted = lower.contains("/bin/")
            && (lower.ends_with(".exe") || lower.ends_with(".dll"))
            && !lower.ends_with("ffplay.exe");
        wanted.then(|| PathBuf::from(file))
    })?;
    if !staged.join("ffmpeg.exe").is_file() {
        return Err("the download didn't contain ffmpeg.exe".into());
    }
    swap_into_place(&staged, &ffmpeg_dir())?;
    let _ = fs::remove_file(&zip);
    Ok(())
}

fn install_deno(hub: &Hub) -> Result<(), String> {
    set_task(hub, "Finding the latest Deno…", 0, 0);
    let tag = latest_tag("denoland/deno")?;
    let base = format!("https://github.com/denoland/deno/releases/download/{tag}");
    let zip = download_dir()?.join(DENO_ASSET);
    let digest = download(hub, &format!("{base}/{DENO_ASSET}"), &zip, &format!("Downloading Deno {tag}"))?;
    verify(&digest, &format!("{base}/{DENO_ASSET}.sha256sum"), DENO_ASSET)?;
    set_task(hub, "Unpacking Deno…", 0, 0);
    let staged = engine_dir().join("deno.new");
    let _ = fs::remove_dir_all(&staged);
    extract(&zip, &staged, |name| name.eq_ignore_ascii_case("deno.exe").then(|| PathBuf::from("deno.exe")))?;
    if !staged.join("deno.exe").is_file() {
        return Err("the download didn't contain deno.exe".into());
    }
    swap_into_place(&staged, &deno_dir())?;
    let _ = fs::remove_file(&zip);
    Ok(())
}

// --------------------------------------------------------------------------
// HTTP, checksums, zip
// --------------------------------------------------------------------------

fn agent(follow_redirects: bool) -> ureq::Agent {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .http_status_as_error(false)
        .user_agent("Snagger/2.0");
    if !follow_redirects {
        cfg = cfg.max_redirects(0).max_redirects_will_error(false);
    }
    cfg.build().into()
}

/// The newest release tag, read from GitHub's /releases/latest redirect
/// (no API rate limit).
fn latest_tag(repo: &str) -> Result<String, String> {
    let resp = agent(false).get(&format!("https://github.com/{repo}/releases/latest")).call().map_err(net_err)?;
    let loc = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| format!("GitHub answered {} without a release link", resp.status()))?;
    let tag = loc.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    if tag.is_empty() || tag == "latest" || tag == "releases" {
        return Err("couldn't read the latest version from GitHub".into());
    }
    Ok(tag.to_string())
}

fn fetch_text(url: &str) -> Result<String, String> {
    let resp = agent(true).get(url).call().map_err(net_err)?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }
    resp.into_body().read_to_string().map_err(net_err)
}

/// Streams `url` into `dest`, reporting progress. Returns the SHA-256.
fn download(hub: &Hub, url: &str, dest: &Path, title: &str) -> Result<String, String> {
    set_task(hub, title, 0, 0);
    let resp = agent(true).get(url).call().map_err(net_err)?;
    if !resp.status().is_success() {
        return Err(format!("the server answered HTTP {}", resp.status().as_u16()));
    }
    let total = resp.body().content_length().unwrap_or(0);
    let mut reader = resp.into_body().into_reader();
    let mut file = File::create(dest).map_err(io_err)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut last = Instant::now();
    set_task(hub, title, 0, total);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download was interrupted ({e})"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(io_err)?;
        hasher.update(&buf[..n]);
        done += n as u64;
        if last.elapsed() > Duration::from_millis(150) {
            set_task(hub, title, done, total);
            last = Instant::now();
        }
    }
    file.flush().map_err(io_err)?;
    if total > 0 && done != total {
        return Err("the download was cut short. Try again.".into());
    }
    set_task(hub, title, done, total.max(done));
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Compares against the release's published checksum. If the checksum file
/// itself can't be fetched, HTTPS from GitHub is trusted on its own.
fn verify(digest: &str, sums_url: &str, file_name: &str) -> Result<(), String> {
    let Ok(sums) = fetch_text(sums_url) else { return Ok(()) };
    let is_hash = |t: &&str| t.len() == 64 && t.chars().all(|c| c.is_ascii_hexdigit());
    let expected =
        sums.lines().filter(|l| l.contains(file_name)).find_map(|l| l.split_whitespace().find(is_hash)).or_else(|| {
            let all: Vec<&str> = sums.split_whitespace().filter(is_hash).collect();
            (all.len() == 1).then(|| all[0])
        });
    match expected {
        Some(h) if !h.eq_ignore_ascii_case(digest) => {
            Err("the downloaded file didn't match its published checksum. Try again.".into())
        }
        _ => Ok(()),
    }
}

/// Extracts the entries `map` accepts, at the relative paths it returns.
fn extract(zip_path: &Path, dest: &Path, map: impl Fn(&str) -> Option<PathBuf>) -> Result<usize, String> {
    let file = File::open(zip_path).map_err(io_err)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("the download isn't a valid zip ({e})"))?;
    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        // enclosed_name refuses entries that would escape the folder.
        let Some(safe) = entry.enclosed_name() else { continue };
        let name = safe.to_string_lossy().replace('\\', "/");
        let Some(rel) = map(&name) else { continue };
        let out = dest.join(rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        let mut f = File::create(&out).map_err(io_err)?;
        std::io::copy(&mut entry, &mut f).map_err(io_err)?;
        count += 1;
    }
    Ok(count)
}

/// Replaces `target` with `staged`. Antivirus scanners briefly lock new
/// files, so renames are retried for a few seconds.
fn swap_into_place(staged: &Path, target: &Path) -> Result<(), String> {
    let old = target.with_extension("old");
    let _ = fs::remove_dir_all(&old);
    if target.exists() {
        retry(|| fs::rename(target, &old)).map_err(|e| format!("couldn't replace the old copy ({e})"))?;
    }
    retry(|| fs::rename(staged, target)).map_err(|e| format!("couldn't move the new copy into place ({e})"))?;
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

fn retry(mut f: impl FnMut() -> std::io::Result<()>) -> std::io::Result<()> {
    let mut last = Ok(());
    for _ in 0..25 {
        last = f();
        if last.is_ok() {
            return last;
        }
        thread::sleep(Duration::from_millis(200));
    }
    last
}

fn download_dir() -> Result<PathBuf, String> {
    let d = engine_dir().join("downloads");
    fs::create_dir_all(&d).map_err(io_err)?;
    Ok(d)
}

fn io_err(e: std::io::Error) -> String {
    e.to_string()
}

fn net_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Io(_) | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) => {
            "couldn't reach GitHub. Check your internet connection.".into()
        }
        other => other.to_string(),
    }
}
