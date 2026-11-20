// Snagger's browser extension, in the background: the right-click menu, and
// a per-tab list of the streams and files each page plays (shown in the
// popup), with a count on the toolbar button.

import { classify, send } from "./snag.js";

const MAX_PER_TAB = 25;
const key = (tabId) => `tab:${tabId}`;

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({ id: "snag-link", title: "Snag this link", contexts: ["link"] });
    chrome.contextMenus.create({ id: "snag-media", title: "Snag this video", contexts: ["video", "audio"] });
    chrome.contextMenus.create({ id: "snag-page", title: "Snag this page", contexts: ["page"] });
  });
  chrome.action.setBadgeBackgroundColor({ color: "#AACFD1" });
  if (chrome.action.setBadgeTextColor) chrome.action.setBadgeTextColor({ color: "#111111" });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (!tab || tab.id === undefined) return;
  const page = info.pageUrl || tab.url || "";
  if (info.menuItemId === "snag-link" && info.linkUrl) {
    send(tab.id, info.linkUrl, page);
  } else if (info.menuItemId === "snag-media" && /^https?:/.test(info.srcUrl || "")) {
    send(tab.id, info.srcUrl, page, tab.title || "");
  } else {
    // A player that hides its stream (blob:) or the page itself: Snagger
    // reads the page; the toolbar list has the streams if it can't.
    send(tab.id, page);
  }
});

chrome.webRequest.onHeadersReceived.addListener(
  (d) => {
    noteResponse(d).catch(() => {});
  },
  { urls: ["<all_urls>"], types: ["main_frame", "media", "xmlhttprequest", "other"] },
  ["responseHeaders"],
);

async function noteResponse(d) {
  if (d.tabId < 0) return;
  if (d.type === "main_frame") {
    // A new page in this tab: start a new list.
    await chrome.storage.session.remove(key(d.tabId));
    await chrome.action.setBadgeText({ tabId: d.tabId, text: "" });
    return;
  }
  if (d.statusCode >= 400) return;
  const header = (name) => (d.responseHeaders || []).find((h) => h.name.toLowerCase() === name)?.value || "";
  // A ranged reply says the whole size after the slash.
  const range = header("content-range").split("/")[1];
  const size = parseInt(range || header("content-length"), 10) || 0;
  const kind = classify(d.url, header("content-type"), size);
  if (!kind) return;
  const k = key(d.tabId);
  const list = (await chrome.storage.session.get(k))[k] || [];
  if (list.some((x) => x.url === d.url)) return;
  list.push({ url: d.url, kind, size, at: Date.now() });
  await chrome.storage.session.set({ [k]: list.slice(-MAX_PER_TAB) });
  await chrome.action.setBadgeText({ tabId: d.tabId, text: String(Math.min(list.length, 99)) });
}

chrome.tabs.onRemoved.addListener((tabId) => {
  chrome.storage.session.remove(key(tabId));
});
