//! After-download tools, run with the ffmpeg Snagger already has:
//! make a file smaller (to fit a size limit), turn part of a video into a
//! GIF, and split a file into tracks by its chapters. Results are saved next
//! to the original, which is never changed.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::hub::Hub;
use crate::platform;
use crate::util;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    /// Re-encode to fit in this many megabytes.
    Shrink { megabytes: u32 },
    /// A looping GIF of `from`..`to` seconds.
    Gif { from: f64, to: f64, width: u32, fps: u32 },
    /// One file per chapter, in a folder next to the original.
    SplitChapters,
}

impl Tool {
    pub fn verb(&self) -> &'static str {
        match self {
            Tool::Shrink { .. } => "Making smaller",
            Tool::Gif { .. } => "Making a GIF",
            Tool::SplitChapters => "Splitting into tracks",
        }
    }
}

/// A tool at work, shown in the status bar.
#[derive(Clone, Debug)]
pub struct Task {
    pub id: String,
    pub label: String,
    pub progress: f32,
}

/// The longest GIF Snagger makes; past that they get enormous.
pub const GIF_MAX_SECONDS: f64 = 60.0;

struct Ff {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

fn ff(hub: &Hub) -> Option<Ff> {
    let dir = hub.lock().engine.ffmpeg.clone()?;
    Some(Ff { ffmpeg: dir.join("ffmpeg.exe"), ffprobe: dir.join("ffprobe.exe") })
}

fn command(exe: &Path) -> Command {
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::null());
    platform::hide_window(&mut cmd);
    cmd
}

/// Runs a tool in the background; progress shows in the status bar and the
/// outcome as a toast.
pub fn run_in_background(hub: &Hub, media: PathBuf, title: String, tool: Tool) {
    let hub = hub.clone();
    std::thread::spawn(move || {
        let id = util::new_id();
        let label = format!("{} · {}", tool.verb(), util::ellipsize(&title, 40));
        hub.lock().tasks.push(Task { id: id.clone(), label, progress: 0.0 });
        hub.repaint();
        let progress = |p: f32| {
            if let Some(t) = hub.lock().tasks.iter_mut().find(|t| t.id == id) {
                t.progress = p.clamp(0.0, 1.0);
            }
            hub.repaint();
        };
        let result = match ff(&hub) {
            None => Err("This needs ffmpeg. Open Settings → Engine and reinstall it.".into()),
            Some(ff) if !media.is_file() => {
                let _ = ff;
                Err("That file has been moved or deleted.".into())
            }
            Some(ff) => match tool {
                Tool::Shrink { megabytes } => shrink(&ff, &media, megabytes, &progress),
                Tool::Gif { from, to, width, fps } => gif(&ff, &media, from, to, width, fps, &progress),
                Tool::SplitChapters => split_chapters(&ff, &media, &progress),
            },
        };
        let mut c = hub.lock();
        c.tasks.retain(|t| t.id != id);
        match result {
            Ok(out) => {
                let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                c.toast(format!("Saved next to the original: {name}"));
                drop(c);
                platform::reveal(&out);
            }
            Err(e) => c.toast(e),
        }
        hub.repaint();
    });
}

/// What ffprobe knows: length in seconds, whether there's a picture, chapters.
struct Probe {
    duration: f64,
    has_video: bool,
    chapters: Vec<(f64, f64, String)>,
}

fn probe(ff: &Ff, media: &Path) -> Result<Probe, String> {
    let out = command(&ff.ffprobe)
        .args(["-v", "error", "-show_format", "-show_streams", "-show_chapters", "-of", "json"])
        .arg(media)
        .output()
        .map_err(|e| format!("Couldn't start ffprobe ({e})."))?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| "ffprobe couldn't read that file.".to_string())?;
    let num = |x: &Value| x.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| x.as_f64());
    let duration = num(&v["format"]["duration"]).unwrap_or(0.0);
    let has_video = v["streams"].as_array().is_some_and(|s| {
        s.iter().any(|st| st["codec_type"] == "video" && st["disposition"]["attached_pic"].as_i64().unwrap_or(0) == 0)
    });
    let chapters = v["chapters"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    let start = num(&c["start_time"])?;
                    let end = num(&c["end_time"])?;
                    let title = c["tags"]["title"].as_str().unwrap_or("").trim().to_string();
                    (end > start).then_some((start, end, title))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Probe { duration, has_video, chapters })
}

