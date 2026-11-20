//! Everything about talking to yt-dlp: building command lines, reading a
//! link's details for the preview, and parsing Snagger's progress lines.

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::engine::EnginePaths;
use crate::model::{Control, Job, PlaylistMode, Quality, STOP_CANCEL, VideoFormat};
use crate::platform::{self, ProcGroup};
use crate::settings::{NameStyle, Organize, Settings, SubsMode};
use crate::util;

pub fn command(paths: &EnginePaths) -> Command {
    let mut cmd = Command::new(&paths.ytdlp);
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1").stdin(Stdio::null());
    platform::hide_window(&mut cmd);
    cmd
}

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

/// Options every yt-dlp run gets: engine locations, sign-in and network.
fn common_args(s: &Settings, p: &EnginePaths, args: &mut Vec<OsString>) {
    push(args, &["--ignore-config", "--color", "never", "--encoding", "utf-8"]);
    if let Some(js) = &p.js {
        args.push("--js-runtimes".into());
        args.push(format!("{}:{}", js.kind, js.path.display()).into());
    }
    if let Some(ff) = &p.ffmpeg {
        args.push("--ffmpeg-location".into());
        args.push(ff.as_os_str().to_owned());
    }
    if !s.cookies_file.trim().is_empty() {
        args.push("--cookies".into());
        args.push(s.cookies_file.trim().into());
    } else if !s.cookies_browser.is_empty() {
        push(args, &["--cookies-from-browser", &s.cookies_browser]);
    }
    if !s.proxy.trim().is_empty() {
        push(args, &["--proxy", s.proxy.trim()]);
    }
    push(args, &["--extractor-retries", "3"]);
}

// --------------------------------------------------------------------------
// Download command line
// --------------------------------------------------------------------------

const P_TMPL: &str = "download:SNAG:P\t%(progress.{status,downloaded_bytes,total_bytes,total_bytes_estimate,speed,eta,fragment_index,fragment_count})j\t%(info.{format_id,vcodec,playlist_autonumber})j";
const PP_TMPL: &str = "postprocess:SNAG:PP\t%(progress.{status,postprocessor})j";
const I_TMPL: &str = "before_dl:SNAG:I\t%(.{id,title,uploader,channel,duration,thumbnail,webpage_url,extractor_key,playlist_title,playlist_autonumber,n_entries,is_live})j\t%(requested_formats.:.{format_id,vcodec,filesize,filesize_approx})j";
const F_TMPL: &str = "after_move:SNAG:F\t%(.{filepath,id,playlist_autonumber,n_entries,webpage_url})j";

/// `%` is special in output templates.
fn tmpl_escape(s: &str) -> String {
    s.replace('%', "%%")
}

/// The output template, sized to fit.
///
/// Windows (and yt-dlp, which can't use `\\?\` paths) stops at 260
/// characters, and yt-dlp's work files are the deepest path there is:
/// `<staging>\work\<folders>\<name>.f137.webm.part-Frag12`. So the room left
/// under the staging folder is shared out between the playlist folder, the
/// uploader folder and the title. Lengths are in UTF-8 bytes, which is never
/// fewer than the UTF-16 units Windows counts.
pub fn output_template(job: &Job, s: &Settings, staging: &Path) -> String {
    let work_len = staging.join("work").to_string_lossy().encode_utf16().count() + 1;
    let clip = job
        .opts
        .clip
        .map(|(a, b)| {
            let end = if b > 0.0 { util::clip_tag(b) } else { "end".into() };
            format!(" [clip {}-{end}]", util::clip_tag(a))
        })
        .unwrap_or_default();
    let sub_name = util::safe_name(&job.opts.folder, 80);
    let sub = if job.opts.folder.is_empty() { String::new() } else { format!("{}/", tmpl_escape(&sub_name)) };
    let is_playlist = job.opts.sub_id.is_empty();

    // Room for folders + name, after the extension and yt-dlp's
    // ".fNNN.ext.part-FragNN" suffixes (28) and Snagger's " (2)" on a clash.
    let mut room = 259usize.saturating_sub(work_len + 32 + clip.len() + sub.len());
    let playlist_len = if is_playlist { (room / 4).clamp(16, 70) } else { 0 };
    let uploader_len = (room / 5).clamp(12, 50);
    room = room.saturating_sub(match s.organize {
        Organize::None => 0,
        Organize::BySite => 25,
        Organize::ByUploader => uploader_len + 1,
    });
    if is_playlist {
        room = room.saturating_sub(playlist_len + 8); // "/" and "0001 - "
    }
    let style_extra = match s.name_style {
        NameStyle::TitleId => 24,
        NameStyle::UploaderTitle => uploader_len + 3,
        NameStyle::DateTitle => 13,
        NameStyle::Title | NameStyle::Custom => 0,
    };
    let t = room.saturating_sub(style_extra).clamp(24, 180);

    let base = match s.name_style {
        NameStyle::TitleId => format!("%(title).{t}B [%(id)s]"),
        NameStyle::Title => format!("%(title).{t}B"),
        NameStyle::UploaderTitle => format!("%(uploader,channel,uploader_id|Unknown).{uploader_len}B - %(title).{t}B"),
        NameStyle::DateTitle => format!("%(upload_date>%Y-%m-%d|Undated)s - %(title).{t}B"),
        NameStyle::Custom => {
            let c = s.custom_template.trim().trim_end_matches(".%(ext)s").to_string();
            if c.is_empty() { format!("%(title).{t}B [%(id)s]") } else { c }
        }
    };
    let org = match s.organize {
        Organize::None => String::new(),
        Organize::BySite => "%(webpage_url_domain|Other).24B/".to_string(),
        Organize::ByUploader => format!("%(uploader,channel,uploader_id|Unknown).{uploader_len}B/"),
    };
    // Playlists get their own folder and numbered files; single videos get
    // "." (no folder) because the playlist fields are empty for them.
    let playlist = if is_playlist {
        format!("%(playlist_title,playlist_id|.).{playlist_len}B/%(playlist_index&{{:03d}} - |)s")
    } else {
        String::new()
    };
    format!("{sub}{org}{playlist}{base}{clip}.%(ext)s")
}

