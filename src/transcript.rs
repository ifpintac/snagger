//! Transcripts: what's said in a video, as a plain .txt next to it.
//!
//! Made from the site's captions, fetched by yt-dlp without downloading the
//! video again. YouTube nearly always has captions, at least automatic ones
//! in the language that's spoken. The captions are cleaned up (YouTube's
//! automatic captions repeat each line as they scroll) and laid out as
//! paragraphs with a timestamp at the start of each.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use crate::engine::EnginePaths;
use crate::model::{Control, STOP_NONE};
use crate::platform::ProcGroup;
use crate::settings::{self, Settings};
use crate::util;
use crate::ytdlp;

/// One caption, in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub text: String,
}

/// What a transcript is made for.
pub struct Request<'a> {
    pub url: &'a str,
    /// The video's title; empty to take it from the file name.
    pub title: &'a str,
    /// The downloaded file the transcript goes next to.
    pub media: &'a Path,
    /// A trimmed download: only that part, with times counted from its start.
    pub clip: Option<(f64, f64)>,
}

/// Where the transcript goes: "Title [id].mp4" → "Title [id].transcript.txt".
pub fn path_for(media: &Path) -> PathBuf {
    let stem = media.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
    media.with_file_name(format!("{stem}.transcript.txt"))
}

/// The transcript next to `media`, if one was made.
pub fn existing(media: &Path) -> Option<PathBuf> {
    let p = path_for(media);
    p.is_file().then_some(p)
}

/// Makes the transcript for a finished download in the background, then
/// opens it. Progress and the outcome are shown as toasts.
pub fn get_in_background(hub: &crate::hub::Hub, url: String, title: String, media: PathBuf, clip: Option<(f64, f64)>) {
    let hub = hub.clone();
    std::thread::spawn(move || {
        let (s, paths) = {
            let mut c = hub.lock();
            c.toast("Getting the transcript…");
            (c.settings.clone(), c.engine.paths())
        };
        hub.repaint();
        let Some(paths) = paths else {
            hub.lock().toast("The download engine isn't installed. Open Settings → Engine.");
            return;
        };
        let ctl = crate::hub::Hub::new_control();
        let req = Request { url: &url, title: &title, media: &media, clip };
        let msg = match from_captions(&req, &s, &paths, &ctl) {
            Ok(Some(path)) => {
                crate::platform::open_path(&path);
                format!(
                    "Transcript saved next to the video: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            }
            Ok(None) => "This video has no captions, so there's no transcript to save.".into(),
            Err(e) => format!("Couldn't get the transcript: {e}"),
        };
        hub.lock().toast(msg);
        hub.repaint();
    });
}

/// Fetches the captions and writes the transcript. Ok(None) when the video
/// has no captions in any language Snagger asked for.
pub fn from_captions(
    req: &Request,
    s: &Settings,
    paths: &EnginePaths,
    ctl: &Arc<Control>,
) -> Result<Option<PathBuf>, String> {
    let work = settings::data_dir().join("transcripts").join(util::new_id());
    fs::create_dir_all(&work).map_err(|e| format!("couldn't make a work folder ({e})"))?;
    let result = fetch_and_write(req, s, paths, ctl, &work);
    let _ = fs::remove_dir_all(&work);
    result
}

fn fetch_and_write(
    req: &Request,
    s: &Settings,
    paths: &EnginePaths,
    ctl: &Arc<Control>,
    work: &Path,
) -> Result<Option<PathBuf>, String> {
    let wanted = caption_langs(&s.sub_langs);
    let args = ytdlp::caption_args(req.url, s, paths, work, &wanted);
    let mut cmd = ytdlp::command(paths);
    cmd.args(&args).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start yt-dlp ({e})"))?;
    let group = ProcGroup::new();
    if let Some(g) = &group {
        g.assign(&child);
    }
    *ctl.group.lock().unwrap_or_else(|p| p.into_inner()) = group;
    let mut errors = Vec::new();
    if let Some(stderr) = child.stderr.take() {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.starts_with("ERROR") {
                errors.push(line);
            }
        }
    }
    let _ = child.wait();
    if ctl.stop() != STOP_NONE {
        return Err("stopped".into());
    }

    let files: Vec<(String, PathBuf)> = fs::read_dir(work)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().into_owned();
            let ext = p.extension()?.to_string_lossy().to_ascii_lowercase();
            if !matches!(ext.as_str(), "vtt" | "srt") {
                return None;
            }
            // "t.en.vtt" → "en"
            let lang = name.strip_prefix("t.")?.rsplit_once('.')?.0.to_string();
            Some((lang, p))
        })
        .collect();
    let Some((lang, file)) = pick(&files, &s.sub_langs) else {
        return match errors.last() {
            Some(e) if !files.is_empty() || !e.to_lowercase().contains("subtitle") => {
                Err(util::friendly_error(e, &s.cookies_browser))
            }
            _ => Ok(None),
        };
    };
    let raw = fs::read_to_string(file).map_err(|e| format!("couldn't read the captions ({e})"))?;
    let mut cues = dedupe(parse(&raw));
    if let Some((a, b)) = req.clip {
        cues = clip(cues, a, b);
    }
    if cues.is_empty() {
        return Ok(None);
    }
    let source = if lang.ends_with("-orig") {
        format!("From the site's automatic captions ({}).", lang.trim_end_matches("-orig"))
    } else {
        format!("From the site's captions ({lang}).")
    };
    let title = if req.title.trim().is_empty() { title_of(req.media) } else { req.title.trim().to_string() };
    let text = render(&title, req.url, &source, &cues);
    let out = path_for(req.media);
    fs::write(&out, text).map_err(|e| format!("couldn't save the transcript ({e})"))?;
    Ok(Some(out))
}

