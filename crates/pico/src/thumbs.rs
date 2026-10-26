//! Background thumbnail decoding and the texture cache that feeds the grid.

use eframe::egui;
use pico_core::media::MediaKind;
use pico_core::meta::PhotoMeta;
use pico_core::thumbnail;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

pub enum Thumb {
    Pending,
    Ready(egui::TextureHandle),
    Failed,
}

struct Job {
    path: PathBuf,
    kind: MediaKind,
    meta: PhotoMeta,
}

type Done = (PathBuf, Result<egui::ColorImage, String>);

pub struct ThumbCache {
    thumbs: HashMap<PathBuf, Thumb>,
    jobs: Sender<Job>,
    done: Receiver<Done>,
}

impl ThumbCache {
    /// With `disk_cache` off, thumbnails are rendered fresh and never stored.
    pub fn new(ctx: &egui::Context, disk_cache: bool) -> Self {
        let (jobs, job_rx) = channel::<Job>();
        let (done_tx, done) = channel::<Done>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 8));
        for i in 0..workers {
            let job_rx = Arc::clone(&job_rx);
            let done_tx = done_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("thumbs-{i}"))
                .spawn(move || {
                    loop {
                        // Hold the lock only while taking a job, not while decoding.
                        let job = match job_rx.lock() {
                            Ok(rx) => rx.recv(),
                            Err(_) => return,
                        };
                        let Ok(job) = job else { return };
                        let render = if disk_cache { thumbnail::cached } else { thumbnail::generate };
                        let result = render(&job.path, job.kind, &job.meta, thumbnail::DEFAULT_SIZE)
                            .map(|img| {
                                let size = [img.width() as usize, img.height() as usize];
                                egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw())
                            })
                            .map_err(|e| e.to_string());
                        if done_tx.send((job.path, result)).is_err() {
                            return;
                        }
                        ctx.request_repaint();
                    }
                })
                .expect("spawn thumbnail worker");
        }
        Self { thumbs: HashMap::new(), jobs, done }
    }

    /// Uploads finished thumbnails to the GPU. Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((path, result)) = self.done.try_recv() {
            let thumb = match result {
                Ok(image) => {
                    let name = path.to_string_lossy().into_owned();
                    Thumb::Ready(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
                }
                Err(_) => Thumb::Failed,
            };
            self.thumbs.insert(path, thumb);
        }
    }

    /// Thumbnails requested but not decoded yet.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.thumbs.values().filter(|t| matches!(t, Thumb::Pending)).count()
    }

    /// Forgets a thumbnail so it is rendered again, e.g. after its edits changed.
    pub fn invalidate(&mut self, path: &Path) {
        self.thumbs.remove(path);
    }

    /// Returns the thumbnail, queueing it for decoding on first request.
    pub fn get(&mut self, path: &Path, kind: MediaKind, meta: &PhotoMeta) -> &Thumb {
        if !self.thumbs.contains_key(path) {
            let _ = self.jobs.send(Job { path: path.to_path_buf(), kind, meta: meta.clone() });
            self.thumbs.insert(path.to_path_buf(), Thumb::Pending);
        }
        &self.thumbs[path]
    }
}
