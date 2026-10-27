//! Video playback in the photo view: frames from ffmpeg, sound from ffplay.

use eframe::egui;
use pico_core::filters::{Filter, FilterStack};
use pico_core::library::MediaFile;
use pico_core::video::{self, Frame, FrameStream, SoundPlayer, VideoInfo};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Decoded frames are at most this wide; enough for a window, light on the CPU.
const MAX_FRAME_WIDTH: u32 = 1280;
const SEEK_STEP: Duration = Duration::from_secs(5);

pub enum PlayerAction {
    SetFilters(FilterStack),
    /// A frame was saved as a new photo next to the video.
    SavedFrame(PathBuf),
    Error(String),
}

#[derive(Default)]
pub struct Player {
    path: Option<PathBuf>,
    info: Option<Result<VideoInfo, String>>,
    size: (u32, u32),
    playing: bool,
    /// Where playback is (or is paused).
    position: Duration,
    /// egui time and video position when playback last (re)started.
    anchor: Option<(f64, Duration)>,
    stream: Option<FrameStream>,
    /// A decoded frame that is not due yet.
    pending: Option<Frame>,
    /// The position the texture shows, to know when a paused frame is stale.
    shown: Option<Duration>,
    texture: Option<egui::TextureHandle>,
    sound: Option<SoundPlayer>,
    muted: bool,
}