/// What a download would be called with these settings, for the preview in
/// Settings. Mirrors `output_template` with a well-known example video.
pub fn sample_name(s: &Settings, playlist: bool) -> String {
    let org = match s.organize {
        Organize::None => "",
        Organize::BySite => "youtube.com\\",
        Organize::ByUploader => "jawed\\",
    };
    let pl = if playlist { "My playlist\\001 - " } else { "" };
    let base = match s.name_style {
        NameStyle::TitleId => "Me at the zoo [jNQXAC9IVRw]".to_string(),
        NameStyle::Title => "Me at the zoo".to_string(),
        NameStyle::UploaderTitle => "jawed - Me at the zoo".to_string(),
        NameStyle::DateTitle => "2005-04-24 - Me at the zoo".to_string(),
        NameStyle::Custom => {
            let t = s.custom_template.trim().trim_end_matches(".%(ext)s");
            if t.is_empty() { "Me at the zoo [jNQXAC9IVRw]".to_string() } else { fill_sample(t) }
        }
    };
    let ext = if s.preset.quality.is_audio() {
        match s.preset.audio.ytdlp() {
            "best" => "m4a",
            other => other,
        }
    } else {
        match s.preset.video {
            VideoFormat::Mkv => "mkv",
            VideoFormat::Mp4 | VideoFormat::Original => "mp4",
        }
    };
    let name = format!("{org}{pl}{base}.{ext}");
    if s.restrict_names { name.replace(' ', "_") } else { name }
}

/// Fills the common fields of a custom template with the example video.
fn fill_sample(t: &str) -> String {
    let fields = [
        ("title", "Me at the zoo"),
        ("id", "jNQXAC9IVRw"),
        ("uploader", "jawed"),
        ("channel", "jawed"),
        ("upload_date", "20050424"),
        ("playlist_title", "My playlist"),
        ("playlist_index", "1"),
        ("webpage_url_domain", "youtube.com"),
        ("extractor", "youtube"),
    ];
    let mut out = String::new();
    let mut rest = t;
    while let Some(start) = rest.find("%(") {
        out.push_str(&rest[..start]);
        let Some(close) = rest[start..].find(')') else { break };
        let inner = &rest[start + 2..start + close];
        let name = inner.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("");
        out.push_str(fields.iter().find(|(f, _)| *f == name).map_or("…", |(_, v)| v));
        let after = &rest[start + close + 1..];
        // Skip the conversion letter (and any width) after the brackets.
        let skip = after.find(|c: char| c.is_ascii_alphabetic()).map_or(0, |i| i + 1);
        rest = &after[skip..];
    }
    out.push_str(rest);
    out.replace('/', "\\")
}

