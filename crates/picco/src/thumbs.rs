//! Background thumbnail decoding and the texture cache that feeds the grid.
//!
//! What is on screen is decoded first, then a couple of rows around it
//! (`prefetch`). Textures long out of view are dropped, so memory stays
//! bounded; the disk cache brings them back quickly.

use eframe::egui;
use picco_core::library::MediaFile;
use picco_core::media::MediaKind;
use picco_core::meta::PhotoMeta;
use picco_core::thumbnail;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Condvar, Mutex};

pub enum Thumb {
    Pending,
    Ready(egui::TextureHandle),
    Failed,
}

struct Entry {
    thumb: Thumb,
    /// The frame it was last drawn or wanted in.
    used: u64,
}

struct Job {
    path: PathBuf,
    kind: MediaKind,
    meta: PhotoMeta,
}

type Done = (PathBuf, Result<egui::ColorImage, String>);

/// Jobs in the order the workers take them, front first.
#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    closed: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    ready: Condvar,
}

pub struct ThumbCache {
    thumbs: HashMap<PathBuf, Entry>,
    shared: Arc<Shared>,
    done: Receiver<Done>,
    frame: u64,
    /// Photos drawn this frame whose thumbnails are not ready yet.
    shown: Vec<PathBuf>,
}

impl ThumbCache {
    /// With `disk_cache` off, thumbnails are rendered fresh and never stored.
    pub fn new(ctx: &egui::Context, disk_cache: bool) -> Self {
        let shared = Arc::new(Shared::default());
        let (done_tx, done) = channel::<Done>();
        let workers = std::thread::available_parallelism().map_or(2, |n| n.get().clamp(1, 8));
        for i in 0..workers {
            let shared = Arc::clone(&shared);
            let done_tx = done_tx.clone();
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("thumbs-{i}"))
                .spawn(move || {
                    loop {
                        // Hold the lock only while taking a job, not while decoding.
                        let job = {
                            let Ok(mut queue) = shared.queue.lock() else { return };
                            loop {
                                if queue.closed {
                                    return;
                                }
                                if let Some(job) = queue.jobs.pop_front() {
                                    break job;
                                }
                                let Ok(next) = shared.ready.wait(queue) else { return };
                                queue = next;
                            }
                        };
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
        Self { thumbs: HashMap::new(), shared, done, frame: 0, shown: Vec::new() }
    }

    /// Uploads finished thumbnails to the GPU. Call once per frame, before
    /// drawing.
    pub fn poll(&mut self, ctx: &egui::Context) {
        self.frame += 1;
        self.shown.clear();
        while let Ok((path, result)) = self.done.try_recv() {
            // Dropped while it was decoding: no longer wanted.
            let Some(entry) = self.thumbs.get_mut(&path) else { continue };
            entry.thumb = match result {
                Ok(image) => {
                    let name = path.to_string_lossy().into_owned();
                    Thumb::Ready(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
                }
                Err(_) => Thumb::Failed,
            };
        }
    }

    /// Thumbnails requested but not decoded yet.
    #[cfg(test)]
    pub fn pending(&self) -> usize {
        self.thumbs.values().filter(|e| matches!(e.thumb, Thumb::Pending)).count()
    }

    /// Forgets a thumbnail so it is rendered again, e.g. after its edits changed.
    pub fn invalidate(&mut self, path: &Path) {
        self.thumbs.remove(path);
    }

    /// Returns the thumbnail of a photo on screen; one not asked for before
    /// goes to the front of the queue.
    pub fn get(&mut self, path: &Path, kind: MediaKind, meta: &PhotoMeta) -> &Thumb {
        let frame = self.frame;
        let entry = self.thumbs.entry(path.to_path_buf()).or_insert_with(|| {
            let job = Job { path: path.to_path_buf(), kind, meta: meta.clone() };
            if let Ok(mut queue) = self.shared.queue.lock() {
                queue.jobs.push_front(job);
            }
            self.shared.ready.notify_one();
            Entry { thumb: Thumb::Pending, used: frame }
        });
        entry.used = frame;
        if matches!(entry.thumb, Thumb::Pending) {
            self.shown.push(path.to_path_buf());
        }
        &entry.thumb
    }

    /// After drawing: queues the photos on screen first, then `near`, the
    /// few rows around the view, nearest first; queued photos that scrolled
    /// away are dropped. Then, past `keep` thumbnails, the longest unused
    /// ones are let go.
    pub fn prefetch<'a>(&mut self, near: impl Iterator<Item = &'a MediaFile>, keep: usize) {
        let Ok(mut queue) = self.shared.queue.lock() else { return };
        let mut old: HashMap<PathBuf, Job> = queue.jobs.drain(..).map(|job| (job.path.clone(), job)).collect();
        let mut jobs: VecDeque<Job> = self.shown.drain(..).filter_map(|path| old.remove(&path)).collect();
        for file in near {
            match self.thumbs.get_mut(&file.path) {
                None => {
                    self.thumbs.insert(file.path.clone(), Entry { thumb: Thumb::Pending, used: self.frame });
                    jobs.push_back(Job { path: file.path.clone(), kind: file.kind, meta: file.meta.clone() });
                }
                Some(entry) => {
                    entry.used = self.frame;
                    // Unless already being decoded.
                    if let Some(job) = old.remove(&file.path) {
                        jobs.push_back(job);
                    }
                }
            }
        }
        // What is left scrolled away: asked for again once back in view.
        for path in old.into_keys() {
            self.thumbs.remove(&path);
        }
        queue.jobs = jobs;
        drop(queue);
        self.shared.ready.notify_all();

        if self.thumbs.len() > keep {
            let mut done: Vec<(u64, PathBuf)> = self
                .thumbs
                .iter()
                .filter(|(_, e)| !matches!(e.thumb, Thumb::Pending))
                .map(|(path, e)| (e.used, path.clone()))
                .collect();
            done.sort_unstable();
            let extra = self.thumbs.len() - keep;
            for (_, path) in done.into_iter().take(extra) {
                self.thumbs.remove(&path);
            }
        }
    }
}

impl Drop for ThumbCache {
    fn drop(&mut self) {
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.closed = true;
        }
        self.shared.ready.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use picco_core::image::{Rgb, RgbImage};
    use std::time::{Duration, Instant};

    #[test]
    fn near_photos_load_and_the_longest_unused_go() {
        let dir = std::env::temp_dir().join(format!("picco-prefetch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<MediaFile> = (0..30)
            .map(|i| {
                let path = dir.join(format!("{i}.png"));
                RgbImage::from_pixel(8, 8, Rgb([i * 8, 0, 0])).save(&path).unwrap();
                MediaFile::new(path, MediaKind::Photo)
            })
            .collect();
        let ctx = egui::Context::default();
        let mut cache = ThumbCache::new(&ctx, false);
        let ready = |cache: &ThumbCache, file: &MediaFile| {
            cache.thumbs.get(&file.path).is_some_and(|e| matches!(e.thumb, Thumb::Ready(_)))
        };
        let load = |cache: &mut ThumbCache, near: &[MediaFile]| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !near.iter().all(|f| ready(cache, f)) {
                assert!(Instant::now() < deadline, "timed out waiting for the near thumbnails");
                cache.poll(&ctx);
                cache.prefetch(near.iter(), 15);
                std::thread::sleep(Duration::from_millis(10));
            }
            cache.poll(&ctx);
            cache.prefetch(near.iter(), 15);
        };
        load(&mut cache, &files[..10]);
        assert!(files[10..].iter().all(|f| !cache.thumbs.contains_key(&f.path)), "far photos are not loaded");

        // Scrolled on: the new ones come, the longest unused go, down to 15.
        load(&mut cache, &files[10..20]);
        assert_eq!(cache.thumbs.len(), 15);
        assert!(files[10..20].iter().all(|f| ready(&cache, f)));
        assert_eq!(files[..10].iter().filter(|f| cache.thumbs.contains_key(&f.path)).count(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
