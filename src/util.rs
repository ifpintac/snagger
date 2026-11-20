//! Small helpers: links, names, numbers, times and error wording.

use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 64 random bits. `RandomState` is seeded from the OS, so this is fine for ids
/// and for the token that guards the single-instance socket.
pub fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos());
    h.finish()
}

pub fn new_id() -> String {
    format!("{:012x}", random_u64() & 0xffff_ffff_ffff)
}

// --------------------------------------------------------------------------
// Links
// --------------------------------------------------------------------------

/// Every http(s) link in a block of text, in order, without duplicates.
/// Handles one-per-line lists, links inside sentences, Markdown and HTML.
pub fn extract_links(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let separators =
        |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`' | '|' | '{' | '}' | '\\' | '^');
    for token in text.split(separators) {
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        // A token can hold several links glued together, or text before the link.
        let mut starts: Vec<usize> = Vec::new();
        for pat in ["https://", "http://"] {
            let mut from = 0;
            while let Some(i) = lower[from..].find(pat) {
                starts.push(from + i);
                from += i + pat.len();
            }
        }
        starts.sort_unstable();
        starts.dedup();
        let mut candidates = Vec::new();
        if starts.is_empty() {
            if lower.starts_with("www.") {
                candidates.push(format!("https://{token}"));
            }
        } else {
            for (n, &s) in starts.iter().enumerate() {
                let end = starts.get(n + 1).copied().unwrap_or(token.len());
                candidates.push(token[s..end].to_string());
            }
        }
        for c in candidates {
            let url = trim_url(&c);
            if is_plausible_url(&url) && seen.insert(url.clone()) {
                out.push(url);
            }
        }
    }
    out
}

/// What the single link box accepts: a link anywhere in the text, or a bare
/// address like `youtu.be/abc`.
pub fn parse_single_link(text: &str) -> Option<String> {
    if let Some(first) = extract_links(text).into_iter().next() {
        return Some(first);
    }
    let t = text.trim();
    let host = t.split('/').next().unwrap_or("");
    let looks_like_host = host.contains('.')
        && !host.starts_with('.')
        && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if looks_like_host && !t.contains(char::is_whitespace) {
        let url = format!("https://{t}");
        if is_plausible_url(&url) {
            return Some(url);
        }
    }
    None
}

fn trim_url(s: &str) -> String {
    let mut s = s.to_string();
    while let Some(last) = s.chars().last() {
        let opens = |o: char| s.matches(o).count();
        let strip = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => true,
            ')' => opens('(') < opens(')'),
            ']' => opens('[') < opens(']'),
            _ => false,
        };
        if !strip {
            break;
        }
        s.pop();
    }
    s
}

fn is_plausible_url(url: &str) -> bool {
    if url.len() > 4000 {
        return false;
    }
    let Some(rest) = url.split_once("://").map(|(_, r)| r) else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    host.contains('.') && host.len() > 3 && !host.ends_with('.') && !host.starts_with('.')
}

/// Links from an "Internet Shortcut" (.url) file.
pub fn links_from_url_file(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("URL=").map(|u| u.trim().to_string()))
        .filter(|u| u.starts_with("http"))
        .collect();
    out.dedup();
    out
}

pub fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

/// "youtube.com/watch?v=…" for showing a link in a narrow space.
pub fn short_url(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    let rest = rest.trim_end_matches('/');
    ellipsize(rest, 70)
}

pub fn ellipsize(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Sites the clipboard watcher reacts to. Anything yt-dlp supports can still be
/// pasted by hand; this list only avoids offering every link you copy.
const MEDIA_HOSTS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "music.youtube.com",
    "tiktok.com",
    "instagram.com",
    "facebook.com",
    "fb.watch",
    "x.com",
    "twitter.com",
    "reddit.com",
    "redd.it",
    "vimeo.com",
    "twitch.tv",
    "soundcloud.com",
    "dailymotion.com",
    "dai.ly",
    "bilibili.com",
    "b23.tv",
    "threads.net",
    "threads.com",
    "bsky.app",
    "tumblr.com",
    "pinterest.com",
    "pin.it",
    "streamable.com",
    "kick.com",
    "rumble.com",
    "bandcamp.com",
    "mixcloud.com",
    "odysee.com",
    "nicovideo.jp",
    "vk.com",
    "ok.ru",
    "bitchute.com",
    "linkedin.com",
    "9gag.com",
    "imgur.com",
    "archive.org",
    "ted.com",
    "loom.com",
    "snapchat.com",
    "likee.video",
    "coub.com",
    "veoh.com",
    "peertube.tv",
];

pub fn is_media_link(url: &str) -> bool {
    let host = host_of(url);
    MEDIA_HOSTS.iter().any(|h| host == *h || host.ends_with(&format!(".{h}")))
}

/// A watch link that also names a playlist, e.g. `watch?v=…&list=…`.
pub fn video_in_playlist(url: &str) -> bool {
    let low = url.to_ascii_lowercase();
    (low.contains("list=") && (low.contains("v=") || low.contains("youtu.be/")))
        || (low.contains("/sets/") && low.contains("in="))
}

/// Channel pages, where newest videos come first.
pub fn looks_like_channel(url: &str) -> bool {
    let low = url.to_ascii_lowercase();
    ["/@", "/channel/", "/c/", "/user/"].iter().any(|p| low.contains(p))
}

/// Replaces the `list=` part so yt-dlp sees a plain playlist link.
pub fn playlist_url_of(url: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    let list = q.split('&').find_map(|kv| kv.strip_prefix("list="))?;
    if list.is_empty() {
        return None;
    }
    let host = host_of(url);
    if host.contains("youtube.com") || host == "youtu.be" {
        Some(format!("https://www.youtube.com/playlist?list={list}"))
    } else {
        None
    }
}

// --------------------------------------------------------------------------
// Names and paths
// --------------------------------------------------------------------------

/// A safe folder or file name for Windows.
pub fn safe_name(s: &str, max_chars: usize) -> String {
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    out = out.chars().take(max_chars).collect();
    let out = out.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "LPT1", "LPT2"];
    if out.is_empty() {
        "Untitled".into()
    } else if reserved.contains(&out.to_ascii_uppercase().as_str()) {
        format!("{out}_")
    } else {
        out
    }
}

/// `name (2).ext`, `name (3).ext`… until the path is free.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for n in 2..10_000 {
        let candidate = parent.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

/// Splits "extra options" text the way a shell would, honouring quotes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                has = true;
            }
            None if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            None => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

// --------------------------------------------------------------------------
// Numbers and times
// --------------------------------------------------------------------------

pub fn fmt_bytes(n: f64) -> String {
    if n <= 0.0 {
        return "0 B".into();
    }
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 || v >= 100.0 { format!("{v:.0} {}", units[i]) } else { format!("{v:.1} {}", units[i]) }
}

pub fn fmt_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

pub fn fmt_eta(secs: u64) -> String {
    if secs >= 3600 {
        format!("{} h {} min left", secs / 3600, secs % 3600 / 60)
    } else if secs >= 60 {
        format!("{} min left", secs.div_ceil(60))
    } else {
        format!("{secs} s left")
    }
}

pub fn fmt_ago(ts: u64) -> String {
    if ts == 0 {
        return "never".into();
    }
    let d = now().saturating_sub(ts);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=172_799 => "yesterday".into(),
        _ => fmt_date(ts),
    }
}

pub fn fmt_date(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(ts as i64, 0).single() {
        Some(t) => t.format("%-d %b %Y, %H:%M").to_string(),
        None => String::new(),
    }
}

/// "90", "1:30", "1:02:03", "1:30.5" → seconds.
pub fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = 0.0;
    for part in s.split(':') {
        let v: f64 = part.trim().parse().ok()?;
        if v < 0.0 {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

pub fn fmt_timestamp(secs: f64) -> String {
    let whole = secs.floor() as u64;
    let frac = secs - whole as f64;
    let base = fmt_duration(whole);
    if frac > 0.05 { format!("{base}.{}", (frac * 10.0).round() as u64 % 10) } else { base }
}

/// "1m30s" for file names.
pub fn clip_tag(secs: f64) -> String {
    let s = secs.round() as u64;
    let (h, m, sec) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 { format!("{h}h{m:02}m{sec:02}s") } else { format!("{m}m{sec:02}s") }
}

// --------------------------------------------------------------------------
// Sites and errors
// --------------------------------------------------------------------------

const SITE_NAMES: &[(&str, &str)] = &[
    ("youtube", "YouTube"),
    ("tiktok", "TikTok"),
    ("instagram", "Instagram"),
    ("facebook", "Facebook"),
    ("reddit", "Reddit"),
    ("twitter", "X"),
    ("vimeo", "Vimeo"),
    ("twitch", "Twitch"),
    ("soundcloud", "SoundCloud"),
    ("dailymotion", "Dailymotion"),
    ("threads", "Threads"),
    ("pinterest", "Pinterest"),
    ("bluesky", "Bluesky"),
    ("bilibili", "Bilibili"),
    ("linkedin", "LinkedIn"),
    ("tumblr", "Tumblr"),
    ("bandcamp", "Bandcamp"),
    ("kick", "Kick"),
    ("rumble", "Rumble"),
    ("streamable", "Streamable"),
];

pub fn site_name(extractor_key: &str, url: &str) -> String {
    let key = extractor_key.to_ascii_lowercase();
    if !key.is_empty()
        && key != "generic"
        && let Some((_, name)) = SITE_NAMES.iter().find(|(p, _)| key.starts_with(p))
    {
        return name.to_string();
    }
    let host = host_of(url);
    if let Some((_, name)) = SITE_NAMES.iter().find(|(p, _)| host.contains(p)) {
        return name.to_string();
    }
    if host == "youtu.be" {
        return "YouTube".into();
    }
    if host == "x.com" {
        return "X".into();
    }
    if !host.is_empty() { host } else { extractor_key.to_string() }
}

/// yt-dlp's message without its prefixes, colour codes or command-line advice.
pub fn clean_message(msg: &str) -> String {
    let mut m = strip_ansi(msg).trim().to_string();
    for p in ["ERROR:", "WARNING:"] {
        if let Some(rest) = m.strip_prefix(p) {
            m = rest.trim_start().to_string();
        }
    }
    // "[youtube] abc123: " → ""
    if m.starts_with('[')
        && let Some(end) = m.find("] ")
    {
        m = m[end + 2..].to_string();
        if let Some(colon) = m.find(": ") {
            let id = &m[..colon];
            if !id.is_empty() && id.len() <= 64 && !id.contains(' ') {
                m = m[colon + 2..].to_string();
            }
        }
    }
    m
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Turns yt-dlp's command-line wording into something useful in the app.
pub fn friendly_error(err: &str, browser: &str) -> String {
    let mut msg = clean_message(err);
    let low = msg.to_lowercase();
    for cut in [" Use --", " See https://", " See http://", " Also see", " please report", " Please report"] {
        if let Some(i) = msg.find(cut) {
            msg.truncate(i);
        }
    }
    if let Some(i) = msg.find(" (caused by") {
        msg.truncate(i);
    }
    let msg = msg.trim().trim_end_matches([',', ';']).to_string();
    let signin = "If you can watch it while signed in, pick that browser under Settings → Sign-in and try again.";

    if low.contains("unsupported url") {
        return "Snagger can't find a video at that link. Open the video itself (not a profile or search page) and copy its address.".into();
    }
    if (low.contains("could not copy") && low.contains("cookie"))
        || low.contains("failed to decrypt")
        || (low.contains("could not find") && low.contains("cookies database"))
    {
        let name = if browser.is_empty() { "that browser".to_string() } else { capitalize(browser) };
        return format!(
            "Couldn't read {name}'s sign-in. Close {name} completely and try again. Firefox works most reliably for this."
        );
    }
    if [
        "getaddrinfo",
        "name or service not known",
        "timed out",
        "network is unreachable",
        "connection refused",
        "failed to establish",
        "nodename nor servname",
        "no address associated",
    ]
    .iter()
    .any(|s| low.contains(s))
    {
        return "Couldn't reach the site. Check your internet connection and try again.".into();
    }
    if low.contains("http error 404") {
        return "That page doesn't exist (404). Check the link and try again.".into();
    }
    if low.contains("http error 403") || low.contains("http 403") {
        return REFUSED.into();
    }
    if low.contains("ffmpeg") && (low.contains("not found") || low.contains("not installed")) {
        return "This download needs ffmpeg. Open Settings → Engine and reinstall ffmpeg.".into();
    }
    if low.starts_with("postprocessing") || low.contains("conversion failed") {
        return "The download finished, but ffmpeg couldn't put the file together. Try again, or pick a different quality or format.".into();
    }
    if low.contains("requested format is not available") {
        return "That quality or format isn't available for this video. Pick another and try again.".into();
    }
    if low.contains("no space left") || low.contains("disk full") || low.contains("errno 28") {
        return "The disk is full. Free up space, then resume (the part downloaded so far is kept).".into();
    }
    if low.contains("drm") {
        return "This video is DRM-protected, so it can't be downloaded.".into();
    }
    let auth_words = [
        "sign in",
        "log in",
        "login",
        "cookies",
        "private",
        "rate-limit",
        "rate limit",
        "age-restricted",
        "age restricted",
        "inappropriate",
        "members",
        "not a bot",
        "requested content is not available",
    ];
    if auth_words.iter().any(|w| low.contains(w)) {
        return if browser.is_empty() {
            format!("{msg} {signin}")
        } else {
            format!("{msg} If this keeps happening, make sure you're signed in to the site in {}.", capitalize(browser))
        };
    }
    if msg.is_empty() { "Something went wrong while downloading.".into() } else { ellipsize(&msg, 400) }
}

/// The site refused the download even after fresh links. Mentions "HTTP 403"
/// so [`refused`] and [`engine_problem`] still recognise it.
pub const REFUSED: &str = "The site refused the download (HTTP 403), even with fresh links. Update yt-dlp (the refresh button on this row does it and tries again), or pick your browser under Settings → Sign-in.";

/// The site said "403 Forbidden". With YouTube that is usually one stream
/// link going bad, and fresh links (a new run) fix it.
pub fn refused(error: &str) -> bool {
    let low = error.to_lowercase();
    low.contains("http error 403") || low.contains("http 403") || low.contains("403 forbidden")
}

/// How long to wait before fetching fresh links after a refusal, or None
/// once that has been tried twice.
pub fn refusal_delay(refusals_so_far: u32) -> Option<u64> {
    [5, 30].get(refusals_so_far as usize).copied()
}

/// The disk filled up, or would have.
pub fn disk_full(error: &str) -> bool {
    let low = error.to_lowercase();
    low.contains("disk is full") || low.contains("not enough disk space")
}

/// Bytes a download needs on disk: the file, plus as much again when video
/// and audio are joined afterwards (both parts sit on disk while the joined
/// file is written), plus a little room.
pub fn space_needed(size: u64, joins: bool) -> u64 {
    size.saturating_mul(if joins { 2 } else { 1 }) + 64 * 1024 * 1024
}

/// "C:" for a path on drive C, or "the download drive".
pub fn drive_name(path: &Path) -> String {
    match path.components().next() {
        Some(std::path::Component::Prefix(p)) => p.as_os_str().to_string_lossy().trim_end_matches('\\').to_string(),
        _ => "the download drive".into(),
    }
}

/// The message for a download that won't fit.
pub fn not_enough_space(needed: u64, free: u64, dir: &Path) -> String {
    format!(
        "Not enough disk space: this needs about {} and {} has {} free. Free up space, then resume.",
        fmt_bytes(needed as f64),
        drive_name(dir),
        fmt_bytes(free as f64)
    )
}

/// True when the registered `snag:` command runs a copy that's gone, or the
/// old Snag.exe, rather than `exe`.
pub fn button_needs_relink(command: &str, exe: &Path) -> bool {
    let registered = command.trim_start().strip_prefix('"').and_then(|r| r.split('"').next()).unwrap_or("");
    if registered.is_empty() || registered.eq_ignore_ascii_case(&exe.display().to_string()) {
        return false;
    }
    let path = Path::new(registered);
    let old_name = path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("Snag.exe"));
    old_name || !path.exists()
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// How long to wait before trying a failed download again by itself, or
/// None when trying again won't help (a private video, a wrong link).
pub fn retry_delay(error: &str, attempts_so_far: u32) -> Option<u64> {
    let attempt = attempts_so_far as usize;
    if attempt >= 3 {
        return None;
    }
    let low = error.to_lowercase();
    let rate_limited = ["429", "too many requests", "rate-limit", "rate limit"].iter().any(|p| low.contains(p));
    if rate_limited {
        return Some([300, 900, 1800][attempt]);
    }
    let passing = [
        "timed out",
        "timeout",
        "couldn't reach",
        "connection",
        "reset by peer",
        "remote end closed",
        "incompleteread",
        "incomplete read",
        "temporarily",
        "http error 500",
        "http error 502",
        "http error 503",
        "http error 504",
        "dropped",
        "cut short",
        "stopped responding",
        "unable to download webpage",
        "eof occurred",
    ];
    passing.iter().any(|p| low.contains(p)).then(|| [20, 90, 300][attempt])
}

/// Errors that usually mean the site changed and yt-dlp needs updating.
pub fn engine_problem(error: &str) -> bool {
    let low = error.to_lowercase();
    [
        "unable to extract",
        "failed to extract",
        "signature extraction",
        "nsig extraction",
        "could not find js function",
        "player response",
        "jsinterp",
        "challenge",
        "failed to parse json",
        // A refusal that fresh links didn't fix: a newer yt-dlp usually does.
        "http 403",
    ]
    .iter()
    .any(|p| low.contains(p))
}

/// "%20" → " ", "%C3%A9" → "é". Leaves broken escapes as they are.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Escapes what may not appear raw in an HTTP request line (spaces,
/// non-ASCII letters, quotes) and keeps everything else, including existing
/// %-escapes, as it is.
pub fn encode_url(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        if b.is_ascii_graphic() && !matches!(b, b'"' | b'<' | b'>' | b'\\' | b'^' | b'`' | b'{' | b'|' | b'}') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// What a `snag:` address carries: the link, and from the browser
/// extension also the page it was found on and that page's title.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnagLink {
    pub link: String,
    pub referer: String,
    pub title: String,
}

