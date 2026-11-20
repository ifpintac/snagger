//! User settings, where Snagger keeps its files, and JSON load/save.

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{JobOptions, PlaylistMode, Preset};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Organize {
    #[default]
    None,
    BySite,
    ByUploader,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NameStyle {
    #[default]
    TitleId,
    Title,
    UploaderTitle,
    DateTitle,
    Custom,
}

impl NameStyle {
    pub const ALL: [NameStyle; 5] =
        [NameStyle::TitleId, NameStyle::Title, NameStyle::UploaderTitle, NameStyle::DateTitle, NameStyle::Custom];

    pub fn label(self) -> &'static str {
        match self {
            NameStyle::TitleId => "Title [id]",
            NameStyle::Title => "Title",
            NameStyle::UploaderTitle => "Uploader - Title",
            NameStyle::DateTitle => "Upload date - Title",
            NameStyle::Custom => "Custom template",
        }
    }

    pub fn example(self) -> &'static str {
        match self {
            NameStyle::TitleId => "Me at the zoo [jNQXAC9IVRw].mp4",
            NameStyle::Title => "Me at the zoo.mp4",
            NameStyle::UploaderTitle => "jawed - Me at the zoo.mp4",
            NameStyle::DateTitle => "2005-04-24 - Me at the zoo.mp4",
            NameStyle::Custom => "Uses yt-dlp output template fields",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubsMode {
    #[default]
    Off,
    Embed,
    Files,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ClipMode {
    Off,
    #[default]
    Ask,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubCheck {
    Off,
    Every6h,
    #[default]
    Daily,
}

impl SubCheck {
    pub fn seconds(self) -> Option<u64> {
        match self {
            SubCheck::Off => None,
            SubCheck::Every6h => Some(6 * 3600),
            SubCheck::Daily => Some(24 * 3600),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemePref {
    System,
    Light,
    /// The console look is a dark style, so it's the default.
    #[default]
    Dark,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub parallel: u32,
    pub preset: Preset,
    pub organize: Organize,
    pub name_style: NameStyle,
    pub custom_template: String,
    pub restrict_names: bool,
    pub use_archive: bool,
    pub speed_limit_kb: u32,
    pub keep_awake: bool,
    pub notify_done: bool,

    pub subtitles: SubsMode,
    /// Save a .txt transcript (from the captions) next to each video.
    pub transcripts: bool,
    pub sub_langs: String,
    pub auto_subs: bool,
    pub embed_thumbnail: bool,
    pub write_thumbnail: bool,
    pub embed_metadata: bool,
    pub sponsorblock: bool,

    pub cookies_browser: String,
    pub cookies_file: String,
    pub proxy: String,
    pub retries: u32,
    pub fragments: u32,
    /// Connections per plain file download (Snagger's own engine).
    pub connections: u32,
    /// Wait a few seconds between playlist videos, so sites don't block you.
    pub gentle_playlists: bool,
    /// Try again by itself after connection problems.
    pub auto_retry: bool,
    /// Only start downloads between these times (minutes after midnight).
    pub window_on: bool,
    pub window_start: u16,
    pub window_end: u16,

    pub clipboard: ClipMode,
    pub batch_playlists: bool,
    pub resume_on_start: bool,
    pub auto_update: bool,
    pub nightly: bool,
    pub sub_check: SubCheck,
    pub theme: ThemePref,
    pub extra_args: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            download_dir: default_download_dir(),
            parallel: 3,
            preset: Preset::default(),
            organize: Organize::None,
            name_style: NameStyle::TitleId,
            custom_template: "%(uploader)s/%(title)s [%(id)s]".into(),
            restrict_names: false,
            use_archive: false,
            speed_limit_kb: 0,
            keep_awake: true,
            notify_done: true,
            subtitles: SubsMode::Off,
            transcripts: true,
            sub_langs: "en.*,en".into(),
            auto_subs: false,
            embed_thumbnail: true,
            write_thumbnail: false,
            embed_metadata: true,
            sponsorblock: false,
            cookies_browser: String::new(),
            cookies_file: String::new(),
            proxy: String::new(),
            retries: 10,
            fragments: 4,
            connections: 6,
            gentle_playlists: true,
            auto_retry: true,
            window_on: false,
            window_start: 22 * 60,
            window_end: 7 * 60,
            clipboard: ClipMode::Ask,
            batch_playlists: false,
            resume_on_start: true,
            auto_update: true,
            nightly: false,
            sub_check: SubCheck::Daily,
            theme: ThemePref::Dark,
            extra_args: String::new(),
        }
    }
}

impl Settings {
    /// Options for links added without a preview (batch, clipboard, command line).
    pub fn job_options(&self) -> JobOptions {
        JobOptions {
            preset: self.preset,
            playlist: if self.batch_playlists { PlaylistMode::Whole } else { PlaylistMode::Single },
            ..Default::default()
        }
    }

    pub fn sanitized(mut self) -> Settings {
        self.parallel = self.parallel.clamp(1, 10);
        self.retries = self.retries.clamp(0, 100);
        self.fragments = self.fragments.clamp(1, 16);
        self.connections = self.connections.clamp(1, 16);
        self.window_start %= 24 * 60;
        self.window_end %= 24 * 60;
        if self.download_dir.as_os_str().is_empty() {
            self.download_dir = default_download_dir();
        }
        self
    }

    /// Whether new downloads may start at `minute` (minutes after local
    /// midnight). A window like 22:00–07:00 wraps past midnight.
    pub fn in_window(&self, minute: u16) -> bool {
        if !self.window_on || self.window_start == self.window_end {
            return true;
        }
        let (a, b) = (self.window_start, self.window_end);
        if a < b { (a..b).contains(&minute) } else { minute >= a || minute < b }
    }
}

/// "22:00"
pub fn fmt_minute(m: u16) -> String {
    format!("{:02}:{:02}", m / 60 % 24, m % 60)
}

/// Minutes after midnight, local time.
pub fn local_minute() -> u16 {
    use chrono::Timelike;
    let t = chrono::Local::now();
    (t.hour() * 60 + t.minute()) as u16
}

pub const BROWSERS: &[(&str, &str)] = &[
    ("", "None"),
    ("firefox", "Firefox"),
    ("chrome", "Chrome"),
    ("edge", "Edge"),
    ("brave", "Brave"),
    ("opera", "Opera"),
    ("vivaldi", "Vivaldi"),
    ("chromium", "Chromium"),
    ("whale", "Whale"),
];

// --------------------------------------------------------------------------
// Locations
// --------------------------------------------------------------------------

/// The user's real Downloads folder (even when moved), plus \Snagger.
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Snagger")
}

/// Versions up to 2.4 were called Snag and kept their folders under that
/// name. Moves them to the new name once, at startup (before anything reads
/// them); if a folder can't be moved (an old copy still open), it's used
/// where it is.
pub fn migrate_from_snag() {
    for base in [dirs::config_dir(), dirs::data_local_dir()].into_iter().flatten() {
        let (old, new) = (base.join("Snag"), base.join("Snagger"));
        if old.is_dir() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// `base`\Snagger, or `base`\Snag while that couldn't be moved yet.
fn app_dir(base: PathBuf) -> PathBuf {
    let (old, new) = (base.join("Snag"), base.join("Snagger"));
    if !new.exists() && old.is_dir() { old } else { new }
}

/// Settings, queue, history: %APPDATA%\Snagger
pub fn data_dir() -> PathBuf {
    app_dir(dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")))
}

/// yt-dlp, ffmpeg, Deno: %LOCALAPPDATA%\Snagger\engine
pub fn engine_dir() -> PathBuf {
    app_dir(dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."))).join("engine")
}

pub fn archive_path() -> PathBuf {
    data_dir().join("archive.txt")
}

pub fn subs_archive_path() -> PathBuf {
    data_dir().join("subscriptions-archive.txt")
}

/// Each job remembers what it already finished, so a paused or retried
/// playlist picks up where it left off.
pub fn job_archive_path(job_id: &str) -> PathBuf {
    data_dir().join("job-archives").join(format!("{job_id}.txt"))
}

// --------------------------------------------------------------------------
// JSON files
// --------------------------------------------------------------------------

pub fn load_json<T: DeserializeOwned + Default>(name: &str) -> T {
    let path = data_dir().join(name);
    let Ok(text) = fs::read_to_string(&path) else { return T::default() };
    match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => {
            // Keep the unreadable file instead of overwriting it.
            let _ = fs::rename(&path, path.with_extension("json.unreadable"));
            T::default()
        }
    }
}

pub fn save_text(name: &str, text: &str) -> std::io::Result<()> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    write_atomic(&dir.join(name), text.as_bytes())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_window_wraps_midnight() {
        let s = Settings { window_on: true, window_start: 22 * 60, window_end: 7 * 60, ..Default::default() };
        assert!(s.in_window(23 * 60));
        assert!(s.in_window(3 * 60));
        assert!(!s.in_window(12 * 60));
        assert!(!s.in_window(7 * 60));
        let day = Settings { window_on: true, window_start: 9 * 60, window_end: 17 * 60, ..Default::default() };
        assert!(day.in_window(9 * 60) && !day.in_window(17 * 60) && !day.in_window(8 * 60));
        assert!(Settings::default().in_window(12 * 60));
    }

    #[test]
    fn times_format() {
        assert_eq!(fmt_minute(450), "07:30");
        assert_eq!(fmt_minute(22 * 60), "22:00");
    }
}