/// Runs ffmpeg with `-progress`, reporting `base + share × done`.
fn run_ffmpeg(
    ff: &Ff,
    args: &[std::ffi::OsString],
    duration: f64,
    base: f32,
    share: f32,
    progress: &dyn Fn(f32),
) -> Result<(), String> {
    let mut child = command(&ff.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-progress", "pipe:1", "-nostats"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start ffmpeg ({e})."))?;
    let mut stderr = child.stderr.take().expect("piped");
    let errs = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<f64>().ok())
                && duration > 0.0
            {
                progress(base + share * (us / 1e6 / duration) as f32);
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = errs.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it stopped with an error");
        Err(format!("ffmpeg couldn't finish: {}", util::ellipsize(last.trim(), 160)))
    }
}

fn os(items: &[&str]) -> Vec<std::ffi::OsString> {
    items.iter().map(Into::into).collect()
}

fn stem(media: &Path) -> String {
    media.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into())
}

/// The bitrates (kbit/s) that fit `megabytes` over `duration` seconds:
/// (video, audio). Video is None for audio files. Keeps 4 % for the
/// container and rounding.
pub fn plan_bitrates(megabytes: u32, duration: f64, has_video: bool) -> Result<(Option<u32>, u32), String> {
    if duration <= 0.0 {
        return Err("Couldn't tell how long that file is.".into());
    }
    let total = megabytes as f64 * 8_000.0 * 0.96 / duration; // kbit/s
    if !has_video {
        let audio = total.min(320.0);
        if audio < 24.0 {
            return Err(format!("That's too long to fit in {megabytes} MB. Trim it first, or pick a bigger size."));
        }
        return Ok((None, audio as u32));
    }
    let audio = if total > 1000.0 {
        128.0
    } else if total > 400.0 {
        96.0
    } else {
        64.0
    };
    let video = total - audio;
    if video < 100.0 {
        return Err(format!("That's too long to fit in {megabytes} MB. Trim it first, or pick a bigger size."));
    }
    Ok((Some(video as u32), audio as u32))
}

/// Picture height for a video bitrate: fewer bits look better smaller.
pub fn height_for(video_kbps: u32) -> u32 {
    match video_kbps {
        0..=700 => 480,
        701..=1800 => 720,
        _ => 1080,
    }
}