/// The languages to ask for: the subtitle languages from Settings ("all"
/// would fetch every machine translation, so it's left out), plus the
/// original-language automatic captions YouTube marks with "-orig".
pub fn caption_langs(setting: &str) -> String {
    let mut langs: Vec<&str> = setting
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('-') && *l != "all" && !l.contains("live_chat"))
        .collect();
    if langs.is_empty() {
        langs = vec!["en.*", "en"];
    }
    langs.push(".*-orig");
    langs.join(",")
}

/// Picks the captions closest to what was said:
/// - YouTube marks its automatic captions of the spoken language "-orig".
///   If plain captions in that same language came too, those are preferred
///   (written by a person, or identical); otherwise the "-orig" ones, so a
///   Tagalog video gets a Tagalog transcript, not a machine translation.
/// - Otherwise the first of the Settings languages that came, then any.
pub fn pick<'a>(files: &'a [(String, PathBuf)], setting: &str) -> Option<(String, &'a PathBuf)> {
    let plain = |lang: &str| files.iter().find(|(l, _)| l == lang);
    if let Some((orig, p)) = files.iter().find(|(l, _)| l.ends_with("-orig")) {
        let base = orig.trim_end_matches("-orig");
        return Some(match plain(base) {
            Some((l, q)) => (l.clone(), q),
            None => (orig.clone(), p),
        });
    }
    for pat in setting.split(',').map(str::trim).filter(|p| !p.is_empty() && !p.starts_with('-')) {
        let hit = match pat.strip_suffix(".*") {
            Some(prefix) => files.iter().find(|(l, _)| l.starts_with(prefix)),
            None => plain(pat),
        };
        if let Some((l, p)) = hit {
            return Some((l.clone(), p));
        }
    }
    files.iter().min_by(|a, b| a.0.cmp(&b.0)).map(|(l, p)| (l.clone(), p))
}