/// Reads `snag:<link>` (the browser button) or
/// `snag:<link>&referer=<page>&title=<title>` (the extension; every part is
/// percent-encoded, so a raw `&` only ever separates them).
pub fn parse_snag(arg: &str) -> Option<SnagLink> {
    let rest = arg.strip_prefix("snag:").or_else(|| arg.strip_prefix("SNAG:"))?;
    let rest = rest.trim_start_matches('/');
    let mut parts = rest.split('&');
    let first = parts.next().unwrap_or("");
    let decoded = percent_decode(first);
    let link = extract_links(decoded.trim_end_matches('/')).into_iter().next()?;
    let mut out = SnagLink { link, ..Default::default() };
    for part in parts {
        match part.split_once('=') {
            Some(("referer", v)) => out.referer = percent_decode(v).trim().to_string(),
            Some(("title", v)) => out.title = percent_decode(v).trim().to_string(),
            _ => {}
        }
    }
    Some(out)
}

/// The link inside a `snag:` address.
pub fn from_snag_scheme(arg: &str) -> Option<String> {
    parse_snag(arg).map(|s| s.link)
}

type LinkMeta = std::sync::Mutex<std::collections::HashMap<String, (String, String)>>;

fn link_meta() -> &'static LinkMeta {
    static META: std::sync::OnceLock<LinkMeta> = std::sync::OnceLock::new();
    META.get_or_init(Default::default)
}

