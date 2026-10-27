//! Video playback: frames from ffmpeg, sound from ffplay. `Playback` plays a
//! video for the photo view and the slideshow; `Player` adds the controls.

use eframe::egui;
use pico_core::library::MediaFile;
use pico_core::video::{self, Frame, FrameStream, SoundPlayer, VideoInfo};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Decoded frames are at most this wide; enough for a window, light on the CPU.
const MAX_FRAME_WIDTH: u32 = 1280;
const SEEK_STEP: Duration = Duration::from_secs(5);

pub enum PlayerAction {
    /// A frame was saved as a new photo next to the video.
    SavedFrame(PathBuf),
    Error(String),
    /// Double-click: back to the library, as for photos.
    Close,
}

/// One open video: what is on screen and, while playing, the decoder and
/// the sound.
pub struct Playback {
    path: PathBuf,
    info: Result<VideoInfo, String>,
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
    /// egui time the sound was asked to start; pictures wait for it a while.
    sound_from: f64,
    muted: bool,
}

impl Playback {
    /// Opens `path` paused at the start; ffprobe tells its size and length.
    pub fn open(path: &Path, muted: bool) -> Playback {
        let info = video::probe(path).map_err(|e| e.to_string());
        let size = match &info {
            Ok(info) => {
                let width = info.width.clamp(2, MAX_FRAME_WIDTH) & !1;
                let height = ((info.height as f32 * width as f32 / info.width as f32).round() as u32).max(2) & !1;
                (width, height)
            }
            Err(_) => (0, 0),
        };
        Playback {
            path: path.to_path_buf(),
            info,
            size,
            playing: false,
            position: Duration::ZERO,
            anchor: None,
            stream: None,
            pending: None,
            shown: None,
            texture: None,
            sound: None,
            sound_from: 0.0,
            muted,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn info(&self) -> Result<VideoInfo, &str> {
        self.info.as_ref().copied().map_err(String::as_str)
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Played to the end and stopped there.
    pub fn finished(&self) -> bool {
        !self.playing && self.info().is_ok_and(|info| self.position >= info.duration)
    }

    pub fn toggle(&mut self, now: f64) {
        if self.playing {
            self.pause();
        } else {
            self.play(now);
        }
    }

    /// Plays from where it is, or from the start once at the end.
    pub fn play(&mut self, now: f64) {
        if self.finished() {
            self.position = Duration::ZERO;
        }
        self.playing = true;
        self.anchor = Some((now, self.position));
        self.start_stream();
        self.restart_sound(now);
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.anchor = None;
        self.stream = None;
        self.pending = None;
        self.sound = None;
    }

    pub fn seek(&mut self, to: Duration, now: f64) {
        self.position = to;
        if self.playing {
            self.play(now);
        } else {
            self.shown = None; // fetch the frame at the new position
        }
    }

    pub fn set_muted(&mut self, muted: bool, now: f64) {
        self.muted = muted;
        self.restart_sound(now);
    }

    fn start_stream(&mut self) {
        let Ok(info) = &self.info else { return };
        self.pending = None;
        self.stream = FrameStream::start(&self.path, self.position, self.size.0, self.size.1, info.fps).ok();
    }

    fn restart_sound(&mut self, now: f64) {
        self.sound = None;
        if self.playing && !self.muted {
            self.sound = SoundPlayer::start(&self.path, self.position);
            self.sound_from = now;
            self.anchor = Some((now, self.position));
        }
    }

    /// Shows the frame due now, and fetches a single frame while paused.
    pub fn advance(&mut self, ctx: &egui::Context, now: f64) {
        let Ok(info) = self.info else { return };
        if !self.playing {
            if self.shown != Some(self.position) {
                if self.stream.is_none() {
                    self.start_stream();
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
        let Some((mut t0, p0)) = self.anchor else { return };
        // ffplay takes a moment to start the sound; the pictures wait for it,
        // or they would run ahead of it for good.
        if self.sound.as_ref().is_some_and(|s| !s.started()) && now - self.sound_from < 2.0 {
            t0 = now;
            self.anchor = Some((t0, p0));
        }
        let target = p0 + Duration::from_secs_f64((now - t0).max(0.0));
        // The newest frame that is due; a frame not due yet waits for its turn.
        let mut due = None;
        loop {
            let next = self.pending.take().or_else(|| self.stream.as_ref()?.frames.try_recv().ok());
            match next {
                Some(frame) if frame.at <= target => due = Some(frame),
                Some(frame) => {
                    self.pending = Some(frame);
                    break;
                }
                None => break,
            }
        }
        if let Some(frame) = due {
            self.position = frame.at;
            self.shown = Some(frame.at);
            self.upload(ctx, frame);
        }
        if target >= info.duration {
            self.position = info.duration;
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
}

/// The photo view's video player: the picture and a row of controls.
#[derive(Default)]
pub struct Player {
    playback: Option<Playback>,
    /// Where the picture is shown, as of the last frame.
    area: Option<egui::Rect>,
    muted: bool,
    /// A click on the picture, which pauses or plays unless it turns out to
    /// be the first half of a double-click.
    click_at: Option<f64>,
}

impl Player {
    /// Size and length of the open video, once ffprobe has told.
    pub fn info(&self) -> Option<VideoInfo> {
        self.playback.as_ref().and_then(|p| p.info().ok())
    }

    pub fn area(&self) -> Option<egui::Rect> {
        self.area
    }

    /// Stops playback and closes the video, sound included.
    pub fn stop(&mut self) {
        self.playback = None;
        self.area = None;
        self.click_at = None;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, file: &MediaFile) -> Option<PlayerAction> {
        if self.playback.as_ref().is_none_or(|p| p.path() != file.path) {
            self.stop();
            self.playback = Some(Playback::open(&file.path, self.muted));
        }
        let playback = self.playback.as_mut()?;
        let info = match playback.info() {
            Ok(info) => info,
            Err(e) => {
                ui.centered_and_justified(|ui| ui.label(format!("This video is shy: {e}")));
                return None;
            }
        };
        let now = ui.input(|i| i.time);
        let mut action = None;

        egui::Panel::bottom("player_controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = if playback.is_playing() { "⏸ Pause" } else { "▶ Play" };
                if ui.button(label).on_hover_text("/ or Space").clicked() {
                    playback.toggle(now);
                }
                // The position bar takes all the room between Play and Mute.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if video::ffplay().is_some() && ui.toggle_value(&mut self.muted, "Mute").changed() {
                        playback.set_muted(self.muted, now);
                    }
                    ui.label(format!("{} / {}", clock(playback.position), clock(info.duration)));
                    ui.spacing_mut().slider_width = ui.available_width();
                    let mut seconds = playback.position.as_secs_f32();
                    let total = info.duration.as_secs_f32().max(0.01);
                    let slider = ui.add(egui::Slider::new(&mut seconds, 0.0..=total).show_value(false));
                    if slider.changed() {
                        playback.seek(Duration::from_secs_f32(seconds), now);
                    }
                });
            });
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
                playback.toggle(now);
            }
            if back {
                playback.seek(playback.position.saturating_sub(SEEK_STEP), now);
            }
            if forward {
                playback.seek((playback.position + SEEK_STEP).min(info.duration), now);
            }
        }
        if let Some(at) = self.click_at {
            let wait = ui.ctx().options(|o| o.input_options.max_double_click_delay);
            if now - at > wait {
                self.click_at = None;
                playback.toggle(now);
            } else {
                ui.ctx().request_repaint_after(Duration::from_secs_f64(wait));
            }
        }

        playback.advance(ui.ctx(), now);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let area = ui.max_rect();
            self.area = Some(area);
            ui.painter().rect_filled(area, 0.0, egui::Color32::BLACK);
            if let Some(texture) = playback.texture() {
                let size = texture.size_vec2();
                let scale = (area.width() / size.x).min(area.height() / size.y);
                let rect = egui::Rect::from_center_size(area.center(), size * scale);
                ui.painter().image(texture.id(), rect, crate::grid::full_uv(), egui::Color32::WHITE);
                let response = ui.interact(rect, ui.scope_id().with("video"), egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Video, true, "Video"));
                if response.double_clicked() {
                    self.click_at = None;
                    action = Some(PlayerAction::Close);
                } else if response.clicked() {
                    self.click_at = Some(now);
                }
            } else {
                ui.centered_and_justified(|ui| ui.spinner());
            }
        });
        action
    }

    /// Saves the frame on screen as `<video name>.<milliseconds>.jpg` next to
    /// the video, the way Picasa named its snapshots.
    pub fn save_frame(&self) -> Option<PlayerAction> {
        let playback = self.playback.as_ref()?;
        let (path, at) = (playback.path(), playback.position);
        let saved = video::extract_frame(path, at, None).and_then(|frame| {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let target = path.with_file_name(format!("{stem}.{}.jpg", at.as_millis()));
            pico_core::image::DynamicImage::ImageRgba8(frame).to_rgb8().save(&target).map_err(std::io::Error::other)?;
            Ok(target)
        });
        Some(match saved {
            Ok(path) => PlayerAction::SavedFrame(path),
            Err(e) => PlayerAction::Error(format!("Could not save the frame: {e}")),
        })
    }

    #[cfg(test)]
    pub fn state(&self) -> (bool, Duration, bool) {
        self.playback.as_ref().map_or((false, Duration::ZERO, false), |p| (p.playing, p.position, p.texture.is_some()))
    }
}

fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}