fn shrink(ff: &Ff, media: &Path, megabytes: u32, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    let p = probe(ff, media)?;
    let (video, audio) = plan_bitrates(megabytes, p.duration, p.has_video)?;
    let work = crate::settings::data_dir().join("tools").join(util::new_id());
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let result = (|| {
        match video {
            None => {
                let out = util::unique_path(&media.with_file_name(format!("{} ({megabytes} MB).mp3", stem(media))));
                let mut a = vec!["-i".into(), media.as_os_str().to_owned()];
                a.extend(os(&["-vn", "-c:a", "libmp3lame", "-b:a", &format!("{audio}k")]));
                a.push(out.as_os_str().to_owned());
                run_ffmpeg(ff, &a, p.duration, 0.0, 1.0, progress)?;
                Ok(out)
            }
            Some(v) => {
                let out = util::unique_path(&media.with_file_name(format!("{} ({megabytes} MB).mp4", stem(media))));
                let log = work.join("x264");
                let scale = format!("scale=-2:'min({},ih)'", height_for(v));
                // Two passes land on the size; the first only measures.
                let pass = |n: &str| {
                    let mut a: Vec<std::ffi::OsString> = vec!["-i".into(), media.as_os_str().to_owned()];
                    a.extend(os(&[
                        "-vf",
                        &scale,
                        "-c:v",
                        "libx264",
                        "-preset",
                        "medium",
                        "-b:v",
                        &format!("{v}k"),
                        "-pass",
                        n,
                    ]));
                    a.push("-passlogfile".into());
                    a.push(log.as_os_str().to_owned());
                    a
                };
                let mut first = pass("1");
                first.extend(os(&["-an", "-f", "null", "NUL"]));
                run_ffmpeg(ff, &first, p.duration, 0.0, 0.5, progress)?;
                let mut second = pass("2");
                second.extend(os(&["-c:a", "aac", "-b:a", &format!("{audio}k"), "-movflags", "+faststart"]));
                second.push(out.as_os_str().to_owned());
                run_ffmpeg(ff, &second, p.duration, 0.5, 0.5, progress)?;
                Ok(out)
            }
        }
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn gif(
    ff: &Ff,
    media: &Path,
    from: f64,
    to: f64,
    width: u32,
    fps: u32,
    progress: &dyn Fn(f32),
) -> Result<PathBuf, String> {
    let p = probe(ff, media)?;
    if !p.has_video {
        return Err("That file has no picture to make a GIF from.".into());
    }
    let to = if to <= 0.0 || (p.duration > 0.0 && to > p.duration) { p.duration } else { to };
    let length = to - from;
    if length <= 0.0 {
        return Err("The end has to come after the start.".into());
    }
    if length > GIF_MAX_SECONDS {
        return Err(format!("A GIF can be up to {} seconds long; pick a shorter part.", GIF_MAX_SECONDS as u32));
    }
    let name = format!(
        "{} ({}–{}).gif",
        stem(media),
        util::fmt_timestamp(from).replace(':', "."),
        util::fmt_timestamp(to).replace(':', ".")
    );
    let out = util::unique_path(&media.with_file_name(name));
    let filter = format!(
        "fps={fps},scale={width}:-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=192[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"
    );
    let mut a = os(&["-ss", &format!("{from:.3}"), "-t", &format!("{length:.3}")]);
    a.push("-i".into());
    a.push(media.as_os_str().to_owned());
    a.extend(os(&["-vf", &filter, "-loop", "0"]));
    a.push(out.as_os_str().to_owned());
    run_ffmpeg(ff, &a, length, 0.0, 1.0, progress)?;
    Ok(out)
}

/// "01 - Intro.mp3": safe on Windows, numbered so they sort in order.
pub fn track_name(index: usize, count: usize, title: &str, ext: &str) -> String {
    let width = count.to_string().len().max(2);
    let mut t: String =
        title
            .chars()
            .map(|c| {
                if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() {
        t = format!("Chapter {}", index + 1);
    }
    let t = util::ellipsize(&t, 120);
    format!("{:0width$} - {t}.{ext}", index + 1)
}

fn split_chapters(ff: &Ff, media: &Path, progress: &dyn Fn(f32)) -> Result<PathBuf, String> {
    let p = probe(ff, media)?;
    if p.chapters.len() < 2 {
        return Err("That file has no chapters to split by. (Snagger saves chapters when the video has them and “Write title, artist, date and chapters” is on in Settings.)".into());
    }
    let folder = util::unique_path(&media.with_file_name(format!("{} (tracks)", stem(media))));
    std::fs::create_dir_all(&folder).map_err(|e| format!("Couldn't make the folder ({e})."))?;
    let ext = media.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "mp4".into());
    let n = p.chapters.len();
    for (i, (start, end, title)) in p.chapters.iter().enumerate() {
        let out = folder.join(track_name(i, n, title, &ext));
        // Copying the streams keeps full quality and takes seconds.
        let mut a = os(&["-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}")]);
        a.push("-i".into());
        a.push(media.as_os_str().to_owned());
        a.extend(os(&["-map", "0", "-map_chapters", "-1", "-c", "copy", "-avoid_negative_ts", "make_zero"]));
        a.extend(os(&["-metadata", &format!("title={title}"), "-metadata", &format!("track={}/{n}", i + 1)]));
        a.push(out.as_os_str().to_owned());
        run_ffmpeg(ff, &a, end - start, i as f32 / n as f32, 1.0 / n as f32, progress)?;
    }
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_fit_their_limit() {
        // 10 MB for a 2-minute video: about 640 kbit/s in all.
        let (v, a) = plan_bitrates(10, 120.0, true).unwrap();
        assert_eq!(a, 96);
        let total = (v.unwrap() + a) as f64 * 120.0 / 8_000.0;
        assert!(total <= 10.0 && total > 9.0, "{total} MB");
        assert_eq!(height_for(v.unwrap()), 480);
        // An hour won't fit in 10 MB.
        assert!(plan_bitrates(10, 3600.0, true).is_err());
        // Audio: capped at 320 kbit/s.
        assert_eq!(plan_bitrates(100, 60.0, false).unwrap(), (None, 320));
    }

    #[test]
    fn track_names_sort_and_are_safe() {
        assert_eq!(track_name(0, 12, "Intro: The Start?", "mp3"), "01 - Intro_ The Start_.mp3");
        assert_eq!(track_name(9, 120, "", "m4a"), "010 - Chapter 10.m4a");
    }
}
