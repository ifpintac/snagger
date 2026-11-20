//! What Snag keeps track of: download jobs, quality presets, history entries
//! and subscriptions. Everything here is saved as JSON in the data folder.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::platform::ProcGroup;

// --------------------------------------------------------------------------
// Quality presets
// --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Quality {
    #[default]
    Best,
    Max(u32),
    Audio,
}

impl Quality {
    pub const ALL: [Quality; 8] = [
        Quality::Best,
        Quality::Max(2160),
        Quality::Max(1440),
        Quality::Max(1080),
        Quality::Max(720),
        Quality::Max(480),
        Quality::Max(360),
        Quality::Audio,
    ];

    pub fn label(self) -> String {
        match self {
            Quality::Best => "Best".into(),
            Quality::Max(2160) => "4K".into(),
            Quality::Max(h) => format!("{h}p"),
            Quality::Audio => "Audio only".into(),
        }
    }

    pub fn long_label(self) -> String {
        match self {
            Quality::Best => "Best available".into(),
            Quality::Max(2160) => "Up to 4K (2160p)".into(),
            Quality::Max(h) => format!("Up to {h}p"),
            Quality::Audio => "Audio only".into(),
        }
    }

    pub fn is_audio(self) -> bool {
        self == Quality::Audio
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VideoFormat {
    /// H.264 + AAC in MP4: plays on phones, TVs, Windows, editors.
    #[default]
    Mp4,
    /// Best codecs (VP9/AV1/Opus) in MKV.
    Mkv,
    /// Whatever the site serves, no remuxing.
    Original,
}

impl VideoFormat {
    pub const ALL: [VideoFormat; 3] = [VideoFormat::Mp4, VideoFormat::Mkv, VideoFormat::Original];

    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "MP4",
            VideoFormat::Mkv => "MKV",
            VideoFormat::Original => "Original",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "Plays everywhere (H.264)",
            VideoFormat::Mkv => "Best quality, newer codecs",
            VideoFormat::Original => "As the site serves it",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum AudioFormat {
    #[default]
    Mp3,
    M4a,
    Opus,
    Flac,
    Wav,
    Original,
}

impl AudioFormat {
    pub const ALL: [AudioFormat; 6] = [
        AudioFormat::Mp3,
        AudioFormat::M4a,
        AudioFormat::Opus,
        AudioFormat::Flac,
        AudioFormat::Wav,
        AudioFormat::Original,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "MP3",
            AudioFormat::M4a => "M4A",
            AudioFormat::Opus => "Opus",
            AudioFormat::Flac => "FLAC",
            AudioFormat::Wav => "WAV",
            AudioFormat::Original => "Original",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "Plays everywhere",
            AudioFormat::M4a => "Smaller, good quality",
            AudioFormat::Opus => "Smallest, best quality",
            AudioFormat::Flac => "Lossless container",
            AudioFormat::Wav => "Uncompressed",
            AudioFormat::Original => "No conversion",
        }
    }

    /// yt-dlp's `--audio-format` value.
    pub fn ytdlp(self) -> &'static str {
        match self {
            AudioFormat::Mp3 => "mp3",
            AudioFormat::M4a => "m4a",
            AudioFormat::Opus => "opus",
            AudioFormat::Flac => "flac",
            AudioFormat::Wav => "wav",
            AudioFormat::Original => "best",
        }
    }

    pub fn takes_thumbnail(self) -> bool {
        matches!(self, AudioFormat::Mp3 | AudioFormat::M4a | AudioFormat::Opus | AudioFormat::Flac)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub quality: Quality,
    pub video: VideoFormat,
    pub audio: AudioFormat,
}

impl Default for Preset {
    fn default() -> Self {
        Preset { quality: Quality::Max(1080), video: VideoFormat::Mp4, audio: AudioFormat::Mp3 }
    }
}

impl Preset {
    pub fn label(&self) -> String {
        if self.quality.is_audio() {
            self.audio.label().to_string()
        } else {
            format!("{} {}", self.quality.label(), self.video.label())
        }
    }
}

// --------------------------------------------------------------------------
// Jobs
// --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlaylistMode {
    /// Let yt-dlp decide: playlist links download every video.
    #[default]
    Auto,
    /// A video link that also names a playlist downloads just the video.
    Single,
    /// Download the whole playlist the link points into.
    Whole,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct JobOptions {
    pub preset: Preset,
    pub playlist: PlaylistMode,
    /// yt-dlp `--playlist-items` selection, e.g. "1,3,5-7". Empty = all.
    pub items: String,
    /// Only download this part, in seconds.
    pub clip: Option<(f64, f64)>,
    /// Extra folder inside the download folder (subscriptions use their name).
    pub folder: String,
    /// Set for subscription checks: only fetch what isn't in the archive yet.
    pub sub_id: String,
    pub break_on_existing: bool,
    /// A plain file link, downloaded by Snag's own engine instead of yt-dlp.
    pub direct: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Status {
    #[default]
    Queued,
    Running,
    Paused,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    pub fn is_active(self) -> bool {
        matches!(self, Status::Queued | Status::Running)
    }
    pub fn is_finished(self) -> bool {
        matches!(self, Status::Done | Status::Failed | Status::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Stage {
    #[default]
    Starting,
    Downloading,
    Processing,
}

/// Fast-changing numbers for a running job. Not saved.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub stage: Stage,
    pub speed: Option<f64>,
    pub eta: Option<u64>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub phase: String,
    pub item: u32,
    pub item_title: String,
}

pub const STOP_NONE: u8 = 0;
pub const STOP_PAUSE: u8 = 1;
pub const STOP_CANCEL: u8 = 2;

/// Lets the UI pause or cancel a running download.
#[derive(Default)]
pub struct Control {
    stop: AtomicU8,
    pub group: Mutex<Option<ProcGroup>>,
}

impl Control {
    pub fn request(&self, what: u8) {
        self.stop.store(what, Ordering::SeqCst);
        if let Ok(g) = self.group.lock()
            && let Some(g) = g.as_ref()
        {
            g.kill();
        }
    }
    pub fn stop(&self) -> u8 {
        self.stop.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Job {
    pub id: String,
    pub url: String,
    pub title: String,
    pub thumb: String,
    pub site: String,
    pub uploader: String,
    /// "extractor:id", for spotting things already downloaded.
    pub vid: String,
    pub duration: u64,
    pub opts: JobOptions,
    pub status: Status,
    pub progress: f32,
    pub created: u64,
    pub finished: u64,
    /// The download folder this job saves into (fixed when it first starts).
    pub dest: Option<PathBuf>,
    /// Where yt-dlp works before files are moved into `dest`.
    pub staging: Option<PathBuf>,
    pub files: Vec<PathBuf>,
    pub extras: Vec<PathBuf>,
    /// The playlist folder, for playlists.
    pub folder: Option<PathBuf>,
    pub error: String,
    /// A neutral outcome, e.g. "No new videos".
    pub note: String,
    pub n_items: u32,
    pub items_done: u32,
    pub items_failed: u32,
    pub bytes: u64,
    /// Automatic retries after connection problems, and when the next one is due.
    pub auto_retries: u32,
    /// Fresh-link retries after the site refused the download (HTTP 403).
    pub refusals: u32,
    pub retry_at: u64,
    #[serde(skip)]
    pub live: Live,
    #[serde(skip)]
    pub log: Vec<String>,
    #[serde(skip)]
    pub control: Option<Arc<Control>>,
}

impl Job {
    pub fn new(url: &str, opts: JobOptions) -> Job {
        Job {
            id: crate::util::new_id(),
            url: url.to_string(),
            site: crate::util::site_name("", url),
            opts,
            created: crate::util::now(),
            ..Default::default()
        }
    }

    pub fn display_title(&self) -> String {
        if !self.title.is_empty() { self.title.clone() } else { crate::util::short_url(&self.url) }
    }

    pub fn is_playlist(&self) -> bool {
        self.n_items > 1 || self.folder.is_some()
    }

    pub fn push_log(&mut self, line: String) {
        const MAX: usize = 400;
        if self.log.len() >= MAX {
            self.log.drain(..MAX / 4);
        }
        self.log.push(line);
    }

    /// Back to the queue: keeps partial data so the download resumes.
    pub fn requeue(&mut self) {
        self.status = Status::Queued;
        self.error.clear();
        self.note.clear();
        self.live = Live::default();
        self.control = None;
        self.finished = 0;
        self.auto_retries = 0;
        self.refusals = 0;
        self.retry_at = 0;
    }

    /// Waiting for an automatic retry that isn't due yet.
    pub fn retry_pending(&self, now: u64) -> bool {
        self.status == Status::Queued && self.retry_at > now
    }
}

// --------------------------------------------------------------------------
// History and subscriptions
// --------------------------------------------------------------------------

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryItem {
    pub id: String,
    pub url: String,
    pub vid: String,
    pub title: String,
    pub site: String,
    pub uploader: String,
    pub thumb: String,
    pub preset: Preset,
    pub files: Vec<PathBuf>,
    pub folder: Option<PathBuf>,
    pub bytes: u64,
    pub items: u32,
    pub finished: u64,
    pub opts: JobOptions,
}

impl HistoryItem {
    /// The file or folder to open.
    pub fn target(&self) -> Option<PathBuf> {
        if self.files.len() == 1 {
            Some(self.files[0].clone())
        } else {
            self.folder.clone().or_else(|| self.files.first().cloned())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SubState {
    #[default]
    Idle,
    /// Recording existing videos as "seen" so only new ones download.
    Preparing,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Subscription {
    pub id: String,
    pub url: String,
    pub title: String,
    pub thumb: String,
    pub folder: String,
    pub preset: Preset,
    pub created: u64,
    pub last_checked: u64,
    pub last_new: u32,
    pub total_new: u32,
    pub error: String,
    pub auto: bool,
    #[serde(skip)]
    pub state: SubState,
}
