//! Thumbnails, fetched on a few background threads and kept as textures.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

const UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";
const KEEP: usize = 500;

enum Slot {
    Loading,
    Ready(TextureHandle),
    Failed,
}

pub struct Thumbs {
    slots: HashMap<String, Slot>,
    order: VecDeque<String>,
    requests: Sender<String>,
    results: Receiver<(String, Option<ColorImage>)>,
}

impl Thumbs {
    pub fn new(ctx: &egui::Context) -> Thumbs {
        let (req_tx, req_rx) = mpsc::channel::<String>();
        let (res_tx, res_rx) = mpsc::channel();
        let req_rx = Arc::new(Mutex::new(req_rx));
        let agent: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).user_agent(UA).build().into();
        for i in 0..3 {
            let (rx, tx, ctx, agent) = (req_rx.clone(), res_tx.clone(), ctx.clone(), agent.clone());
            let _ = thread::Builder::new().name(format!("thumbs-{i}")).spawn(move || {
                loop {
                    let url = match rx.lock() {
                        Ok(r) => match r.recv() {
                            Ok(u) => u,
                            Err(_) => return,
                        },
                        Err(_) => return,
                    };
                    let img = fetch(&agent, &url);
                    if tx.send((url, img)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Thumbs { slots: HashMap::new(), order: VecDeque::new(), requests: req_tx, results: res_rx }
    }

    /// The texture if it's ready; otherwise starts loading it.
    pub fn get(&mut self, url: &str) -> Option<TextureHandle> {
        if url.is_empty() {
            return None;
        }
        match self.slots.get(url) {
            Some(Slot::Ready(t)) => Some(t.clone()),
            Some(_) => None,
            None => {
                self.slots.insert(url.to_string(), Slot::Loading);
                self.order.push_back(url.to_string());
                let _ = self.requests.send(url.to_string());
                None
            }
        }
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((url, img)) = self.results.try_recv() {
            let slot = match img {
                Some(img) => Slot::Ready(ctx.load_texture(&url, img, TextureOptions::LINEAR)),
                None => Slot::Failed,
            };
            self.slots.insert(url, slot);
        }
        while self.order.len() > KEEP {
            if let Some(old) = self.order.pop_front() {
                self.slots.remove(&old);
            }
        }
    }
}

fn fetch(agent: &ureq::Agent, url: &str) -> Option<ColorImage> {
    if !url.starts_with("http") {
        return None;
    }
    let resp = agent.get(url).header("Accept", "image/avif,image/webp,image/*;q=0.8").call().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.into_body().with_config().limit(8_000_000).read_to_vec().ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let img = img.thumbnail(480, 480);
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
}
