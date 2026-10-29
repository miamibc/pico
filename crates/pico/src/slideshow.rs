//! Full-screen slideshow of the current folder.

use crate::player::Playback;
use eframe::egui;
use pico_core::image::imageops::FilterType;
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use pico_core::render;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

/// Slides are rendered at most this large; plenty for full-screen display.
const SLIDE_SIDE: u32 = 2560;
const FADE_SECONDS: f64 = 0.6;
pub const DEFAULT_SECONDS: f64 = 3.0;

pub enum SlideshowAction {
    Exit,
}

pub struct Slideshow {
    pub index: usize,
    pub playing: bool,
    pub seconds: f64,
    /// When the current slide appeared (egui time).
    shown_at: Option<f64>,
    previous: Option<egui::TextureHandle>,
    slides: HashMap<PathBuf, Slide>,
    /// The current slide when it is a video: it plays, and the show moves on
    /// when it ends.
    video: Option<Playback>,
}

enum Slide {
    Loading(Receiver<Option<egui::ColorImage>>),
    Ready(egui::TextureHandle),
    Failed,
}

impl Slideshow {
    pub fn new(index: usize) -> Self {
        Self {
            index,
            playing: true,
            seconds: DEFAULT_SECONDS,
            shown_at: None,
            previous: None,
            slides: HashMap::new(),
            video: None,
        }
    }