/// Reads WebVTT or SubRip into cues, without tags or styling.
pub fn parse(raw: &str) -> Vec<Cue> {
    let raw = raw.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();
    for block in raw.split("\n\n") {
        let mut lines = block.lines().map(str::trim).filter(|l| !l.is_empty());
        let mut timing = None;
        for l in lines.by_ref() {
            if l.contains("-->") {
                timing = Some(l);
                break;
            }
        }
        let Some(timing) = timing else { continue };
        let Some(start) = timing.split("-->").next().and_then(|t| seconds(t.trim())) else { continue };
        let text: Vec<String> = lines.map(clean).filter(|l| !l.is_empty()).collect();
        if !text.is_empty() {
            cues.push(Cue { start, text: text.join("\n") });
        }
    }
    cues
}

/// "01:02:03.450", "02:03.450" or "01:02:03,450" → seconds.
fn seconds(t: &str) -> Option<f64> {
    let t = t.split_whitespace().next()?.replace(',', ".");
    let parts: Vec<&str> = t.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        [m, s] => (0.0, m.parse::<f64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Removes <tags> and decodes the common entities.
fn clean(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_tag = false;
    for c in line.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One cue per new line of speech. YouTube's automatic captions show each
/// line twice (once new, once scrolled up) plus tiny repeat cues; every line
/// that was just said is dropped.
pub fn dedupe(cues: Vec<Cue>) -> Vec<Cue> {
    let mut out: Vec<Cue> = Vec::new();
    let mut recent: Vec<String> = Vec::new();
    for cue in cues {
        for line in cue.text.lines() {
            if line.is_empty() || recent.iter().any(|r| r == line) {
                continue;
            }
            out.push(Cue { start: cue.start, text: line.to_string() });
            recent.push(line.to_string());
            if recent.len() > 3 {
                recent.remove(0);
            }
        }
    }
    out
}

/// Only the cues inside a trimmed download, timed from its start.
fn clip(cues: Vec<Cue>, a: f64, b: f64) -> Vec<Cue> {
    let end = if b > 0.0 { b } else { f64::INFINITY };
    cues.into_iter()
        .filter(|c| c.start >= a - 0.5 && c.start < end)
        .map(|c| Cue { start: (c.start - a).max(0.0), ..c })
        .collect()
}

/// "Title [id]" → "Title".
fn title_of(media: &Path) -> String {
    let stem = media.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    match stem.rsplit_once(" [") {
        Some((title, id)) if id.ends_with(']') && !title.is_empty() => title.to_string(),
        _ => stem,
    }
}

fn stamp(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// Paragraphs of about half a minute, broken at a sentence end or a pause,
/// each starting with its time.
pub fn render(title: &str, url: &str, source: &str, cues: &[Cue]) -> String {
    let mut out = format!("{title}\r\n{url}\r\n{source}\r\n");
    let mut para: Vec<&str> = Vec::new();
    let mut para_start = 0.0;
    let mut last = 0.0;
    for cue in cues {
        let sentence_done = para.last().is_some_and(|l| l.ends_with(['.', '?', '!', '"', '…']));
        let long = cue.start - para_start;
        // Captions start every few seconds while someone talks.
        let pause = cue.start - last > 6.0;
        if !para.is_empty() && (pause || (long > 30.0 && sentence_done) || long > 60.0) {
            out.push_str(&format!("\r\n[{}] {}\r\n", stamp(para_start), para.join(" ")));
            para.clear();
        }
        if para.is_empty() {
            para_start = cue.start;
        }
        para.push(&cue.text);
        last = cue.start;
    }
    if !para.is_empty() {
        out.push_str(&format!("\r\n[{}] {}\r\n", stamp(para_start), para.join(" ")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const YT_AUTO: &str = "WEBVTT\nKind: captions\nLanguage: en\n\n\
00:00:00.320 --> 00:00:02.869 align:start position:0%\n \nhello<00:00:00.640><c> everyone</c><00:00:01.120><c> and</c>\n\n\
00:00:02.869 --> 00:00:02.879 align:start position:0%\nhello everyone and\n \n\n\
00:00:02.879 --> 00:00:05.510 align:start position:0%\nhello everyone and\nwelcome<00:00:03.200><c> to</c><00:00:03.600><c> the</c><00:00:04.000><c> show</c>\n\n\
00:00:05.510 --> 00:00:05.520 align:start position:0%\nwelcome to the show\n \n";

    #[test]
    fn youtube_automatic_captions_lose_their_repeats() {
        let cues = dedupe(parse(YT_AUTO));
        let text: Vec<&str> = cues.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(text, ["hello everyone and", "welcome to the show"]);
        assert!((cues[1].start - 2.879).abs() < 1e-6);
    }

    #[test]
    fn subrip_reads_too() {
        let srt = "1\r\n00:00:01,000 --> 00:00:03,500\r\nFirst &amp; <i>second</i>\r\n\r\n2\r\n00:01:02,250 --> 00:01:04,000\r\nThird\r\n";
        let cues = parse(srt);
        assert_eq!(
            cues,
            [Cue { start: 1.0, text: "First & second".into() }, Cue { start: 62.25, text: "Third".into() }]
        );
    }

    #[test]
    fn the_spoken_language_wins_over_a_translation() {
        let f = |l: &str| (l.to_string(), PathBuf::from(format!("t.{l}.vtt")));
        // A Tagalog video: YouTube offers Tagalog (spoken) and an English translation.
        let files = [f("en"), f("tl-orig")];
        assert_eq!(pick(&files, "en.*,en").unwrap().0, "tl-orig");
        // An English video: plain English is preferred over the "-orig" copy.
        let files = [f("en"), f("en-orig")];
        assert_eq!(pick(&files, "en.*,en").unwrap().0, "en");
        // Another site: the first Settings language that came.
        let files = [f("de"), f("en-US")];
        assert_eq!(pick(&files, "en.*,en").unwrap().0, "en-US");
        assert!(pick(&[], "en").is_none());
    }

    #[test]
    fn caption_languages_never_ask_for_everything() {
        assert_eq!(caption_langs("en.*,en"), "en.*,en,.*-orig");
        assert_eq!(caption_langs("all,-live_chat"), "en.*,en,.*-orig");
        assert_eq!(caption_langs("tl, en"), "tl,en,.*-orig");
    }

    #[test]
    fn paragraphs_with_times() {
        let cues: Vec<Cue> = (0..8).map(|i| Cue { start: i as f64 * 5.0, text: format!("Line {i}.") }).collect();
        let text = render("A film", "https://example.com/v", "From the site's captions (en).", &cues);
        assert!(text.starts_with("A film\r\nhttps://example.com/v\r\nFrom the site's captions (en).\r\n"));
        assert!(text.contains("\r\n[00:00] Line 0. Line 1. Line 2. Line 3. Line 4. Line 5. Line 6.\r\n"), "{text}");
        assert!(text.contains("\r\n[00:35] Line 7.\r\n"), "{text}");
        // A long silence starts a new paragraph.
        let gap = [Cue { start: 0.0, text: "Hi".into() }, Cue { start: 20.0, text: "Back".into() }];
        assert!(render("t", "u", "s", &gap).contains("\r\n[00:20] Back\r\n"));
        assert_eq!(stamp(3725.0), "1:02:05");
    }

    #[test]
    fn trimmed_downloads_get_their_part() {
        let cues = vec![
            Cue { start: 5.0, text: "a".into() },
            Cue { start: 61.0, text: "b".into() },
            Cue { start: 95.0, text: "c".into() },
        ];
        let got = clip(cues, 60.0, 90.0);
        assert_eq!(got, [Cue { start: 1.0, text: "b".into() }]);
    }

    #[test]
    fn names_and_titles() {
        let media = Path::new(r"C:\d\Sprite Fright - Blender Open Movie [_cMxraX_5RE].mp4");
        assert_eq!(path_for(media), Path::new(r"C:\d\Sprite Fright - Blender Open Movie [_cMxraX_5RE].transcript.txt"));
        assert_eq!(title_of(media), "Sprite Fright - Blender Open Movie");
    }
}