/// Remembers the page a link was found on (sent as Referer, which many
/// video servers insist on) and a title for the file.
pub fn remember_link(link: &str, referer: &str, title: &str) {
    if referer.is_empty() && title.is_empty() {
        return;
    }
    let mut m = link_meta().lock().unwrap_or_else(|p| p.into_inner());
    let e = m.entry(link.to_string()).or_default();
    if !referer.is_empty() {
        e.0 = referer.to_string();
    }
    if !title.is_empty() {
        e.1 = title.to_string();
    }
}

pub fn link_referer(link: &str) -> Option<String> {
    let m = link_meta().lock().unwrap_or_else(|p| p.into_inner());
    m.get(link).map(|e| e.0.clone()).filter(|r| !r.is_empty())
}

pub fn link_title(link: &str) -> Option<String> {
    let m = link_meta().lock().unwrap_or_else(|p| p.into_inner());
    m.get(link).map(|e| e.1.clone()).filter(|t| !t.is_empty())
}

/// File types Snagger downloads itself, without yt-dlp, when a link ends in them.
const FILE_EXTS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "img", "dmg", "pkg", "exe", "msi", "msix",
    "apk", "appimage", "deb", "rpm", "pdf", "epub", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "csv", "psd", "psb",
    "ai", "eps", "indd", "sketch", "fig", "xd", "blend", "fbx", "obj", "stl", "step", "stp", "3mf", "glb", "gltf",
    "usdz", "dwg", "dxf", "ttf", "otf", "woff2", "jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "heic", "raw",
    "cr2", "nef", "arw", "dng", "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "mp3", "m4a", "wav", "flac", "ogg",
    "opus", "aac", "aif", "aiff", "bin", "dat",
];