pub fn download_args(job: &Job, s: &Settings, p: &EnginePaths, staging: &Path, archive: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    common_args(s, p, &mut a);
    push(&mut a, &["--newline", "--progress", "--progress-delta", "0.3", "--no-simulate"]);
    push(&mut a, &["--progress-template", P_TMPL, "--progress-template", PP_TMPL]);
    push(&mut a, &["--print", I_TMPL, "--print", F_TMPL]);
    a.push("--paths".into());
    a.push(format!("home:{}", staging.join("out").display()).into());
    a.push("--paths".into());
    a.push(format!("temp:{}", staging.join("work").display()).into());
    a.push("-o".into());
    a.push(output_template(job, s, staging).into());
    a.push("--download-archive".into());
    a.push(archive.as_os_str().to_owned());
    if job.opts.break_on_existing {
        a.push("--break-on-existing".into());
    }
    push(&mut a, &["--no-mtime", "--no-abort-on-error", "--windows-filenames"]);
    let retries = s.retries.to_string();
    // After two refusals, ask for one piece at a time: some servers refuse
    // many parallel requests for the same stream.
    let frags = if job.refusals >= 2 { "1".to_string() } else { s.fragments.to_string() };
    push(&mut a, &["--retries", &retries, "--fragment-retries", &retries, "-N", &frags]);
    if s.restrict_names {
        a.push("--restrict-filenames".into());
    }
    if s.speed_limit_kb > 0 {
        push(&mut a, &["--limit-rate", &format!("{}K", s.speed_limit_kb)]);
    }

    match job.opts.playlist {
        PlaylistMode::Auto => {}
        PlaylistMode::Single => a.push("--no-playlist".into()),
        PlaylistMode::Whole => a.push("--yes-playlist".into()),
    }
    if !job.opts.items.trim().is_empty() {
        push(&mut a, &["--playlist-items", job.opts.items.trim()]);
    }
    // Downloading a whole playlist or channel flat out gets you blocked
    // ("HTTP Error 429"); a short pause between videos avoids that.
    let many = job.opts.playlist == PlaylistMode::Whole || !job.opts.sub_id.is_empty() || job.n_items > 1;
    if s.gentle_playlists && many {
        push(&mut a, &["--sleep-requests", "0.75", "--sleep-interval", "2", "--max-sleep-interval", "6"]);
    }

    // Quality and format.
    let preset = job.opts.preset;
    let has_ffmpeg = p.ffmpeg.is_some();
    if preset.quality.is_audio() {
        push(&mut a, &["-f", "ba/b", "-x", "--audio-format", preset.audio.ytdlp(), "--audio-quality", "0"]);
        if s.embed_thumbnail && preset.audio.takes_thumbnail() && has_ffmpeg {
            push(&mut a, &["--embed-thumbnail", "--convert-thumbnails", "jpg"]);
        }
    } else {
        let res = match preset.quality {
            Quality::Max(h) => format!("res:{h}"),
            _ => "res".into(),
        };
        if has_ffmpeg {
            a.push("-f".into());
            a.push("bv*+ba/b".into());
        } else {
            // Without ffmpeg only single-file formats work.
            a.push("-f".into());
            a.push("b/bv*+ba".into());
        }
        match preset.video {
            VideoFormat::Mp4 => {
                push(&mut a, &["-S", &format!("{res},fps,+codec:avc:m4a"), "--merge-output-format", "mp4"]);
                if has_ffmpeg {
                    push(&mut a, &["--remux-video", "mp4"]);
                }
            }
            VideoFormat::Mkv => push(&mut a, &["-S", &format!("{res},fps"), "--merge-output-format", "mkv"]),
            VideoFormat::Original => push(&mut a, &["-S", &format!("{res},fps")]),
        }
        let embeddable = matches!(preset.video, VideoFormat::Mp4 | VideoFormat::Mkv);
        if s.embed_thumbnail && embeddable && has_ffmpeg {
            push(&mut a, &["--embed-thumbnail", "--convert-thumbnails", "jpg"]);
        }
        match s.subtitles {
            SubsMode::Off => {}
            SubsMode::Embed if embeddable && has_ffmpeg => {
                push(&mut a, &["--write-subs", "--sub-langs", s.sub_langs.trim(), "--embed-subs"]);
                if s.auto_subs {
                    a.push("--write-auto-subs".into());
                }
            }
            _ => {
                push(&mut a, &["--write-subs", "--sub-langs", s.sub_langs.trim()]);
                if has_ffmpeg {
                    push(&mut a, &["--convert-subs", "srt"]);
                }
                if s.auto_subs {
                    a.push("--write-auto-subs".into());
                }
            }
        }
        if s.sponsorblock && has_ffmpeg {
            push(&mut a, &["--sponsorblock-remove", "sponsor,selfpromo,interaction"]);
        }
    }
    if s.embed_metadata && has_ffmpeg {
        push(&mut a, &["--embed-metadata", "--embed-chapters"]);
    }
    if s.write_description {
        a.push("--write-description".into());
    }
    if s.write_thumbnail {
        push(&mut a, &["--write-thumbnail", "--convert-thumbnails", "jpg"]);
    }
    if let Some((start, end)) = job.opts.clip {
        a.push("--download-sections".into());
        let end = if end > 0.0 { end.to_string() } else { "inf".into() };
        a.push(format!("*{start}-{end}").into());
        if has_ffmpeg {
            a.push("--force-keyframes-at-cuts".into());
        }
    }
    for extra in util::split_args(&s.extra_args) {
        a.push(extra.into());
    }
    a.push("--".into());
    a.push(job.url.clone().into());
    a
}

