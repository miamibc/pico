//! Full-screen slideshow of the current folder.

use crate::player::{PlaySign, Playback};
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
    /// When the current slide appeared (egui time), for the fade from the
    /// one before, and when its time on screen started counting (again
    /// after a pause).
    appeared_at: Option<f64>,
    shown_at: Option<f64>,
    /// The slide before, with its quarter turns (videos are turned when
    /// painted), fading out.
    previous: Option<(egui::TextureHandle, u8)>,
    slides: HashMap<PathBuf, Slide>,
    /// The current slide when it is a video: it plays, and the show moves on
    /// when it ends.
    video: Option<Playback>,
    /// The video before, with its quarter turns: it plays on while it fades
    /// out, rather than freeze.
    previous_video: Option<(Playback, u8)>,
    /// The play and pause sign over a video, shown on Space or a mouse move.
    sign: PlaySign,
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
            appeared_at: None,
            shown_at: None,
            previous: None,
            slides: HashMap::new(),
            video: None,
            previous_video: None,
            sign: PlaySign::default(),
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
            // The show plays it on its own: no sign until asked for.
            self.sign = PlaySign::new(self.playing, now, false);
        }
        // A video that ffprobe cannot read shows its still like a photo.
        let video = self.video.as_mut().filter(|v| v.info().is_ok());
        // Stills come rendered with their rotation; video frames are turned
        // when painted. A video shows only its own frames, from its start: its
        // still is a frame from further in.
        let mut current = match self.slides.get(&file.path) {
            Some(Slide::Ready(texture)) if video.is_none() => Some((texture.clone(), 0)),
            _ => None,
        };
        let still_failed = matches!(self.slides.get(&file.path), Some(Slide::Failed));

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
        // A click on a video plays or pauses it, as Space; the sign in the
        // middle grows under the pointer while it shows.
        let area = ui.max_rect();
        let mut hovered = false;
        let mut clicked = false;
        if video.is_some() {
            let response = ui.interact(area, ui.scope_id().with("slideshow video"), egui::Sense::click());
            let label = if self.playing { "Pause" } else { "Play" };
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
            clicked = response.clicked();
            let button =
                egui::Rect::from_center_size(area.center(), egui::Vec2::splat(crate::player::sign_radius(area) * 2.4));
            hovered = self.sign.visible() && response.hover_pos().is_some_and(|p| button.contains(p));
            if hovered {
                response.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
        }
        if space || clicked {
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
                current = Some((texture.clone(), file.meta.rotate));
            }
            video_ended = video.finished();
        }
        let is_video = self.video.as_ref().is_some_and(|v| v.info().is_ok());
        // A video is ready with its first frame; until then the slide before
        // stays (and a video before plays on).
        let ready = current.is_some() || (!is_video && still_failed);
        if ready && self.shown_at.is_none() {
            self.shown_at = Some(now);
            self.appeared_at = Some(now);
        }
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

        let painter = ui.painter();
        painter.rect_filled(area, 0.0, egui::Color32::BLACK);
        // Space only pauses and plays: the fade is the slide's arrival's.
        let fade = (self.appeared_at.map_or(0.0, |t| now - t) / FADE_SECONDS).clamp(0.0, 1.0) as f32;
        if fade >= 1.0 {
            // Gone: its sound stops with it.
            self.previous_video = None;
        }
        if let Some((video, turns)) = &mut self.previous_video {
            video.advance(&ctx, now);
            if let Some(texture) = video.texture() {
                paint_fit(painter, area, texture, *turns, 1.0 - fade);
            }
        } else if let Some((previous, turns)) = &self.previous
            && fade < 1.0
        {
            paint_fit(painter, area, previous, *turns, 1.0 - fade);
        }
        let has_previous = self.previous.is_some() || self.previous_video.is_some();
        if let Some((texture, turns)) = &current {
            paint_fit(painter, area, texture, *turns, if has_previous { fade } else { 1.0 });
        } else if !ready && !has_previous {
            painter.text(
                area.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::proportional(32.0),
                egui::Color32::GRAY,
            );
        }
        // Over a video, as in the player: follows the show's pausing, not
        // the video's own end, which only moves the show on.
        if is_video
            && current.is_some()
            && self.sign.paint(ui, area, self.playing, now, false, hovered && !self.playing)
        {
            ctx.request_repaint();
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

    fn step(&mut self, len: usize, delta: isize, current: Option<(egui::TextureHandle, u8)>) {
        self.index = (self.index as isize + delta).rem_euclid(len as isize) as usize;
        // A video leaving plays on as it fades out.
        match self.video.take().filter(|v| v.texture().is_some()) {
            Some(video) => {
                self.previous_video = Some((video, current.map_or(0, |(_, turns)| turns)));
                self.previous = None;
            }
            None => {
                self.previous_video = None;
                self.previous = current;
            }
        }
        self.shown_at = None;
        self.appeared_at = None;
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

fn paint_fit(painter: &egui::Painter, area: egui::Rect, texture: &egui::TextureHandle, turns: u8, opacity: f32) {
    let tint = egui::Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8);
    crate::player::paint_turned(painter, area, texture, turns, tint);
}
