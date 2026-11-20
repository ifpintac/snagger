// The toolbar popup: send this page, or one of the streams it played.

import { describe, fmtBytes, send } from "./snag.js";

const $ = (s) => document.querySelector(s);

const ICONS = {
  "HLS stream": "icons/ph/broadcast-thin.svg",
  "DASH stream": "icons/ph/broadcast-thin.svg",
  "Video file": "icons/ph/film-strip-thin.svg",
  "Audio file": "icons/ph/music-notes-thin.svg",
};

/** Streams first (they're usually the video), then the biggest files. */
function order(list) {
  const rank = (x) => (x.kind.endsWith("stream") ? 0 : 1);
  return [...list].sort((a, b) => rank(a) - rank(b) || (b.size || 0) - (a.size || 0) || a.at - b.at);
}

function row(item, tab) {
  const li = document.createElement("li");
  const icon = document.createElement("span");
  icon.className = "icon";
  icon.style.setProperty("--icon", `url(${ICONS[item.kind] || ICONS["Video file"]})`);
  icon.setAttribute("aria-hidden", "true");

  const { name, host } = describe(item.url);
  const text = document.createElement("div");
  text.style.minWidth = "0";
  const n = document.createElement("div");
  n.className = "name";
  n.textContent = name;
  n.title = item.url;
  const m = document.createElement("div");
  m.className = "meta";
  m.textContent = [item.kind, fmtBytes(item.size), host].filter(Boolean).join(" · ");
  text.append(n, m);

  const b = document.createElement("button");
  b.className = "key";
  b.type = "button";
  b.textContent = "Send";
  b.setAttribute("aria-label", `Send ${name} to Snagger`);
  // The page goes along: servers often only answer requests from it.
  b.onclick = async () => {
    await send(tab.id, item.url, tab.url || "", tab.title || "");
    window.close();
  };
  li.append(icon, text, b);
  return li;
}

async function main() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab) return;
  $("#page-title").textContent = tab.title || "This page";
  try {
    $("#page-host").textContent = new URL(tab.url).hostname.replace(/^www\./, "");
  } catch {
    $("#page-host").textContent = "";
  }
  const web = /^https?:/.test(tab.url || "");
  $("#send-page").disabled = !web;
  $("#send-page").onclick = async () => {
    await send(tab.id, tab.url);
    window.close();
  };

  const k = `tab:${tab.id}`;
  const found = order((await chrome.storage.session.get(k))[k] || []);
  const list = $("#list");
  list.replaceChildren(...found.map((item) => row(item, tab)));
  $("#empty").hidden = found.length > 0;
}

if (globalThis.chrome?.tabs) {
  main();
} else {
  // Opened as a plain page (to check the layout): show sample rows.
  const tab = { id: 0, url: "https://www.behance.net/gallery/1/Project", title: "Paper Boats — Animated Short" };
  $("#page-title").textContent = tab.title;
  $("#page-host").textContent = "behance.net";
  const sample = [
    { url: "https://cdn-prod-ccv.adobe.io/abc/master.m3u8", kind: "HLS stream", size: 0, at: 1 },
    { url: "https://mir-s3-cdn-cf.behance.net/project_modules/source/clip.mp4", kind: "Video file", size: 48_200_000, at: 2 },
  ];
  $("#list").replaceChildren(...sample.map((item) => row(item, tab)));
}