/// Fetches only a video's captions (for its transcript) into `out` as
/// `t.<lang>.vtt`. The video itself isn't downloaded again.
pub fn caption_args(url: &str, s: &Settings, p: &EnginePaths, out: &Path, langs: &str) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    common_args(s, p, &mut a);
    push(
        &mut a,
        &[
            "--skip-download",
            "--no-playlist",
            "--ignore-no-formats-error",
            "--write-subs",
            "--write-auto-subs",
            "--sub-langs",
            langs,
            "--sub-format",
            "vtt/srt/best",
        ],
    );
    if p.ffmpeg.is_some() {
        push(&mut a, &["--convert-subs", "vtt"]);
    }
    a.push("--paths".into());
    a.push(out.as_os_str().to_owned());
    push(&mut a, &["-o", "t.%(ext)s", "--"]);
    a.push(url.into());
    a
}

// --------------------------------------------------------------------------
// Progress lines
// --------------------------------------------------------------------------

pub enum Line {
    /// A video is about to download: its details and its parts.
    Item(Value, Vec<Value>),
    Progress(Value, Value),
    Post(Value),
    File(Value),
    Other(String),
}

pub fn parse_line(line: &str) -> Line {
    let mut parts = line.splitn(3, '\t');
    let tag = parts.next().unwrap_or("");
    let a = parts.next().and_then(|s| serde_json::from_str::<Value>(s).ok());
    let b = parts.next().and_then(|s| serde_json::from_str::<Value>(s).ok());
    match (tag, a) {
        ("SNAG:I", Some(info)) => {
            let fmts = b.and_then(|v| v.as_array().cloned()).unwrap_or_default();
            Line::Item(info, fmts)
        }
        ("SNAG:P", Some(p)) => Line::Progress(p, b.unwrap_or(Value::Null)),
        ("SNAG:PP", Some(p)) => Line::Post(p),
        ("SNAG:F", Some(f)) => Line::File(f),
        _ => Line::Other(line.to_string()),
    }
}

pub fn phase_label(postprocessor: &str) -> &'static str {
    match postprocessor {
        p if p.contains("Merger") => "Joining video and audio",
        p if p.contains("ExtractAudio") => "Converting audio",
        p if p.contains("EmbedThumbnail") => "Adding cover art",
        p if p.contains("Metadata") => "Writing tags",
        p if p.contains("SponsorBlock") || p.contains("ModifyChapters") => "Cutting sponsor segments",
        p if p.contains("EmbedSubtitle") => "Adding subtitles",
        p if p.contains("SubtitlesConvertor") => "Converting subtitles",
        p if p.contains("ThumbnailsConvertor") => "Converting thumbnail",
        p if p.contains("VideoRemuxer") || p.contains("VideoConvertor") => "Converting video",
        p if p.contains("MoveFiles") => "Saving",
        _ => "Finishing up",
    }
}

// --------------------------------------------------------------------------
// Preview: what's behind a link
// --------------------------------------------------------------------------

/// One video in a playlist, a channel or a search.
#[derive(Clone, Debug, Default)]
pub struct Entry {
    pub index: u32,
    pub title: String,
    pub duration: u64,
    pub url: String,
    pub uploader: String,
    pub views: Option<u64>,
    pub thumb: String,
}

impl Entry {
    fn from_flat(e: &Value, index: u32) -> Entry {
        let s = |keys: &[&str]| {
            keys.iter().find_map(|k| e[*k].as_str().filter(|v| !v.is_empty()).map(str::to_string)).unwrap_or_default()
        };
        Entry {
            index: e["playlist_index"].as_u64().map(|n| n as u32).unwrap_or(index),
            title: s(&["title", "id", "url"]),
            duration: e["duration"].as_f64().unwrap_or(0.0) as u64,
            url: s(&["webpage_url", "url"]),
            uploader: s(&["channel", "uploader", "uploader_id"]),
            views: e["view_count"].as_u64(),
            thumb: small_thumb(e),
        }
    }
}

