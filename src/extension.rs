//! The Edge/Chrome extension ships inside Snagger. "Set up" writes it to a
//! folder, which the browser loads once ("Load unpacked"); Snagger rewrites
//! it at startup, so it stays current with the app.

use std::fs;
use std::io;
use std::path::PathBuf;

macro_rules! files {
    ($($path:literal),* $(,)?) => {
        &[$(($path, include_bytes!(concat!("../extension/", $path)) as &[u8])),*]
    };
}

const FILES: &[(&str, &[u8])] = files!(
    "manifest.json",
    "background.js",
    "snag.js",
    "popup.html",
    "popup.css",
    "popup.js",
    "LICENSE-phosphor.txt",
    "icons/16.png",
    "icons/32.png",
    "icons/48.png",
    "icons/128.png",
    "icons/LICENSE-fluentui-emoji.txt",
    "icons/ph/broadcast-thin.svg",
    "icons/ph/download-simple-thin.svg",
    "icons/ph/film-strip-thin.svg",
    "icons/ph/globe-thin.svg",
    "icons/ph/link-thin.svg",
    "icons/ph/music-notes-thin.svg",
    "fonts/Saira-Regular.ttf",
    "fonts/Saira-SemiBold.ttf",
    "fonts/FiraCode-Regular.ttf",
    "fonts/OFL-Saira.txt",
    "fonts/OFL-FiraCode.txt",
);

/// %APPDATA%\Snagger\browser-extension
pub fn folder() -> PathBuf {
    crate::settings::data_dir().join("browser-extension")
}

/// Writes (or refreshes) the extension's files. Returns the folder.
pub fn write() -> io::Result<PathBuf> {
    let dir = folder();
    for (rel, bytes) in FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Skip unchanged files, so the browser doesn't see needless changes.
        if fs::read(&path).ok().as_deref() != Some(*bytes) {
            fs::write(&path, bytes)?;
        }
    }
    Ok(dir)
}

/// Keeps an already set-up extension current with this version of Snagger.
pub fn refresh_if_set_up() {
    if folder().join("manifest.json").is_file() {
        let _ = write();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_the_extension_names_is_shipped() {
        let names: Vec<&str> = FILES.iter().map(|(p, _)| *p).collect();
        for needed in ["manifest.json", "background.js", "snag.js", "popup.html", "popup.css", "popup.js"] {
            assert!(names.contains(&needed), "{needed}");
        }
        // Every icon the popup points at is in the bundle.
        let popup = String::from_utf8_lossy(FILES.iter().find(|(p, _)| *p == "popup.js").unwrap().1).into_owned()
            + &String::from_utf8_lossy(FILES.iter().find(|(p, _)| *p == "popup.html").unwrap().1);
        for part in popup.split("icons/ph/").skip(1) {
            let file = part.split(['"', ')', '\'']).next().unwrap();
            assert!(names.contains(&format!("icons/ph/{file}").as_str()), "missing icons/ph/{file}");
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(FILES.iter().find(|(p, _)| *p == "manifest.json").unwrap().1).unwrap();
        assert_eq!(manifest["manifest_version"], 3);
    }
}