impl Player {
    /// Size and length of the open video, once ffprobe has told.
    pub fn info(&self) -> Option<VideoInfo> {
        self.info.as_ref().and_then(|info| info.as_ref().ok()).copied()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, file: &MediaFile) -> Option<PlayerAction> {
        if self.path.as_deref() != Some(file.path.as_path()) {
            *self = Player { muted: self.muted, ..Player::default() };
            self.open(&file.path);
            self.position = trim_start(&file.meta.filters);
        }
        let info = match &self.info {
            Some(Ok(info)) => *info,
            Some(Err(e)) => {
                ui.centered_and_justified(|ui| ui.label(format!("This video is shy: {e}")));
                return None;
            }
            None => return None,
        };
        let start = trim_start(&file.meta.filters);
        let end = trim_end(&file.meta.filters).unwrap_or(info.duration).min(info.duration);
        let now = ui.input(|i| i.time);
        let mut action = None;

        egui::Panel::bottom("player_controls").show(ui, |ui| {
            action = self.controls(ui, file, info, start, end, now);
        });

        // Keys as in Picasa: "/" plays and pauses, "," and "." skip.
        if !ui.ctx().egui_wants_keyboard_input() {
            let (toggle, back, forward) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Slash) || i.key_pressed(egui::Key::Space),
                    i.key_pressed(egui::Key::Comma),
                    i.key_pressed(egui::Key::Period),
                )
            });
            if toggle {
                self.toggle(&file.path, now, start, end);
            }
            if back {
                self.seek(&file.path, self.position.saturating_sub(SEEK_STEP).max(start), now);
            }
            if forward {
                self.seek(&file.path, (self.position + SEEK_STEP).min(end), now);
            }
        }

        self.advance(ui.ctx(), &file.path, info, now, end);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let area = ui.max_rect();
            ui.painter().rect_filled(area, 0.0, egui::Color32::BLACK);
            if let Some(texture) = &self.texture {
                let size = texture.size_vec2();
                let scale = (area.width() / size.x).min(area.height() / size.y);
                let rect = egui::Rect::from_center_size(area.center(), size * scale);
                ui.painter().image(texture.id(), rect, crate::grid::full_uv(), egui::Color32::WHITE);
                let response = ui.interact(rect, ui.scope_id().with("video"), egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Video, true, "Video"));
                if response.clicked() {
                    self.toggle(&file.path, now, start, end);
                }
            } else {
                ui.centered_and_justified(|ui| ui.spinner());
            }
        });
        action
    }

    fn open(&mut self, path: &Path) {
        self.path = Some(path.to_path_buf());
        let info = video::probe(path).map_err(|e| e.to_string());
        if let Ok(info) = &info {
            let width = info.width.clamp(2, MAX_FRAME_WIDTH) & !1;
            let height = ((info.height as f32 * width as f32 / info.width as f32).round() as u32).max(2) & !1;
            self.size = (width, height);
        }
        self.info = Some(info);
    }

    fn controls(
        &mut self,
        ui: &mut egui::Ui,
        file: &MediaFile,
        info: VideoInfo,
        start: Duration,
        end: Duration,
        now: f64,
    ) -> Option<PlayerAction> {
        let mut action = None;
        ui.horizontal(|ui| {
            let label = if self.playing { "⏸ Pause" } else { "▶ Play" };
            if ui.button(label).on_hover_text("/ or Space").clicked() {
                self.toggle(&file.path, now, start, end);
            }
            let mut seconds = self.position.as_secs_f32();
            let total = info.duration.as_secs_f32().max(0.01);
            let slider = ui.add(egui::Slider::new(&mut seconds, 0.0..=total).show_value(false).text(format!(
                "{} / {}",
                clock(self.position),
                clock(info.duration)
            )));
            if slider.changed() {
                self.seek(&file.path, Duration::from_secs_f32(seconds), now);
            }
            if video::ffplay().is_some() && ui.toggle_value(&mut self.muted, "Mute").changed() {
                self.restart_sound(&file.path);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Trim:");
            if ui.button("Set Start").on_hover_text("Playback starts here").clicked() {
                action =
                    Some(PlayerAction::SetFilters(with_trim(&file.meta.filters, "moviestart", Some(self.position))));
            }
            if ui.button("Set End").on_hover_text("Playback stops here").clicked() {
                action = Some(PlayerAction::SetFilters(with_trim(&file.meta.filters, "movieend", Some(self.position))));
            }
            let trimmed = start > Duration::ZERO || trim_end(&file.meta.filters).is_some();
            if ui.add_enabled(trimmed, egui::Button::new("Clear Trim")).clicked() {
                let stack = with_trim(&with_trim(&file.meta.filters, "moviestart", None), "movieend", None);
                action = Some(PlayerAction::SetFilters(stack));
            }
            if trimmed {
                ui.label(egui::RichText::new(format!("plays {} to {}", clock(start), clock(end))).weak());
            }
            ui.separator();
            if ui.button("Save Frame as Photo").clicked() {
                action = Some(match save_frame(&file.path, self.position) {
                    Ok(path) => PlayerAction::SavedFrame(path),
                    Err(e) => PlayerAction::Error(format!("Could not save the frame: {e}")),
                });
            }
        });
        action
    }

    fn toggle(&mut self, path: &Path, now: f64, start: Duration, end: Duration) {
        if self.playing {
            self.pause();
        } else {
            if self.position >= end {
                self.position = start;
            }
            self.play(path, now);
        }
    }

    fn play(&mut self, path: &Path, now: f64) {
        self.playing = true;
        self.anchor = Some((now, self.position));
        self.start_stream(path);
        self.restart_sound(path);
    }

    fn pause(&mut self) {
        self.playing = false;
        self.anchor = None;
        self.stream = None;
        self.pending = None;
        self.sound = None;
    }

    fn seek(&mut self, path: &Path, to: Duration, now: f64) {
        self.position = to;
        if self.playing {
            self.play(path, now);
        } else {
            self.shown = None; // fetch the frame at the new position
        }
    }

    fn start_stream(&mut self, path: &Path) {
        let fps = match &self.info {
            Some(Ok(info)) => info.fps,
            _ => return,
        };
        self.pending = None;
        self.stream = FrameStream::start(path, self.position, self.size.0, self.size.1, fps).ok();
    }

    fn restart_sound(&mut self, path: &Path) {
        self.sound = None;
        if self.playing && !self.muted {
            self.sound = SoundPlayer::start(path, self.position);
        }
    }

    /// Shows the frame due now, and fetches a single frame while paused.
    fn advance(&mut self, ctx: &egui::Context, path: &Path, info: VideoInfo, now: f64, end: Duration) {
        if !self.playing {
            if self.shown != Some(self.position) {
                if self.stream.is_none() {
                    self.start_stream(path);
                }
                if let Some(frame) = self.stream.as_ref().and_then(|s| s.frames.try_recv().ok()) {
                    self.upload(ctx, frame);
                    self.shown = Some(self.position);
                    self.stream = None;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(15));
                }
            }
            return;
        }
        let Some((t0, p0)) = self.anchor else { return };
        let target = p0 + Duration::from_secs_f64((now - t0).max(0.0));
        let mut due = self.pending.take().filter(|f| f.at <= target);
        if due.is_none()
            && let Some(stream) = &self.stream
        {
            while let Ok(frame) = stream.frames.try_recv() {
                if frame.at <= target {
                    due = Some(frame); // keep only the newest due frame
                } else {
                    self.pending = Some(frame);
                    break;
                }
            }
        }
        if let Some(frame) = due {
            self.position = frame.at;
            self.shown = Some(frame.at);
            self.upload(ctx, frame);
        }
        if target >= end || (self.stream.is_some() && target > info.duration) {
            self.position = end.min(info.duration);
            self.pause();
            return;
        }
        ctx.request_repaint_after(Duration::from_secs_f32(0.5 / info.fps));
    }

    fn upload(&mut self, ctx: &egui::Context, frame: Frame) {
        let image = egui::ColorImage::from_rgba_unmultiplied([self.size.0 as usize, self.size.1 as usize], &frame.rgba);
        match &mut self.texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("video", image, egui::TextureOptions::LINEAR)),
        }
    }

    #[cfg(test)]
    pub fn state(&self) -> (bool, Duration, bool) {
        (self.playing, self.position, self.texture.is_some())
    }
}

fn trim_start(filters: &FilterStack) -> Duration {
    filters.0.iter().rev().find(|f| f.id == "moviestart").and_then(Filter::time).unwrap_or_default()
}

fn trim_end(filters: &FilterStack) -> Option<Duration> {
    filters.0.iter().rev().find(|f| f.id == "movieend").and_then(Filter::time)
}

/// The stack with the trim point `id` replaced (or removed with `None`).
fn with_trim(filters: &FilterStack, id: &str, at: Option<Duration>) -> FilterStack {
    let mut stack = FilterStack(filters.0.iter().filter(|f| f.id != id).cloned().collect());
    if let Some(at) = at {
        stack.0.push(Filter::trim(id, at));
    }
    stack
}

/// Saves the frame at `at` as `<video name>.<milliseconds>.jpg` next to the
/// video, the way Picasa named its snapshots.
fn save_frame(path: &Path, at: Duration) -> std::io::Result<PathBuf> {
    let frame = video::extract_frame(path, at, None)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let target = path.with_file_name(format!("{stem}.{}.jpg", at.as_millis()));
    pico_core::image::DynamicImage::ImageRgba8(frame).to_rgb8().save(&target).map_err(std::io::Error::other)?;
    Ok(target)
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}