/// A list-sized thumbnail (yt-dlp lists them smallest first).
fn small_thumb(e: &Value) -> String {
    let list: Vec<&Value> = e["thumbnails"]
        .as_array()
        .map(|l| l.iter().filter(|t| t["url"].as_str().is_some_and(|u| u.starts_with("http"))).collect())
        .unwrap_or_default();
    list.iter()
        .find(|t| t["width"].as_u64().is_some_and(|w| w >= 200))
        .or(list.last())
        .and_then(|t| t["url"].as_str())
        .or_else(|| e["thumbnail"].as_str())
        .unwrap_or("")
        .to_string()
}

#[derive(Clone, Debug)]
pub struct Choice {
    pub quality: Quality,
    pub note: String,
    /// Estimated size: any codec (MKV, Original), and preferring H.264 (MP4).
    pub size: Option<u64>,
    pub size_mp4: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub url: String,
    pub vid: String,
    pub title: String,
    pub uploader: String,
    pub duration: u64,
    pub site: String,
    pub thumb: String,
    pub is_playlist: bool,
    pub count: u32,
    pub entries: Vec<Entry>,
    pub choices: Vec<Choice>,
    pub audio_only: bool,
    /// Highest resolution on offer (0 when unknown).
    pub top: u32,
    pub is_live: bool,
}

/// Runs `yt-dlp -J` on a link. `ctl` lets the UI abandon it.
pub fn probe(
    paths: &EnginePaths,
    s: &Settings,
    url: &str,
    mode: PlaylistMode,
    ctl: &Arc<Control>,
) -> Result<Probe, String> {
    let mut args: Vec<OsString> = Vec::new();
    common_args(s, paths, &mut args);
    push(&mut args, &["-J", "--flat-playlist", "--no-warnings"]);
    match mode {
        PlaylistMode::Single | PlaylistMode::Auto => args.push("--no-playlist".into()),
        PlaylistMode::Whole => args.push("--yes-playlist".into()),
    }
    args.push("--".into());
    args.push(url.into());
    let info = run_json(paths, s, args, ctl)?;
    Ok(read_probe(&info, url, s))
}

/// Where a search runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchSite {
    #[default]
    YouTube,
    SoundCloud,
}

impl SearchSite {
    pub const ALL: [SearchSite; 2] = [SearchSite::YouTube, SearchSite::SoundCloud];

    pub fn label(self) -> &'static str {
        match self {
            SearchSite::YouTube => "YouTube",
            SearchSite::SoundCloud => "SoundCloud",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            SearchSite::YouTube => "ytsearch",
            SearchSite::SoundCloud => "scsearch",
        }
    }
}

/// The top `count` results for `query`.
pub fn search(
    paths: &EnginePaths,
    s: &Settings,
    query: &str,
    site: SearchSite,
    count: u32,
    ctl: &Arc<Control>,
) -> Result<Vec<Entry>, String> {
    let mut args: Vec<OsString> = Vec::new();
    common_args(s, paths, &mut args);
    push(&mut args, &["-J", "--flat-playlist", "--no-warnings", "--"]);
    args.push(format!("{}{count}:{}", site.prefix(), query.trim()).into());
    let info = run_json(paths, s, args, ctl)?;
    let entries = info["entries"].as_array().cloned().unwrap_or_default();
    Ok(entries
        .iter()
        .filter(|e| !e.is_null())
        .enumerate()
        .map(|(i, e)| Entry::from_flat(e, i as u32 + 1))
        .filter(|e| e.url.starts_with("http"))
        .collect())
}

/// Runs yt-dlp with `-J`-style arguments and parses what it prints.
fn run_json(paths: &EnginePaths, s: &Settings, args: Vec<OsString>, ctl: &Arc<Control>) -> Result<Value, String> {
    let mut cmd = command(paths);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child =
        cmd.spawn().map_err(|e| format!("Couldn't start yt-dlp ({e}). Reinstall it from Settings → Engine."))?;
    let group = ProcGroup::new();
    if let Some(g) = &group {
        g.assign(&child);
    }
    *ctl.group.lock().unwrap_or_else(|p| p.into_inner()) = group;

    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf);
        buf
    });

    // Give up after two minutes; some sites hang instead of failing.
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if started.elapsed() > Duration::from_secs(120) => {
                ctl.request(STOP_CANCEL);
                let _ = child.kill();
                break None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    if ctl.stop() == STOP_CANCEL {
        return Err(if status.is_none() {
            "The site took too long to answer. Try again.".into()
        } else {
            String::new()
        });
    }
    if !status.is_some_and(|s| s.success()) {
        let line =
            err.lines().rev().find(|l| l.starts_with("ERROR")).unwrap_or_else(|| err.lines().last().unwrap_or(""));
        return Err(util::friendly_error(line, &s.cookies_browser));
    }
    serde_json::from_slice(&out).map_err(|_| "yt-dlp returned something Snagger couldn't read.".to_string())
}

