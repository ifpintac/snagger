# Snag 2

A private video and audio downloader for your desktop. A native Windows app
written in Rust: no Python, no browser tab, no console window.

Paste a link from YouTube, TikTok, Instagram, X, Reddit, Facebook, Vimeo,
Twitch, SoundCloud or any of the 1,800+ sites yt-dlp supports, pick a quality,
and the file lands in `Downloads\Snag`.

## Run it

Double-click **Snag.exe**. The first launch fetches the download engine
(about 110 MB, from the projects' own GitHub releases) and then you're ready.

## What it does

**Getting links in**
- Paste into the box, click **Paste**, or press **Ctrl+V** anywhere in the window.
- **Add many**: paste a whole list, or any text that contains links (a chat, an
  email, a web page). Snag pulls out every link and skips duplicates.
- **Import**: every link in a `.txt`, `.csv`, `.html`, bookmarks export or
  `.url` shortcut. You can also drop such files onto the window.
- **Copied links**: copy a video link in your browser and Snag offers to
  download it (or, if you choose, queues it straight away).
- `Snag.exe <link>` or "Open with" passes links to the window that's already open.

**Choosing what to save**
- Quality buttons show what the video actually offers, with estimated sizes
  (Best/4K/1440p/1080p/720p/480p/360p, or audio only).
- Video as **MP4** (H.264, plays everywhere), **MKV** (best codecs) or as the
  site serves it. Audio as **MP3, M4A, Opus, FLAC, WAV** or untouched.
- **Trim**: download only part of a video (from 1:30 to 2:45).
- **Playlists and channels**: pick which videos to get; they go in their own
  numbered folder. A video link that's part of a playlist can be switched to
  the whole playlist in one click.
- Optional subtitles (embedded or `.srt`), cover art, tags and chapters,
  SponsorBlock cutting, plain ASCII file names, custom name templates,
  sorting into folders by site or uploader.

**The queue**
- Runs 1 to 10 downloads at once (3 by default); the rest wait their turn.
- Pause, resume, cancel, retry, "download next", move to the end, per row or
  all at once. Retrying a playlist only fetches the videos that failed.
- Closing Snag pauses running downloads; they continue from where they
  stopped the next time it opens.
- Progress, speed and time left per download, in the window title and on the
  taskbar button (red if something in the batch failed). The taskbar flashes
  with a chime when a batch is done. The PC is kept awake while downloading.
- Never overwrites a file: a second copy becomes `name (2).ext`.
- Speed limit, retries, proxy, sign-in from your browser (Firefox, Chrome,
  Edge, Brave…) or a cookies.txt file for private or members-only videos.
- Optional download archive: skip anything Snag has ever downloaded.

**History** lists everything saved, searchable, with Open, Show in folder and
Download again. **Subscriptions** follow channels and playlists and download
only new videos, on demand or every 6 hours / daily while Snag is open.

**Engine**: yt-dlp updates itself when Snag starts (stable or nightly builds),
which fixes most "this site stopped working" problems. Settings → Engine shows
versions and can reinstall each part.

## Where things live

| What | Where |
| --- | --- |
| Downloads | `Downloads\Snag` (change in Settings) |
| Settings, queue, history, subscriptions | `%APPDATA%\Snag` |
| yt-dlp, ffmpeg, Deno | `%LOCALAPPDATA%\Snag\engine` |

To remove Snag, delete `Snag.exe` and those two folders.

## How it works

Snag is a Rust app (egui/eframe for the window). Downloading itself is done by
[yt-dlp](https://github.com/yt-dlp/yt-dlp), the engine behind every serious
downloader, because its 1,800 site extractors change weekly and can't be
reimplemented. Snag installs the official standalone `yt-dlp.exe` (no Python
needed), checks its SHA-256, keeps it updated, and drives it with a
machine-readable progress format. ffmpeg comes from
[yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds). YouTube needs
a JavaScript runtime: Snag uses Deno, or Node.js 23.5+ if it's already on the PC.

Each download runs in its own hidden staging folder (`.snag-temp`) inside the
download folder and inside a Windows job object, so pausing or quitting stops
yt-dlp and ffmpeg together and nothing is left running.

| File | What's in it |
| --- | --- |
| `src/main.rs` | Startup, single instance, window |
| `src/engine.rs` | Installing and updating yt-dlp, ffmpeg, Deno |
| `src/ytdlp.rs` | Command lines, link previews, progress parsing |
| `src/worker.rs` | One running download |
| `src/manager.rs` | The queue, saving, taskbar, keep-awake, subscription schedule |
| `src/hub.rs` | Shared state and queue operations |
| `src/clipwatch.rs`, `src/ipc.rs` | Copied-link watcher, second-launch hand-off |
| `src/ui/` | The window: downloads, history, subscriptions, settings |

## Build

Needs Rust (stable) and the Visual Studio Build Tools with the Windows SDK.

```
cargo build --release
```

The app is `target\release\Snag.exe`. `cargo test` runs the unit tests.

## Credits

- [yt-dlp](https://github.com/yt-dlp/yt-dlp) (Unlicense), [FFmpeg](https://ffmpeg.org)
  (GPL build), [Deno](https://deno.com) (MIT): downloaded at first run, not bundled.
- [egui / eframe](https://github.com/emilk/egui) (MIT/Apache-2.0).
- Icons: [Phosphor Icons](https://phosphoricons.com) (MIT) via egui-phosphor.
- App icon: "Inbox tray" from [Microsoft Fluent Emoji](https://github.com/microsoft/fluentui-emoji)
  (MIT, see `assets/LICENSE-fluentui-emoji.txt`).

Only download videos you have the right to keep: your own uploads, Creative
Commons work, or videos the creator allows you to save.
