# Snag 2

A private video and audio downloader for your desktop. A native Windows app
written in Rust: no Python, no browser tab, no console window.

Paste a link from YouTube, TikTok, Instagram, X, Reddit, Facebook, Vimeo,
Twitch, SoundCloud or any of the 1,800+ sites yt-dlp supports, pick a quality,
and the file lands in `Downloads\Snag`.

## Look

Snag 2.3 looks like a command console: one pale ice-blue line colour on
near-black, hairline modules, clipped-corner frames and keys that light up.
Dark by default; Settings → Theme has a light version too. The design notes
(tokens, audit, test tasks) are in `docs/design-console.md`, with screenshots
in `docs/screens`.

## Run it

Double-click **Snag.exe**. The first launch fetches the download engine
(about 110 MB, from the projects' own GitHub releases) and then you're ready.

## What it does

**Getting links in**
- Paste into the box, click **Paste**, or press **Ctrl+V** anywhere in the window.
- **Search**: type words instead of a link to search YouTube or SoundCloud.
  Tick several results and download them together, or press the eye on a row
  to preview it first (and come back to the results after).
- **Add many**: paste a whole list, or any text that contains links (a chat, an
  email, a web page). Snag pulls out every link and skips duplicates.
- **Import**: every link in a `.txt`, `.csv`, `.html`, bookmarks export or
  `.url` shortcut. You can also drop such files onto the window.
- **Copied links**: copy a video link in your browser and Snag offers to
  download it (or, if you choose, queues it straight away).
- **Browser button**: Settings → Browser button adds a "Snag it" bookmark; on
  any video page, click it and that page opens in Snag (via `snag:` links).
- `Snag.exe <link>` or "Open with" passes links to the window that's already open.

**Plain files**: a link to a file rather than a page (`.zip`, `.pdf`, `.psd`,
`.blend`, `.mp4` on a server…) is downloaded by Snag's own Rust engine: several
connections at once (6 by default), pause and resume, a speed limit, and a
retry for any piece whose connection stalls. Links yt-dlp doesn't recognise are
checked for a file too.

**Choosing what to save**
- Quality buttons show what the video actually offers, with estimated sizes
  for the format you picked (Best/4K/1440p/1080p/720p/480p/360p, or audio only).
- Video as **MP4** (H.264, plays everywhere), **MKV** (best codecs) or as the
  site serves it. Audio as **MP3, M4A, Opus, FLAC, WAV** or untouched.
- **Trim**: download only part of a video (from 1:30 to 2:45).
- **Playlists and channels**: a list with thumbnails to pick from. Filter by
  title, click to tick, Shift+click to tick a run, or use All / None / Invert /
  First 10 / Only matching. Picked videos go in their own numbered folder. A
  video link that's part of a playlist can be switched to the whole playlist
  in one click.
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
- **Select several rows** (click, Ctrl+click, Shift+click, Ctrl+A) to resume,
  pause, retry, move up, copy the links of, or remove them together. Esc clears.
- **When done**: open the download folder, put the PC to sleep or shut it down
  once the queue finishes (with a one-minute warning you can cancel).
- **Download hours**: only start downloads between, say, 22:00 and 07:00.
  "Start now anyway" overrides it for the session.
- **Tries again by itself** after a dropped connection, a busy server or "too
  many requests" (a few times, waiting longer each time). When a site seems to
  have changed, a failed row offers "update yt-dlp and try again".
- Waits a few seconds between playlist videos so sites don't block you.
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

Plain files skip yt-dlp entirely. `src/direct.rs` asks the server for one byte
to learn the name, size and whether it allows ranges, then splits the file into
chunks (up to 8 MB) that several connections fetch at once, writing each straight
into its place in the file. Finished chunks are recorded in `state.json`, so a
pause, a quit or a dropped connection only costs the chunks in flight.

| File | What's in it |
| --- | --- |
| `src/main.rs` | Startup, single instance, `snag:` links, window |
| `src/engine.rs` | Installing and updating yt-dlp, ffmpeg, Deno |
| `src/ytdlp.rs` | Command lines, link previews, search, progress parsing |
| `src/direct.rs` | Snag's own multi-connection file downloader |
| `src/worker.rs` | One running yt-dlp download |
| `src/manager.rs` | The queue, saving, taskbar, keep-awake, download hours, "when done", subscription schedule |
| `src/hub.rs` | Shared state, queue operations, automatic retries |
| `src/clipwatch.rs`, `src/ipc.rs` | Copied-link watcher, second-launch hand-off |
| `src/ui/` | The window: downloads (preview, search, picker, queue), history, subscriptions, settings |

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
- Icons: [Phosphor](https://phosphoricons.com), Thin weight, from `@phosphor-icons/core`
  2.1.1 (MIT, see `assets/phosphor/LICENSE`), rendered with
  [resvg](https://github.com/linebender/resvg) (Apache-2.0/MIT).
- Type: [Saira](https://github.com/Omnibus-Type/Saira) (SIL OFL 1.1, see
  `assets/fonts/OFL-Saira.txt`) and [Fira Code](https://github.com/tonsky/FiraCode)
  (SIL OFL 1.1, see `assets/fonts/OFL-FiraCode.txt`).
- App icon: "Inbox tray" from [Microsoft Fluent Emoji](https://github.com/microsoft/fluentui-emoji)
  (MIT, see `assets/LICENSE-fluentui-emoji.txt`).

Only download videos you have the right to keep: your own uploads, Creative
Commons work, or videos the creator allows you to save.