fn read_probe(info: &Value, url: &str, s: &Settings) -> Probe {
    let mut info = info.clone();
    let mut is_playlist = info["_type"].as_str() == Some("playlist");
    let entries_raw: Vec<Value> = info["entries"].as_array().cloned().unwrap_or_default();
    // A "playlist" holding exactly one full video (an Instagram post, a tweet) is just that video.
    if is_playlist && entries_raw.len() == 1 && entries_raw[0].get("formats").is_some() {
        info = entries_raw[0].clone();
        is_playlist = false;
    }
    let str_of = |v: &Value, keys: &[&str]| {
        keys.iter().find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()).map(str::to_string)).unwrap_or_default()
    };

    let mut entries = Vec::new();
    if is_playlist {
        for (i, e) in entries_raw.iter().enumerate() {
            if e.is_null() {
                continue;
            }
            entries.push(Entry::from_flat(e, i as u32 + 1));
        }
    }
    let duration = if is_playlist { 0 } else { info["duration"].as_f64().unwrap_or(0.0) as u64 };
    let (choices, audio_only, top) = if is_playlist {
        (
            Quality::ALL
                .iter()
                .map(|&q| Choice { quality: q, note: String::new(), size: None, size_mp4: None })
                .collect(),
            false,
            0,
        )
    } else {
        quality_choices(&info, duration)
    };
    let extractor = str_of(&info, &["extractor_key", "ie_key"]);
    let page = str_of(&info, &["webpage_url", "original_url"]);
    let page = if page.is_empty() { url.to_string() } else { page };
    let _ = s;
    Probe {
        vid: format!("{}:{}", extractor.to_ascii_lowercase(), str_of(&info, &["id"])),
        title: {
            let t = str_of(&info, &["title", "fulltitle", "id"]);
            if t.is_empty() { "Untitled".into() } else { t }
        },
        uploader: str_of(&info, &["uploader", "channel", "uploader_id", "playlist_uploader"]),
        site: util::site_name(&extractor, &page),
        thumb: pick_thumb(&info),
        count: if is_playlist {
            info["playlist_count"].as_u64().map(|n| n as u32).unwrap_or(entries.len() as u32)
        } else {
            0
        },
        url: page,
        is_playlist,
        entries,
        choices,
        audio_only,
        top,
        duration,
        is_live: info["is_live"].as_bool().unwrap_or(false),
    }
}

