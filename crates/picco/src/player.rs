//! Video playback: frames from ffmpeg, sound from ffplay. `Playback` plays a
//! video for the photo view and the slideshow; `Player` adds the controls.

use eframe::egui;
use picco_core::library::MediaFile;
use picco_core::video::{self, Frame, FrameStream, SoundPlayer, VideoInfo};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Decoded frames are at most this wide, even on a larger screen: each
/// frame travels from ffmpeg as raw RGBA, and 4K would be a gigabyte a
/// second.
const MAX_FRAME_WIDTH: u32 = 2560;
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
    /// Frames are decoded at the video's own size, or smaller to fit `screen`
    /// (in pixels), so that they are not scaled twice on the way.
    pub fn open(path: &Path, muted: bool, screen: (u32, u32)) -> Playback {
        let info = video::probe(path).map_err(|e| e.to_string());
        let size = match &info {
            Ok(info) => {
                let (w, h) = (info.width.max(2) as f32, info.height.max(2) as f32);
                let scale = (screen.0 as f32 / w).min(screen.1 as f32 / h).min(MAX_FRAME_WIDTH as f32 / w).min(1.0);
                let even = |side: f32| ((side * scale).round() as u32).max(2) & !1;
                (even(w), even(h))
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

/// The screen's size in pixels, for [`Playback::open`].
pub fn screen_pixels(ctx: &egui::Context) -> (u32, u32) {
    let points = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1920.0, 1080.0));
    let pixels = points * ctx.pixels_per_point();
    (pixels.x.round() as u32, pixels.y.round() as u32)
}

/// Paints `texture` turned `turns` quarter turns clockwise (a video rotated
/// in Picco, as its thumbnail is), as large as fits `area`, centred; returns
/// where it went. Turned by its corners' texture coordinates, so frames need
/// no re-encoding.
pub fn paint_turned(
    painter: &egui::Painter,
    area: egui::Rect,
    texture: &egui::TextureHandle,
    turns: u8,
    tint: egui::Color32,
) -> egui::Rect {
    let mut size = texture.size_vec2();
    if turns % 2 == 1 {
        size = egui::vec2(size.y, size.x);
    }
    let scale = (area.width() / size.x).min(area.height() / size.y);
    let rect = egui::Rect::from_center_size(area.center(), size * scale);
    // Texture corners in screen order (top left, top right, bottom right,
    // bottom left), each turn moving them on by one.
    let uv = [egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)];
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (i, corner) in corners.into_iter().enumerate() {
        mesh.vertices.push(egui::epaint::Vertex { pos: corner, uv: uv[(i + 4 - turns as usize % 4) % 4], color: tint });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    rect
}

/// Seconds the play sign takes to turn into the pause sign, or back.
const TURN: f64 = 0.25;
/// Seconds the pause sign stays after the video starts, and after the mouse
/// moves over it, and takes to fade.
const PAUSE_SHOWN: f64 = 1.0;
const MOVE_SHOWN: f64 = 2.0;
const PAUSE_FADE: f64 = 0.3;

/// The play sign over a paused video, which turns into the pause sign when
/// it plays. The pause sign shows a second after the video starts and two
/// after the mouse last moved over it, then fades; the turn reverses
/// wherever it is when the video stops or starts again.
#[derive(Default)]
pub struct PlaySign {
    /// 0 the play sign, 1 the pause sign, wound towards whichever is due,
    /// and when it was last wound (egui time).
    turn: f32,
    wound_at: f64,
    /// Whether the video plays, and since when.
    playing: bool,
    since: f64,
    /// When the mouse last moved over the playing video.
    moved_at: Option<f64>,
    /// How visible the pause sign was when the video stopped.
    shown_at_stop: f32,
    /// How visible the sign was last painted.
    shown: f32,
}

impl PlaySign {
    /// The sign for a video that plays (or not) from `now`; `show` false
    /// starts it quietly, the pause sign hidden until a key or the mouse.
    pub fn new(playing: bool, now: f64, show: bool) -> Self {
        let turn = if playing { 1.0 } else { 0.0 };
        let since = if show { now } else { f64::NEG_INFINITY };
        Self { turn, wound_at: now, playing, since, moved_at: None, shown_at_stop: 0.0, shown: 0.0 }
    }

    /// Whether the sign is on screen, so it can be clicked.
    pub fn visible(&self) -> bool {
        self.shown > 0.0
    }

    /// Paints the sign over `area` (the picture's panel); true while it moves.
    pub fn paint(
        &mut self,
        ui: &egui::Ui,
        area: egui::Rect,
        playing: bool,
        now: f64,
        still: bool,
        hovered: bool,
    ) -> bool {
        if playing != self.playing {
            self.playing = playing;
            self.since = now;
        }
        let target = if playing { 1.0 } else { 0.0 };
        if still {
            self.turn = target;
        } else {
            let step = ((now - self.wound_at).clamp(0.0, 0.1) / TURN) as f32;
            self.turn = if playing { (self.turn + step).min(1.0) } else { (self.turn - step).max(0.0) };
        }
        self.wound_at = now;
        let moved = ui.input(|i| {
            i.pointer.delta() != egui::Vec2::ZERO && i.pointer.hover_pos().is_some_and(|p| area.contains(p))
        });
        if moved && playing {
            self.moved_at = Some(now);
        }
        let since = if still { f64::INFINITY } else { now - self.since };
        let lasting = |seconds: f64, shown: f64| 1.0 - (((seconds - shown) / PAUSE_FADE) as f32).clamp(0.0, 1.0);
        self.shown = if playing {
            let after_move = self.moved_at.map_or(0.0, |at| lasting(now - at, MOVE_SHOWN));
            self.shown_at_stop = lasting(since - TURN, PAUSE_SHOWN).max(after_move);
            self.shown_at_stop
        } else {
            // Coming back as the play sign, whatever was left of the pause sign.
            (1.0 - self.turn) + self.turn * self.shown_at_stop
        };
        crate::star::play_button(ui.painter(), area.center(), sign_radius(area), self.turn, self.shown, hovered);
        self.turn != target || (playing && self.shown > 0.0)
    }
}

/// How big the play sign is over `area`.
pub fn sign_radius(area: egui::Rect) -> f32 {
    (area.width().min(area.height()) * 0.12).clamp(36.0, 96.0)
}

/// The photo view's video player: the picture and a row of controls.
#[derive(Default)]
pub struct Player {
    playback: Option<Playback>,
    /// Where the picture is shown, as of the last frame.
    area: Option<egui::Rect>,
    muted: bool,
    /// The play and pause sign over the picture.
    sign: PlaySign,
    /// No animations (Options > Animations off).
    pub still: bool,
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
    }

    pub fn show(&mut self, ui: &mut egui::Ui, file: &MediaFile) -> Option<PlayerAction> {
        if self.playback.as_ref().is_none_or(|p| p.path() != file.path) {
            self.stop();
            self.playback = Some(Playback::open(&file.path, self.muted, screen_pixels(ui.ctx())));
            self.sign = PlaySign::new(false, ui.input(|i| i.time), true);
        }
        let playback = self.playback.as_mut()?;
        let info = match playback.info() {
            Ok(info) => info,
            Err(e) => {
                ui.centered_and_justified(|ui| ui.label(tr!("This video is shy: {e}", e = e)));
                return None;
            }
        };
        let now = ui.input(|i| i.time);
        let mut action = None;

        egui::Panel::bottom("player_controls").show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = if playback.is_playing() { tr!("⏸ Pause") } else { tr!("▶ Play") };
                if ui.button(label).on_hover_text(tr!("/ or Space")).clicked() {
                    playback.toggle(now);
                }
                // The position bar takes all the room between Play and Mute.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if video::ffplay().is_some() && ui.toggle_value(&mut self.muted, tr!("Mute")).changed() {
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
        playback.advance(ui.ctx(), now);
        let playing = playback.is_playing();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let area = ui.max_rect();
            self.area = Some(area);
            ui.painter().rect_filled(area, 0.0, egui::Color32::BLACK);
            if let Some(texture) = playback.texture() {
                let rect = paint_turned(ui.painter(), area, texture, file.meta.rotate, egui::Color32::WHITE);
                let response = ui.interact(rect, ui.scope_id().with("video"), egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Video, true, "Video"));
                // A click plays or pauses at once; a second click making it a
                // double-click closes the video anyway.
                if response.double_clicked() {
                    action = Some(PlayerAction::Close);
                } else if response.clicked() {
                    playback.toggle(now);
                }
                // Over the picture, so it takes the click: plays at once.
                let radius = sign_radius(area);
                let button = egui::Rect::from_center_size(area.center(), egui::Vec2::splat(radius * 2.4));
                let hovered = if playing {
                    false
                } else {
                    let response = ui.interact(button, ui.scope_id().with("play"), egui::Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Play"));
                    if response.clicked() {
                        playback.play(now);
                    }
                    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tr!("/ or Space")).hovered()
                };
                if self.sign.paint(ui, area, playing, now, self.still, hovered) {
                    ui.ctx().request_repaint();
                }
            } else {
                ui.centered_and_justified(|ui| ui.spinner());
            }
        });
        action
    }

    /// Saves the frame on screen as `<video name>.<milliseconds>.jpg` next to
    /// the video, the way Picasa named its snapshots.
    /// The frame is turned `turns` quarter turns, as the video is shown.
    pub fn save_frame(&self, turns: u8) -> Option<PlayerAction> {
        let playback = self.playback.as_ref()?;
        let (path, at) = (playback.path(), playback.position);
        let saved = video::extract_frame(path, at, None).and_then(|frame| {
            let frame = match turns % 4 {
                1 => picco_core::image::imageops::rotate90(&frame),
                2 => picco_core::image::imageops::rotate180(&frame),
                3 => picco_core::image::imageops::rotate270(&frame),
                _ => frame,
            };
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let target = path.with_file_name(format!("{stem}.{}.jpg", at.as_millis()));
            picco_core::image::DynamicImage::ImageRgba8(frame)
                .to_rgb8()
                .save(&target)
                .map_err(std::io::Error::other)?;
            Ok(target)
        });
        Some(match saved {
            Ok(path) => PlayerAction::SavedFrame(path),
            Err(e) => PlayerAction::Error(tr!("Could not save the frame: {e}", e = e)),
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