/// A link to a file (ends in .zip, .pdf, .mp4…) rather than a page.
pub fn file_like_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let Some(after_host) = path.split_once("://").map(|(_, r)| r) else { return false };
    let Some((_, path)) = after_host.split_once('/') else { return false };
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && FILE_EXTS.contains(&ext.to_ascii_lowercase().as_str()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_what_can_pass() {
        assert_eq!(retry_delay("Couldn't reach the site. Check your internet connection", 0), Some(20));
        assert_eq!(retry_delay("HTTP Error 429: Too Many Requests", 1), Some(900));
        assert_eq!(retry_delay("Private video. Sign in", 0), None);
        assert_eq!(retry_delay("The connection dropped", 3), None);
        assert!(engine_problem("Unable to extract uploader id"));
        assert!(!engine_problem("Video unavailable"));
    }

    #[test]
    fn decodes_links_and_schemes() {
        assert_eq!(percent_decode("My%20File%C3%A9.zip"), "My Fileé.zip");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(encode_url("https://x.com/a b/é%20.zip"), "https://x.com/a%20b/%C3%A9%20.zip");
        assert_eq!(
            from_snag_scheme("snag:https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dabc").as_deref(),
            Some("https://www.youtube.com/watch?v=abc")
        );
        assert_eq!(from_snag_scheme("snag://https://vimeo.com/1/").as_deref(), Some("https://vimeo.com/1"));
    }

    #[test]
    fn spots_file_links() {
        assert!(file_like_url("https://cdn.example.com/a/Plans%20v2.ZIP?sig=1"));
        assert!(file_like_url("https://example.com/f/render.mp4"));
        assert!(!file_like_url("https://www.youtube.com/watch?v=abc"));
        assert!(!file_like_url("https://example.com/page.html"));
        assert!(!file_like_url("https://example.com/"));
    }

    #[test]
    fn finds_links_in_messy_text() {
        let t = "watch https://youtu.be/abc, and (https://example.com/a_(b)) \n\
                 [x](https://vimeo.com/123). www.reddit.com/r/x https://a.com/1https://b.com/2";
        assert_eq!(
            extract_links(t),
            vec![
                "https://youtu.be/abc",
                "https://example.com/a_(b)",
                "https://vimeo.com/123",
                "https://www.reddit.com/r/x",
                "https://a.com/1",
                "https://b.com/2",
            ]
        );
    }

    #[test]
    fn single_link_accepts_bare_hosts() {
        assert_eq!(parse_single_link("youtu.be/abc").as_deref(), Some("https://youtu.be/abc"));
        assert_eq!(parse_single_link("hello there"), None);
    }

    #[test]
    fn timestamps() {
        assert_eq!(parse_timestamp("1:30"), Some(90.0));
        assert_eq!(parse_timestamp("1:02:03"), Some(3723.0));
        assert_eq!(parse_timestamp("x"), None);
        assert_eq!(clip_tag(90.0), "1m30s");
    }

    #[test]
    fn args_split_like_a_shell() {
        assert_eq!(split_args(r#"--a "b c" 'd' e"#), vec!["--a", "b c", "d", "e"]);
    }

    #[test]
    fn cleans_ytdlp_errors() {
        assert_eq!(clean_message("ERROR: [youtube] abc123: Video unavailable"), "Video unavailable");
        assert!(friendly_error("ERROR: Unsupported URL: https://x", "").starts_with("Snagger can't"));
    }

    #[test]
    fn playlist_links() {
        assert!(video_in_playlist("https://www.youtube.com/watch?v=a&list=PL1"));
        assert_eq!(
            playlist_url_of("https://www.youtube.com/watch?v=a&list=PL1&index=2").as_deref(),
            Some("https://www.youtube.com/playlist?list=PL1")
        );
    }

    #[test]
    fn refusals_get_fresh_links_then_a_plain_message() {
        let raw = "ERROR: unable to download video data: HTTP Error 403: Forbidden";
        assert!(refused(raw));
        let msg = friendly_error(raw, "");
        assert_eq!(msg, REFUSED);
        assert!(refused(&msg), "the friendly text must still read as a refusal");
        assert!(engine_problem(&msg), "a lasting refusal offers the update-and-retry button");
        assert_eq!(refusal_delay(0), Some(5));
        assert_eq!(refusal_delay(1), Some(30));
        assert_eq!(refusal_delay(2), None);
        assert!(!refused("HTTP Error 404: Not Found"));
    }

    #[test]
    fn disk_space_messages() {
        let mb = 1024 * 1024;
        assert_eq!(space_needed(100 * mb, false), 164 * mb);
        assert_eq!(space_needed(100 * mb, true), 264 * mb);
        assert_eq!(drive_name(Path::new(r"C:\Users\me\Downloads")), "C:");
        let msg = not_enough_space(264 * mb, 100 * mb, Path::new(r"D:\Videos"));
        assert!(msg.contains("D:") && msg.contains("264 MB") && msg.contains("100 MB"), "{msg}");
        assert!(disk_full(&msg));
        assert!(disk_full(&friendly_error("ERROR: [Errno 28] No space left on device", "")));
        assert!(!disk_full("HTTP Error 403: Forbidden"));
    }

    #[test]
    fn extension_links_carry_their_page() {
        let enc = |s: &str| s.bytes().map(|b| format!("%{b:02X}")).collect::<String>();
        let arg = format!(
            "snag:{}&referer={}&title={}",
            enc("https://cdn.example.com/v/master.m3u8?sig=a&b=c"),
            enc("https://www.behance.net/gallery/1/Project"),
            enc("Paper Boats & more")
        );
        let s = parse_snag(&arg).unwrap();
        assert_eq!(s.link, "https://cdn.example.com/v/master.m3u8?sig=a&b=c");
        assert_eq!(s.referer, "https://www.behance.net/gallery/1/Project");
        assert_eq!(s.title, "Paper Boats & more");
        // The browser button's plain form still works.
        assert_eq!(
            parse_snag("snag:https%3A%2F%2Fvimeo.com%2F1").unwrap(),
            SnagLink { link: "https://vimeo.com/1".into(), ..Default::default() }
        );
        remember_link(&s.link, &s.referer, &s.title);
        assert_eq!(link_referer(&s.link).as_deref(), Some("https://www.behance.net/gallery/1/Project"));
        assert_eq!(link_title("https://nowhere.example/x"), None);
    }

    #[test]
    fn the_browser_button_moves_from_snag_to_snagger() {
        let dir = std::env::temp_dir();
        let me = dir.join("Snagger.exe");
        let quoted = |p: &Path| format!("\"{}\" \"%1\"", p.display());
        // Set up for the old Snag.exe: move it here.
        assert!(button_needs_relink(&quoted(&dir.join("Snag.exe")), &me));
        // Set up for this copy: leave it.
        assert!(!button_needs_relink(&quoted(&me), &me));
        // Set up for a copy that's gone: take it.
        assert!(button_needs_relink(&quoted(&dir.join("gone-for-good").join("Snagger.exe")), &me));
        // Set up for another copy that still exists: leave it (Settings offers to switch).
        let other = std::env::current_exe().unwrap();
        assert!(!button_needs_relink(&quoted(&other), &me));
    }
}