pub fn pick_thumb(info: &Value) -> String {
    if let Some(t) = info["thumbnail"].as_str().filter(|t| t.starts_with("http")) {
        return t.to_string();
    }
    if let Some(list) = info["thumbnails"].as_array() {
        // yt-dlp sorts thumbnails worst-to-best; a mid-size one is plenty.
        let urls: Vec<&str> = list.iter().filter_map(|t| t["url"].as_str()).filter(|u| u.starts_with("http")).collect();
        if let Some(u) = urls.last() {
            return u.to_string();
        }
    }
    if let Some(entries) = info["entries"].as_array() {
        for e in entries {
            let t = pick_thumb(e);
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

struct Fmt {
    res: u32,
    fps: f64,
    tbr: f64,
    size: Option<f64>,
    has_audio: bool,
    h264: bool,
}

/// Quality buttons for one video, each with a size estimate.
/// "Resolution" is the smaller side, so a 1080×1920 TikTok counts as 1080p.
fn quality_choices(info: &Value, duration: u64) -> (Vec<Choice>, bool, u32) {
    let fmts: Vec<Value> = match info["formats"].as_array() {
        Some(f) if !f.is_empty() => f.clone(),
        _ => vec![info.clone()],
    };
    let size_of = |f: &Value| {
        f["filesize"].as_f64().or(f["filesize_approx"].as_f64()).or_else(|| {
            let tbr = f["tbr"].as_f64()?;
            (duration > 0).then(|| tbr * 1000.0 / 8.0 * duration as f64)
        })
    };
    let mut videos = Vec::new();
    let mut best_audio: Option<(f64, Option<f64>)> = None;
    for f in &fmts {
        let vcodec = f["vcodec"].as_str().unwrap_or("");
        let acodec = f["acodec"].as_str().unwrap_or("");
        if f["format_note"].as_str().is_some_and(|n| n.contains("storyboard")) || f["ext"].as_str() == Some("mhtml") {
            continue;
        }
        if vcodec == "none" {
            let abr = f["abr"].as_f64().or(f["tbr"].as_f64()).unwrap_or(0.0);
            if best_audio.is_none_or(|(b, _)| abr > b) {
                best_audio = Some((abr, size_of(f)));
            }
            continue;
        }
        let (w, h) = (f["width"].as_u64(), f["height"].as_u64());
        let res = match (w, h) {
            (Some(w), Some(h)) => w.min(h),
            (None, Some(h)) => h,
            _ => 0,
        } as u32;
        videos.push(Fmt {
            res,
            fps: f["fps"].as_f64().unwrap_or(0.0),
            tbr: f["tbr"].as_f64().unwrap_or(0.0),
            size: size_of(f),
            has_audio: !acodec.is_empty() && acodec != "none",
            h264: vcodec.starts_with("avc") || vcodec.starts_with("h264"),
        });
    }
    let audio_size = best_audio.and_then(|(_, s)| s);
    if videos.is_empty() {
        let size = audio_size.map(|s| s as u64);
        let choices = vec![Choice { quality: Quality::Audio, note: "audio".into(), size, size_mp4: size }];
        return (choices, true, 0);
    }

    // The same order yt-dlp sorts by: resolution, then frame rate, then
    // (for MP4) H.264 over newer codecs, then bitrate.
    let estimate = |cap: Option<u32>, prefer_h264: bool| -> (Option<u64>, u32, f64) {
        let key = |v: &&Fmt| (v.res, v.fps.round() as u32, prefer_h264 && v.h264, v.tbr as u64);
        let pick = videos
            .iter()
            .filter(|v| cap.is_none_or(|c| v.res <= c + c / 10) || v.res == 0)
            .max_by(|a, b| key(a).cmp(&key(b)));
        match pick {
            Some(v) => {
                let total = v.size.map(|s| s + if v.has_audio { 0.0 } else { audio_size.unwrap_or(0.0) });
                (total.map(|t| t as u64), v.res, v.fps)
            }
            None => (None, 0, 0.0),
        }
    };

    let resolutions: Vec<u32> = videos.iter().map(|v| v.res).filter(|&r| r > 0).collect();
    let top = resolutions.iter().copied().max().unwrap_or(0);
    let mut choices = Vec::new();
    let (size, res, fps) = estimate(None, false);
    let (size_mp4, _, _) = estimate(None, true);
    let mut note = match res {
        0 => String::new(),
        2160.. => "4K".into(),
        r => format!("{r}p"),
    };
    if fps >= 49.0 && !note.is_empty() {
        note.push_str(&format!("{}", fps.round() as u32));
    }
    choices.push(Choice { quality: Quality::Best, note, size, size_mp4 });
    // A rung is offered when it downloads something different from the
    // rung above. Widescreen films (2048×858) have no exact 720 or 480, but
    // "Up to 720p" still picks their 1280×536 stream, as yt-dlp's res sort does.
    let mut last_res = res;
    for rung in [2160u32, 1440, 1080, 720, 480, 360] {
        if top > 0 && rung < top {
            let (size, picked, _) = estimate(Some(rung), false);
            if picked > 0 && picked != last_res {
                let (size_mp4, _, _) = estimate(Some(rung), true);
                choices.push(Choice { quality: Quality::Max(rung), note: String::new(), size, size_mp4 });
                last_res = picked;
            }
        }
        if choices.len() == 5 {
            break;
        }
    }
    let audio = audio_size.map(|s| s as u64);
    choices.push(Choice { quality: Quality::Audio, note: String::new(), size: audio, size_mp4: audio });
    (choices, false, top)
}

/// A channel's or playlist's name, without listing all of it.
pub fn playlist_title(paths: &EnginePaths, s: &Settings, url: &str) -> Result<(String, String), String> {
    let mut args: Vec<OsString> = Vec::new();
    common_args(s, paths, &mut args);
    push(&mut args, &["-J", "--flat-playlist", "--playlist-end", "3", "--no-warnings", "--yes-playlist", "--"]);
    args.push(url.into());
    let mut cmd = command(paths);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().map_err(|e| format!("Couldn't start yt-dlp ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| l.starts_with("ERROR")).unwrap_or("");
        return Err(util::friendly_error(line, &s.cookies_browser));
    }
    let info: Value = serde_json::from_slice(&out.stdout)
        .map_err(|_| "yt-dlp returned something Snagger couldn't read.".to_string())?;
    if info["_type"].as_str() != Some("playlist") {
        return Err("That's a single video. Follow a channel or playlist link instead.".into());
    }
    let title = ["title", "channel", "uploader", "id"]
        .iter()
        .find_map(|k| info[*k].as_str().filter(|t| !t.is_empty()))
        .unwrap_or("Subscription")
        .to_string();
    Ok((title, pick_thumb(&info)))
}

/// Marks everything currently in a channel or playlist as "seen", so a new
/// subscription only downloads videos posted from now on.
pub fn mark_existing(paths: &EnginePaths, s: &Settings, url: &str, archive: &Path) -> Result<(u32, String), String> {
    let mut args: Vec<OsString> = Vec::new();
    common_args(s, paths, &mut args);
    push(&mut args, &["--flat-playlist", "--simulate", "--force-write-archive", "--no-warnings"]);
    push(&mut args, &["--print", "playlist:SNAG:T\t%(title)j"]);
    args.push("--download-archive".into());
    args.push(archive.as_os_str().to_owned());
    args.push("--".into());
    args.push(url.into());
    let lines = || std::fs::read_to_string(archive).map(|t| t.lines().count() as u32).unwrap_or(0);
    let before = lines();
    let mut cmd = command(paths);
    cmd.args(&args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().map_err(|e| format!("Couldn't start yt-dlp ({e})"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let title = stdout
        .lines()
        .find_map(|l| l.strip_prefix("SNAG:T\t"))
        .and_then(|t| serde_json::from_str::<String>(t).ok())
        .unwrap_or_default();
    if !out.status.success() && title.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err.lines().rev().find(|l| l.starts_with("ERROR")).unwrap_or("");
        return Err(util::friendly_error(line, &s.cookies_browser));
    }
    Ok((lines().saturating_sub(before), title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::JobOptions;

    /// The longest path a template can produce: every `.NB` field at its cap.
    fn worst_case(tmpl: &str) -> usize {
        let mut out = 0;
        let mut rest = tmpl;
        while let Some(start) = rest.find("%(") {
            out += rest[..start].len();
            let end = start + rest[start..].find(')').unwrap() + 1;
            let field = &rest[start..end];
            let after = &rest[end..];
            let spec_len = after.find(|c: char| c.is_ascii_alphabetic()).unwrap() + 1;
            let spec = &after[..spec_len];
            out += if let Some(n) = spec.strip_prefix('.').and_then(|s| s.strip_suffix('B')) {
                n.parse::<usize>().unwrap()
            } else if field.contains("playlist_index") {
                7
            } else if field.contains("(ext") {
                4 + 28
            } else if field.contains("(id") {
                20
            } else if field.contains("upload_date") {
                10
            } else {
                12
            };
            rest = &after[spec_len..];
        }
        out + rest.len()
    }

    #[test]
    fn widescreen_films_get_a_full_quality_list() {
        let f = |w: u32, h: u32, tbr: f64| serde_json::json!({"width": w, "height": h, "vcodec": "avc1", "acodec": "none", "tbr": tbr});
        let info = serde_json::json!({"formats": [
            f(640, 268, 300.0), f(854, 358, 600.0), f(1280, 536, 1500.0), f(2048, 858, 4000.0),
            {"vcodec": "none", "acodec": "mp4a", "abr": 128.0, "tbr": 128.0},
        ]});
        let (choices, audio_only, top) = quality_choices(&info, 600);
        let qs: Vec<Quality> = choices.iter().map(|c| c.quality).collect();
        assert!(!audio_only);
        assert_eq!(top, 858);
        assert_eq!(qs, vec![Quality::Best, Quality::Max(720), Quality::Max(480), Quality::Audio]);
    }

    #[test]
    fn templates_fit_windows_paths() {
        let mut s = Settings::default();
        for organize in [Organize::None, Organize::BySite, Organize::ByUploader] {
            for style in [NameStyle::TitleId, NameStyle::UploaderTitle, NameStyle::DateTitle] {
                s.organize = organize;
                s.name_style = style;
                for base in [
                    r"C:\Users\Someone\Downloads\Snagger",
                    r"D:\A Rather Long Folder Name\With Some Nesting\And More Nesting Here\Snagger",
                ] {
                    let staging = Path::new(base).join(".snag-temp").join("abcdef123456");
                    let job = Job::new("https://example.com/v", JobOptions::default());
                    let t = output_template(&job, &s, &staging);
                    let work = staging.join("work").to_string_lossy().len() + 1;
                    assert!(work + worst_case(&t) <= 260, "{base} {t}: {}", work + worst_case(&t));
                }
            }
        }
    }
}