    /// Draws the slideshow and handles its keys. `files` are the folder's files;
    /// the slideshow wraps around at the end like Picasa's.
    pub fn show(&mut self, ui: &mut egui::Ui, files: &[MediaFile]) -> Option<SlideshowAction> {
        if files.is_empty() {
            return Some(SlideshowAction::Exit);
        }
        self.index %= files.len();
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        // Keep the current slide and the next one decoded.
        for offset in [0, 1] {
            self.request(&ctx, &files[(self.index + offset) % files.len()]);
        }
        self.poll(&ctx);
        self.forget_far_slides(files);
        let file = &files[self.index];
        if file.kind != MediaKind::Video {
            self.video = None;
        } else if self.video.as_ref().is_none_or(|v| v.path() != file.path) {
            self.video = Some(Playback::open(&file.path, false));
        }
        // A video that ffprobe cannot read shows its still like a photo.
        let video = self.video.as_mut().filter(|v| v.info().is_ok());
        let mut current = match self.slides.get(&file.path) {
            Some(Slide::Ready(texture)) => Some(texture.clone()),
            _ => None,
        };
        let ready = current.is_some() || matches!(self.slides.get(&files[self.index].path), Some(Slide::Failed));
        if ready && self.shown_at.is_none() {
            self.shown_at = Some(now);
        }

        let mut action = None;
        let (left, right, space, escape, faster, slower) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Minus),
                i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals),
            )
        });
        if escape {
            action = Some(SlideshowAction::Exit);
        }
        if space {
            self.playing = !self.playing;
            self.shown_at = Some(now);
        }
        let mut video_ended = false;
        if let Some(video) = video {
            if self.playing && !video.is_playing() && !video.finished() {
                video.play(now);
            } else if !self.playing && video.is_playing() {
                video.pause();
            }
            video.advance(&ctx, now);
            if let Some(texture) = video.texture() {
                current = Some(texture.clone());
            }
            video_ended = video.finished();
        }
        let is_video = self.video.as_ref().is_some_and(|v| v.info().is_ok());
        if faster {
            self.seconds = (self.seconds - 1.0).max(1.0);
        }
        if slower {
            self.seconds = (self.seconds + 1.0).min(60.0);
        }
        let elapsed = self.shown_at.map_or(0.0, |t| now - t);
        let done = if is_video { video_ended } else { ready && elapsed >= self.seconds };
        if right || (self.playing && done) {
            self.step(files.len(), 1, current.clone());
        } else if left {
            self.step(files.len(), -1, current.clone());
        }

        let area = ui.max_rect();
        let painter = ui.painter();
        painter.rect_filled(area, 0.0, egui::Color32::BLACK);
        let fade = (elapsed / FADE_SECONDS).clamp(0.0, 1.0) as f32;
        if let Some(previous) = &self.previous
            && fade < 1.0
        {
            paint_fit(painter, area, previous, 1.0 - fade);
        }
        if let Some(texture) = &current {
            paint_fit(painter, area, texture, if self.previous.is_some() { fade } else { 1.0 });
        } else if !ready {
            painter.text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::proportional(32.0),
                egui::Color32::GRAY,
            );
        }
        let file = &files[self.index];
        let caption = file.meta.caption.clone().unwrap_or_else(|| file.name());
        let status = if self.playing { tr!("{n}s", n = self.seconds) } else { tr!("paused").into() };
        painter.text(
            area.center_bottom() + egui::vec2(0.0, -24.0),
            egui::Align2::CENTER_BOTTOM,
            caption,
            egui::FontId::proportional(20.0),
            egui::Color32::from_white_alpha(220),
        );
        painter.text(
            area.right_bottom() + egui::vec2(-16.0, -12.0),
            egui::Align2::RIGHT_BOTTOM,
            format!(
                "{} / {}  ·  {status}  ·  {}",
                self.index + 1,
                files.len(),
                tr!("Space pause, arrows move, -/+ speed, Esc exit")
            ),
            egui::FontId::proportional(13.0),
            egui::Color32::from_white_alpha(120),
        );

        if self.playing || fade < 1.0 {
            let until_next = if self.playing { (self.seconds - elapsed).max(0.0) } else { 1.0 };
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(until_next.clamp(0.0, 1.0 / 30.0)));
        }
        action
    }

    #[cfg(test)]
    pub fn video_playing(&self) -> bool {
        self.video.as_ref().is_some_and(Playback::is_playing)
    }

    fn step(&mut self, len: usize, delta: isize, current: Option<egui::TextureHandle>) {
        self.index = (self.index as isize + delta).rem_euclid(len as isize) as usize;
        self.previous = current;
        self.shown_at = None;
    }

    fn request(&mut self, ctx: &egui::Context, file: &MediaFile) {
        if self.slides.contains_key(&file.path) {
            return;
        }
        let (tx, rx) = channel();
        let (path, kind, meta, ctx2) = (file.path.clone(), file.kind, file.meta.clone(), ctx.clone());
        // Some GPUs take no textures over 2048 pixels.
        let slide_side = SLIDE_SIDE.min(ctx.input(|i| i.max_texture_side) as u32);
        std::thread::spawn(move || {
            let image = render::decode(&path, kind, &meta).ok().map(|img| {
                let side = img.width().max(img.height());
                let (img, scale) = if side > slide_side {
                    (img.resize(slide_side, slide_side, FilterType::Triangle), slide_side as f32 / side as f32)
                } else {
                    (img, 1.0)
                };
                let rgba = render::apply_edits_scaled(img, &meta, scale).to_rgba8();
                egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
            });
            let _ = tx.send(image);
            ctx2.request_repaint();
        });
        self.slides.insert(file.path.clone(), Slide::Loading(rx));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        for slide in self.slides.values_mut() {
            if let Slide::Loading(rx) = slide
                && let Ok(result) = rx.try_recv()
            {
                *slide = match result {
                    Some(image) => Slide::Ready(ctx.load_texture("slide", image, egui::TextureOptions::LINEAR)),
                    None => Slide::Failed,
                };
            }
        }
    }

    /// Keeps memory bounded on long slideshows: only the neighbours stay.
    fn forget_far_slides(&mut self, files: &[MediaFile]) {
        let len = files.len();
        let keep: Vec<&PathBuf> = [len - 1, 0, 1].iter().map(|&o| &files[(self.index + o) % len].path).collect();
        self.slides.retain(|path, _| keep.contains(&path));
    }
}

fn paint_fit(painter: &egui::Painter, area: egui::Rect, texture: &egui::TextureHandle, opacity: f32) {
    let size = texture.size_vec2();
    let scale = (area.width() / size.x).min(area.height() / size.y);
    let rect = egui::Rect::from_center_size(area.center(), size * scale);
    let tint = egui::Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8);
    painter.image(texture.id(), rect, crate::grid::full_uv(), tint);
}
