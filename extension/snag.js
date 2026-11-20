// Shared by the popup and the background worker: how a link is handed to
// Snagger, and how the streams a page plays are recognised.

/** snag:<link>[&referer=<page>][&title=<title>], every part encoded. */
export function snagUrl(link, referer = "", title = "") {
  let url = "snag:" + encodeURIComponent(link);
  if (referer) url += "&referer=" + encodeURIComponent(referer);
  if (title) url += "&title=" + encodeURIComponent(title);
  return url;
}

/**
 * Opens a snag: address from the tab. The browser asks once whether to
 * open Snagger ("Always allow" makes it quiet); the page stays as it is.
 */
export async function send(tabId, link, referer = "", title = "") {
  const url = snagUrl(link, referer, title);
  try {
    await chrome.tabs.update(tabId, { url });
  } catch (e) {
    // Some pages (the browser's own) can't be navigated: use a new tab.
    const t = await chrome.tabs.create({ url, active: false });
    setTimeout(() => chrome.tabs.remove(t.id).catch(() => {}), 4000);
  }
}

const SEGMENT = /\.(ts|m4s|cmfv|cmfa|vtt|webvtt|key)(\?|#|$)/;

/**
 * What a response is, if it's worth sending: a stream's playlist (HLS,
 * DASH) or a whole video/audio file. Segments of streams, tiny files and
 * YouTube's own pieces (send the page instead) are skipped.
 */
export function classify(url, contentType, size) {
  const u = url.toLowerCase();
  const type = (contentType || "").toLowerCase();
  if (!u.startsWith("http")) return null;
  if (u.includes("googlevideo.com") || u.includes("youtube.com/api")) return null;
  if (SEGMENT.test(u) || type.includes("mp2t") || type.includes("iso.segment")) return null;
  if (/\.m3u8(\?|#|$)/.test(u) || type.includes("mpegurl")) return "HLS stream";
  if (/\.mpd(\?|#|$)/.test(u) || type.includes("dash+xml")) return "DASH stream";
  if (type.startsWith("video/") || /\.(mp4|m4v|webm|mov|mkv)(\?|#|$)/.test(u)) {
    return size && size < 300_000 ? null : "Video file";
  }
  if (type.startsWith("audio/") || /\.(m4a|mp3|aac|ogg|opus|flac|wav)(\?|#|$)/.test(u)) {
    return size && size < 100_000 ? null : "Audio file";
  }
  return null;
}

/** "master.m3u8 · cdn.example.com" */
export function describe(url) {
  try {
    const u = new URL(url);
    const name = decodeURIComponent(u.pathname.split("/").filter(Boolean).pop() || u.hostname);
    return { name, host: u.hostname.replace(/^www\./, "") };
  } catch {
    return { name: url, host: "" };
  }
}

export function fmtBytes(n) {
  if (!n) return "";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) {
    n /= 1000;
    i++;
  }
  return `${n.toFixed(n < 10 && i > 0 ? 1 : 0)} ${units[i]}`;
}
