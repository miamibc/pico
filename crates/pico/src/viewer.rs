//! Full-size view of one photo with the edit panel.
//!
//! The photo is decoded once into a working copy; edits are then rendered
//! from that copy in the background, so slider changes preview quickly.

use crate::grid;
use crate::info;
use crate::player::{Player, PlayerAction};
use crate::star;
use crate::tools::{self, CropTool, Tool, ToolAction};
use eframe::egui;
use pico_core::adjust;
use pico_core::effects::{self, Group, ParamValue};
use pico_core::filters::{Filter, FilterStack, Value};
use pico_core::image::DynamicImage;
use pico_core::image::imageops::FilterType;
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use pico_core::meta::PhotoMeta;
use pico_core::render;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, OnceLock};

/// Longest side of the working copy; larger photos are downscaled for display.
const WORKING_SIDE: u32 = 2560;
/// Longest side of the smaller copy edits are previewed on while a slider
/// moves, so the picture keeps up with it.
const PREVIEW_SIDE: u32 = 1280;

/// Strongest zoom, in screen pixels per photo pixel.
const MAX_ZOOM: f32 = 3.0;
/// The least zoom the slider, Ctrl+wheel and the percent button go to; Fit
/// may show a big photo smaller still.
const MIN_ZOOM: f32 = 0.3;

/// Wheel travel, in points, that moves to the next or previous photo.
const WHEEL_STEP: f32 = 40.0;

pub enum ViewerAction {
    Previous,
    Next,
    First,
    Last,
    /// Switch full-screen viewing (F11) on or off.
    FullScreen(bool),
    Close,
    ToggleStar,
    Rotate(i8),
    SetCaption(Option<String>),
    SetFilters(FilterStack),
    Slideshow,
    /// A new file was created in the folder (a saved video frame).
    FileAdded(PathBuf),
    Status(String),
}

/// Photos prepared ahead on each side of the one on screen.
pub const AHEAD: usize = 2;

/// A photo decoded and rendered with its edits, ready to show.
struct Prepared {
    base: Arc<DynamicImage>,
    /// Working-copy pixels per photo pixel.
    scale: f32,
    texture: (RenderKey, egui::TextureHandle),
    /// The picture with the photo's saved edits, which `texture` may only
    /// preview.
    key: RenderKey,
    /// The smaller copy, when the picture was first made from it.
    preview: Option<PreviewInfo>,
    /// The sharp picture still being made after a preview.
    sharp: Option<(Receiver<Rendered>, Stop)>,
}

/// The working copy decoded, its scale and a first picture of the photo with
/// its edits, or why decoding failed.
type PreparedResult = Result<Made, String>;

struct Made {
    base: Arc<DynamicImage>,
    scale: f32,
    image: egui::ColorImage,
}

/// The smaller copy, its size, and working-copy pixels per pixel of it.
type PreviewInfo = (PreviewCopy, [u32; 2], f32);

/// Tells a render in the background to give up once dropped.
struct Stop(Arc<AtomicBool>);

impl Stop {
    fn new() -> (Self, Arc<AtomicBool>) {
        let flag = Arc::new(AtomicBool::new(false));
        (Self(Arc::clone(&flag)), flag)
    }
}

impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// A rendered picture.
type Rendered = egui::ColorImage;

/// Width and height in pixels.
type Size = (u32, u32);

/// What a texture shows: the file plus the edits it was rendered with.
type RenderKey = (PathBuf, String);
/// The smaller copy for previews, made by the first render that wants it.
type PreviewCopy = Arc<OnceLock<DynamicImage>>;
/// The start of a photo's edit stack and the picture it makes, before the
/// rotation.
type KeptRender = (PathBuf, Vec<Filter>, Arc<DynamicImage>);
/// Pictures kept for the smaller copy and for the working copy.
type KeptRenders = Arc<Mutex<[Vec<KeptRender>; 2]>>;
/// How many pictures are kept for each copy.
const KEPT: usize = 4;

/// How the photo is zoomed; the user picks it, and it stays from photo to photo.
#[derive(Default, Clone, Copy, PartialEq)]
enum ZoomMode {
    /// The whole photo in the window.
    #[default]
    Fit,
    /// One photo pixel per screen pixel.
    Actual,
    /// The user's own zoom, from the slider, Ctrl+wheel or the percent button.
    Custom,
}

/// Room Fit, 1:1 and the zoom slider with its value take in the bar.
const ZOOM_CONTROLS_WIDTH: f32 = 280.0;

#[derive(Default)]
pub struct Viewer {
    /// Working copy and its size relative to the original photo.
    base: Option<(PathBuf, Arc<DynamicImage>, f32)>,
    /// The working copy and picture being made for the photo on screen.
    base_loading: Option<(RenderKey, Receiver<PreparedResult>)>,
    /// Neighbours of the photo on screen (and the one just left), prepared
    /// so paging through shows each at once, without a blank frame.
    ahead: std::collections::HashMap<PathBuf, Result<Prepared, String>>,
    ahead_jobs: std::collections::HashMap<PathBuf, (RenderKey, Receiver<PreparedResult>)>,
    texture: Option<(RenderKey, egui::TextureHandle)>,
    /// The picture being rendered, whether it previews edits not saved yet,
    /// and what makes it give up when dropped.
    rendering: Option<(RenderKey, Receiver<Rendered>, bool, Stop)>,
    /// The working copy made smaller for previews, made once when first
    /// wanted, and how many working-copy pixels one of its pixels stands for.
    preview: Option<(PathBuf, PreviewCopy, [u32; 2], f32)>,
    /// Starts of edit stacks already rendered: each render begins from the
    /// longest one it shares, so trying out the edit on top, or adding one,
    /// does not apply all the edits under it again.
    kept: KeptRenders,
    error: Option<String>,
    caption: String,
    caption_for: Option<PathBuf>,
    /// Uncommitted edits while a slider is being dragged.
    draft: Option<(PathBuf, FilterStack)>,
    show_edit_panel: bool,
    /// The open Crop or Straighten tool and the file it belongs to.
    tool: Option<(PathBuf, Tool)>,
    player: Player,
    /// Screen pixels per photo pixel (1.0 is 1:1); `None` fits the window.
    zoom: Option<f32>,
    /// Fit, 1:1 or a zoom of the user's: changed only by the user, kept from
    /// photo to photo. `zoom` follows from it each frame.
    zoom_mode: ZoomMode,
    /// The user's zoom, for [`ZoomMode::Custom`] and the percent button.
    custom_zoom: Option<f32>,
    /// Offset of the zoomed photo's centre from the middle of the view.
    pan: egui::Vec2,
    /// The zoom that fits the photo, as of the last frame.
    fit_zoom: f32,
    /// Wheel travel not yet turned into a photo change.
    wheel: f32,
    /// The file the zoom, pan and detail belong to.
    view_for: Option<PathBuf>,
    /// The photo at full resolution with its edits, for zooming past the
    /// working copy.
    detail: Option<(RenderKey, egui::TextureHandle)>,
    detail_job: Option<(RenderKey, Receiver<egui::ColorImage>)>,
    /// The photo without its edits (rotation kept), for before/after.
    original: Option<(RenderKey, egui::TextureHandle)>,
    original_job: Option<(RenderKey, Receiver<egui::ColorImage>)>,
    /// Whether "Original" is held down this frame.
    show_original: bool,
    /// Full-screen viewing: just the photo, no panels.
    full_screen: bool,
    /// Edits taken off by Undo, newest last, for Redo. Any other edit
    /// clears them, as in every editor.
    redo: Vec<Filter>,
    /// Pixel size from the file header, for the info line.
    header_size: Option<(PathBuf, Option<Size>)>,
    /// Where the Star button was this frame, for the star to fly from.
    star_button: Option<egui::Rect>,
    /// The star flying onto a photo just starred.
    star_flight: Option<star::Flight>,
    /// The badge vanishing from a photo just unstarred.
    star_puff: Option<star::Puff>,
    /// The space the photo or video is shown in.
    photo_area: Option<egui::Rect>,
    /// Starring shows no flying star (Options > Animations off).
    pub still: bool,
    /// The photo's thumbnail, with its edits, shown until the photo is ready.
    pub placeholder: Option<egui::TextureHandle>,
    /// The crop aspect last picked, for the next crop.
    crop_shape: tools::CropShape,
}

impl Viewer {
    pub fn show(&mut self, ui: &mut egui::Ui, file: &MediaFile, position: (usize, usize)) -> Option<ViewerAction> {
        self.star_button = None;
        let action = self.show_photo(ui, file, position);
        let now = ui.input(|i| i.time);
        let Some(area) = self.photo_area else { return action };
        if matches!(action, Some(ViewerAction::ToggleStar)) && !self.still {
            if file.meta.star {
                self.star_flight = None;
                self.star_puff = Some(star::Puff::new(now));
            } else {
                let from = self.star_button.map_or(area.center_bottom(), |r| r.center());
                self.star_flight = Some(star::Flight::new(now, from));
                self.star_puff = None;
            }
        }
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("star")));
        // The badge is painted from its left bottom corner (see `picture`).
        let badge = area.left_bottom() + egui::vec2(24.0, -26.0);
        if let Some(flight) = &self.star_flight {
            if flight.paint(&painter, now, area, badge) {
                ui.ctx().request_repaint();
            } else {
                self.star_flight = None;
            }
        }
        if let Some(puff) = &self.star_puff {
            if puff.paint(&painter, now, badge) {
                ui.ctx().request_repaint();
            } else {
                self.star_puff = None;
            }
        }
        action
    }

    /// Stops a playing video, when the viewer is left.
    pub fn stop_video(&mut self) {
        self.player.stop();
    }

    #[cfg(test)]
    pub fn star_flying(&self) -> bool {
        self.star_flight.is_some()
    }

    #[cfg(test)]
    pub fn star_puffing(&self) -> bool {
        self.star_puff.is_some()
    }

    fn show_photo(&mut self, ui: &mut egui::Ui, file: &MediaFile, position: (usize, usize)) -> Option<ViewerAction> {
        // A draft ends with another photo, or once the app saved it.
        if self.draft.as_ref().is_some_and(|(path, stack)| path != &file.path || stack == &file.meta.filters) {
            self.draft = None;
        }
        if self.tool.as_ref().is_some_and(|(path, _)| path != &file.path) {
            self.tool = None;
        }
        if self.view_for.as_ref() != Some(&file.path) {
            self.view_for = Some(file.path.clone());
            // A star flying for the photo before would land on this one.
            self.star_flight = None;
            self.star_puff = None;
            // The zoom mode stays; the new photo starts in its middle.
            self.pan = egui::Vec2::ZERO;
            self.wheel = 0.0;
            self.detail = None;
            self.detail_job = None;
            self.original = None;
            self.original_job = None;
            self.redo.clear();
            let size =
                (file.kind == MediaKind::Photo).then(|| pico_core::image::image_dimensions(&file.path).ok()).flatten();
            self.header_size = Some((file.path.clone(), size));
        }
        if self.caption_for.as_ref() != Some(&file.path) {
            self.caption = file.meta.caption.clone().unwrap_or_default();
            self.caption_for = Some(file.path.clone());
        }
        let mut action = None;

        if !self.full_screen {
            egui::Panel::bottom("viewer_tools").show(ui, |ui| {
                if let Some(a) = self.toolbar(ui, file, position) {
                    action = Some(a);
                }
            });
            egui::Panel::top("viewer_info").show(ui, |ui| self.info_line(ui, file));
        }
        if file.kind == MediaKind::Video {
            // Videos get the player instead of the photo editor.
            self.player.still = self.still;
            let player_action = self.player.show(ui, file);
            self.photo_area = self.player.area();
            if let Some(area) = self.photo_area {
                self.paint_star_badge(ui, area, file.meta.star);
            }
            return action.or(player_action.map(from_player)).or_else(|| self.navigation_keys(ui, file));
        }
        self.player.stop();
        if !self.full_screen && (self.show_edit_panel || self.tool.is_some()) {
            egui::Panel::left("edit_panel").resizable(false).default_size(240.0).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                    if let Some(a) = self.edit_panel(ui, file) {
                        action = Some(a);
                    }
                });
            });
        }
        // Until the app saves the new edits, they are what to show; else the
        // picture before them would be rendered for a moment.
        if let Some(ViewerAction::SetFilters(stack)) = &action {
            self.draft = Some((file.path.clone(), stack.clone()));
        }

        let meta = self.effective_meta(file);
        self.update_rendering(ui.ctx(), file, &meta);
        self.update_original(ui.ctx(), file);
        let picture_action = egui::CentralPanel::no_frame().show(ui, |ui| self.picture(ui, &meta)).inner;
        self.update_detail(ui.ctx(), file, &meta);
        if action.is_none() && picture_action.is_some() {
            return picture_action;
        }

        // Shortcuts only when no text field is taking the keys.
        let keys_free = action.is_none() && !ui.ctx().egui_wants_keyboard_input();
        // Tool panels have no text fields, so Enter and Esc always reach the tool,
        // even while one of its sliders has keyboard focus.
        if action.is_none() && self.tool.is_some() {
            let (apply, cancel) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            let tool_action = if cancel {
                Some(ToolAction::Cancel)
            } else if apply {
                match &self.tool {
                    Some((_, Tool::Crop(crop))) => crop.apply(file.meta.rotate),
                    Some((_, Tool::Straighten { amount })) => {
                        (*amount != 0.0).then(|| ToolAction::Apply(tools::straighten_filter(*amount)))
                    }
                    Some((_, Tool::Effect { spec, params })) => {
                        Some(ToolAction::Apply(effects::to_filter(spec, params)))
                    }
                    None => None,
                }
            } else {
                None
            };
            return tool_action.and_then(|a| self.finish_tool(a, file));
        }
        if keys_free {
            action = self.navigation_keys(ui, file);
        }
        action
    }

    /// Keys shared by photos and videos, ignored while a text field has focus
    /// (which has its own Ctrl+Z).
    fn navigation_keys(&mut self, ui: &egui::Ui, file: &MediaFile) -> Option<ViewerAction> {
        if ui.ctx().egui_wants_keyboard_input() {
            return None;
        }
        let (undo, redo) = ui.input(|i| {
            let command = i.modifiers.command;
            (
                command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        if file.kind != MediaKind::Video {
            if undo {
                return self.undo(file);
            }
            if redo {
                return self.redo(file);
            }
        }
        ui.input(|i| {
            let f = i.key_pressed(egui::Key::F) && i.modifiers.is_none();
            if f || i.key_pressed(egui::Key::F11) || (self.full_screen && i.key_pressed(egui::Key::Escape)) {
                Some(ViewerAction::FullScreen(!self.full_screen))
            } else if i.key_pressed(egui::Key::Escape) {
                Some(ViewerAction::Close)
            } else if i.key_pressed(egui::Key::Home) {
                Some(ViewerAction::First)
            } else if i.key_pressed(egui::Key::End) {
                Some(ViewerAction::Last)
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                Some(ViewerAction::Previous)
            } else if i.key_pressed(egui::Key::ArrowRight) {
                Some(ViewerAction::Next)
            } else if i.key_pressed(egui::Key::S) && i.modifiers.is_none() {
                Some(ViewerAction::ToggleStar)
            } else if i.key_pressed(egui::Key::R) && i.modifiers.command {
                Some(ViewerAction::Rotate(if i.modifiers.shift { -1 } else { 1 }))
            } else {
                None
            }
        })
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, file: &MediaFile, position: (usize, usize)) -> Option<ViewerAction> {
        let mut action = None;
        // Wraps onto a second row where the window is too narrow for it all,
        // rather than running under the Properties panel.
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr!("⬅ Back to Library")).on_hover_text("Esc").clicked() {
                action = Some(ViewerAction::Close);
            }
            // Greyed out for videos rather than hidden, so the bar does not jump.
            let photo = file.kind != MediaKind::Video;
            if ui
                .add_enabled(photo, egui::Button::selectable(photo && self.show_edit_panel, tr!("Edit")))
                .on_hover_text(tr!("Show the edit panel"))
                .on_disabled_hover_text(tr!("Videos are not edited in Pico"))
                .clicked()
            {
                self.show_edit_panel = !self.show_edit_panel;
            }
            if ui.button(tr!("▶ Slideshow")).on_hover_text("Ctrl+4").clicked() {
                action = Some(ViewerAction::Slideshow);
            }
            if ui.button("⛶").on_hover_text(tr!("Full screen (F or F11; Esc to leave)")).clicked() {
                action = Some(ViewerAction::FullScreen(true));
            }
            ui.separator();
            if ui.button("◀").on_hover_text(tr!("Previous (Left arrow; Home for the first)")).clicked() {
                action = Some(ViewerAction::Previous);
            }
            ui.label(format!("{} / {}", position.0 + 1, position.1));
            if ui.button("▶").on_hover_text(tr!("Next (Right arrow; End for the last)")).clicked() {
                action = Some(ViewerAction::Next);
            }
            ui.separator();
            let star = if file.meta.star { tr!("★ Starred") } else { tr!("☆ Star") };
            let button = ui.button(star).on_hover_text("S");
            self.star_button = Some(button.rect);
            if button.clicked() {
                action = Some(ViewerAction::ToggleStar);
            }
            if ui.button("⟲").on_hover_text(tr!("Rotate counterclockwise (Ctrl+Shift+R)")).clicked() {
                action = Some(ViewerAction::Rotate(-1));
            }
            if ui.button("⟳").on_hover_text(tr!("Rotate clockwise (Ctrl+R)")).clicked() {
                action = Some(ViewerAction::Rotate(1));
            }
            // Shown before the photo is ready too, so the bar keeps its rows.
            if file.kind != MediaKind::Video && self.tool.is_none() {
                let ready = self.texture.is_some();
                let edited = ready && !file.meta.filters.0.is_empty();
                let button = ui
                    .add_enabled(edited, egui::Button::new(tr!("Original")))
                    .on_hover_text(tr!("Hold to see the photo without its edits (or hold \\)"))
                    .on_disabled_hover_text(tr!("No edits to compare with"));
                let key_held = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_down(egui::Key::Backslash));
                self.show_original = edited && (button.is_pointer_button_down_on() || key_held);
                ui.separator();
                // Fit, 1:1 and the slider stay together on one row.
                if ui.available_size_before_wrap().x < ZOOM_CONTROLS_WIDTH {
                    ui.end_row();
                }
                self.zoom_controls(ui, ready);
            } else {
                self.show_original = false;
            }
            if file.kind == MediaKind::Video
                && ui
                    .button(tr!("Save Frame as Photo"))
                    .on_hover_text(tr!("Save the frame on screen next to the video"))
                    .clicked()
            {
                action = self.player.save_frame(file.meta.rotate).map(from_player);
            }
            ui.separator();
            // The caption takes the rest of the row, else a row of its own.
            if ui.available_size_before_wrap().x < 200.0 {
                ui.end_row();
            }
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.caption)
                    .hint_text(tr!("Add a caption. Future you will thank you."))
                    .desired_width(ui.available_size_before_wrap().x),
            );
            let committed = edit.lost_focus() || (edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            let caption = Some(self.caption.trim().to_owned()).filter(|c| !c.is_empty());
            if committed && caption != file.meta.caption {
                action = Some(ViewerAction::SetCaption(caption));
            }
        });
        action
    }

    /// Name, date taken, pixel size and file size, like the line above the
    /// photo in Picasa.
    fn info_line(&self, ui: &mut egui::Ui, file: &MediaFile) {
        let mut parts = vec![file.name()];
        parts.push(match &file.taken {
            Some(taken) => crate::i18n::date_label(taken),
            None => tr!("file date {date}", date = crate::i18n::date_label(&file.sort_date())),
        });
        if file.kind == MediaKind::Video {
            if let Some(video) = self.player.info() {
                parts.push(format!("{} × {}", video.width, video.height));
                let secs = video.duration.as_secs();
                parts.push(format!("{}:{:02}", secs / 60, secs % 60));
            }
        } else if let Some(((w, h), original)) = self.pixel_size(file) {
            parts.push(match original {
                Some((ow, oh)) => {
                    tr!("{size} (original {original})", size = format!("{w} × {h}"), original = format!("{ow} × {oh}"))
                }
                None => format!("{w} × {h}"),
            });
        }
        parts.push(info::human_size(file.size));
        ui.label(egui::RichText::new(parts.join("   ·   ")).weak()).on_hover_text(file.path.display().to_string());
    }

    /// The photo's size in pixels as edited, and its original size when a crop
    /// changed it; known once the photo is decoded.
    fn pixel_size(&self, file: &MediaFile) -> Option<(Size, Option<Size>)> {
        let (path, base, scale) = self.base.as_ref().filter(|(path, ..)| path == &file.path)?;
        let (key, texture) = self.texture.as_ref().filter(|(key, _)| &key.0 == path)?;
        // The file header has the exact size; the working copy may be scaled
        // down. EXIF orientation can swap the header's sides.
        let header = self.header_size.as_ref().filter(|(p, _)| p == path).and_then(|(_, size)| *size);
        let original = match header {
            Some((w, h)) if (w > h) != (base.width() > base.height()) && w != h => (h, w),
            Some(size) => size,
            None => ((base.width() as f32 / scale).round() as u32, (base.height() as f32 / scale).round() as u32),
        };
        let per_px = original.0 as f32 / base.width() as f32;
        let [w, h] = self.shown_size(key, texture).into();
        let edited = ((w * per_px).round() as u32, (h * per_px).round() as u32);
        // Turning the photo swaps its sides without changing its size.
        let turned = key.1.starts_with('1') || key.1.starts_with('3');
        let same = if turned { (original.1, original.0) } else { original };
        let slack = per_px.ceil() as u32 + 1;
        let near = |a: u32, b: u32| a.abs_diff(b) <= slack;
        Some(if near(edited.0, same.0) && near(edited.1, same.1) { (same, None) } else { (edited, Some(same)) })
    }

    /// Fit, 1:1 and the zoom in percent as one switch of three, then the
    /// zoom slider; greyed out until the photo is `ready`. About
    /// [`ZOOM_CONTROLS_WIDTH`] wide.
    fn zoom_controls(&mut self, ui: &mut egui::Ui, ready: bool) {
        let fit = self.fit_zoom.max(0.01);
        let current = self.zoom.unwrap_or(fit);
        // The zoom on screen, whichever mode made it: the fit's own in Fit.
        let percent_label = format!("{:.0}%", current * 100.0);
        ui.scope(|ui| {
            // Joined: square inner corners, a hairline between the parts.
            ui.spacing_mut().item_spacing.x = 1.0;
            let round = ui.visuals().widgets.inactive.corner_radius;
            let left = egui::CornerRadius { ne: 0, se: 0, ..round };
            let right = egui::CornerRadius { nw: 0, sw: 0, ..round };
            // Framed even when off, so the three read as one button; the
            // percent keeps its width as the number changes.
            let segment = |selected: bool, text: &'static str, corners: egui::CornerRadius| {
                egui::Button::new(text).selected(selected).corner_radius(corners).min_size(egui::vec2(44.0, 0.0))
            };
            if ui
                .add_enabled(ready, segment(self.zoom_mode == ZoomMode::Fit, "Fit", left))
                .on_hover_text(tr!("Fit the photo in the window"))
                .clicked()
            {
                self.zoom_mode = ZoomMode::Fit;
                self.pan = egui::Vec2::ZERO;
            }
            if ui
                .add_enabled(ready, segment(self.zoom_mode == ZoomMode::Actual, "1:1", egui::CornerRadius::ZERO))
                .on_hover_text(tr!("One photo pixel per screen pixel"))
                .clicked()
            {
                self.scale_pan(1.0);
                self.zoom_mode = ZoomMode::Actual;
            }
            // The third part holds the percent and the slider, framed as
            // one with the two before.
            let custom = self.zoom_mode == ZoomMode::Custom;
            let visuals = ui.visuals();
            let fill = if custom { visuals.selection.bg_fill } else { visuals.widgets.inactive.weak_bg_fill };
            let height = ui.spacing().interact_size.y;
            ui.add_enabled_ui(ready, |ui| {
                egui::Frame::new()
                    .fill(fill)
                    .corner_radius(right)
                    .inner_margin(egui::Margin { left: 4, right: 10, top: 0, bottom: 0 })
                    .show(ui, |ui| {
                        ui.set_min_height(height);
                        ui.spacing_mut().item_spacing.x = 4.0;
                        // The slider's rail (and resting handle) in a shade of
                        // the part's own fill, so it reads alike on the light
                        // and the dark theme and on the highlight.
                        let text = ui.visuals().text_color();
                        ui.visuals_mut().widgets.inactive.bg_fill = fill.lerp_to_gamma(text, 0.3);
                        let percent_button = egui::Button::new(&percent_label)
                            .selected(custom)
                            .frame(false)
                            .min_size(egui::vec2(40.0, height));
                        if ui.add(percent_button).on_hover_text(tr!("Keep this zoom from photo to photo")).clicked() {
                            // The zoom it shows, as it is now.
                            self.set_custom_zoom(current);
                        }
                        let mut percent = current * 100.0;
                        let slider = egui::Slider::new(&mut percent, MIN_ZOOM * 100.0..=MAX_ZOOM * 100.0)
                            .logarithmic(true)
                            .show_value(false);
                        // A change smaller than a whole percent is the
                        // slider's rounding, not the user.
                        if ui
                            .add(slider)
                            .on_hover_text(tr!("Zoom (Ctrl+wheel; drag the photo to look around)"))
                            .changed()
                            && (percent - current * 100.0).abs() >= 0.5
                        {
                            self.set_custom_zoom(percent / 100.0);
                        }
                    });
            });
        });
    }

    /// Switches to the user's own zoom, around the middle of the view.
    fn set_custom_zoom(&mut self, zoom: f32) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.scale_pan(zoom);
        self.custom_zoom = Some(zoom);
        self.zoom_mode = ZoomMode::Custom;
    }

    /// Keeps what is in the middle of the view there as the zoom goes to `zoom`.
    fn scale_pan(&mut self, zoom: f32) {
        let old = self.zoom.unwrap_or(self.fit_zoom.max(0.01));
        self.pan *= zoom / old;
    }

    /// Takes the last saved edit off the stack.
    fn undo(&mut self, file: &MediaFile) -> Option<ViewerAction> {
        let mut stack = file.meta.filters.clone();
        let filter = stack.0.pop()?;
        self.redo.push(filter);
        self.draft = None;
        Some(ViewerAction::SetFilters(stack))
    }

    /// Puts the last undone edit back.
    fn redo(&mut self, file: &MediaFile) -> Option<ViewerAction> {
        let filter = self.redo.pop()?;
        let mut stack = file.meta.filters.clone();
        stack.0.push(filter);
        self.draft = None;
        Some(ViewerAction::SetFilters(stack))
    }

    /// Ends the open tool, turning an applied tool into a new edit on the stack.
    fn finish_tool(&mut self, action: ToolAction, file: &MediaFile) -> Option<ViewerAction> {
        self.tool = None;
        match action {
            ToolAction::Apply(filter) => {
                self.redo.clear();
                let mut stack = file.meta.filters.clone();
                stack.0.push(filter);
                Some(ViewerAction::SetFilters(stack))
            }
            ToolAction::Cancel => None,
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui, file: &MediaFile) -> Option<ViewerAction> {
        let image_px = self.texture.as_ref().map_or(egui::vec2(1.0, 1.0), |(key, t)| self.shown_size(key, t));
        let tool_action = match &mut self.tool {
            Some((_, Tool::Crop(crop))) => {
                crop.screen = ui.ctx().input(|i| i.viewport().monitor_size).unwrap_or(ui.ctx().content_rect().size());
                let action = crop.controls(ui, image_px, file.meta.rotate);
                self.crop_shape = crop.shape();
                Some(action)
            }
            Some((_, Tool::Straighten { amount })) => Some(tools::straighten_controls(ui, amount)),
            Some((_, Tool::Effect { spec, params })) => Some(tools::effect_controls(ui, spec, params)),
            None => None,
        };
        if let Some(tool_action) = tool_action {
            return tool_action.and_then(|a| self.finish_tool(a, file));
        }

        let committed = file.meta.filters.clone();
        let mut stack = self.draft.as_ref().map_or_else(|| committed.clone(), |(_, s)| s.clone());
        let mut commit = false;

        ui.heading(tr!("Basic Fixes"));
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr!("Crop")).clicked() {
                self.draft = None;
                self.tool = Some((file.path.clone(), Tool::Crop(CropTool::new(self.crop_shape))));
            }
            if ui.button(tr!("Straighten")).clicked() {
                self.draft = None;
                self.tool = Some((file.path.clone(), Tool::Straighten { amount: 0.0 }));
            }
            for (label, id) in [
                (tr!("I'm Feeling Lucky"), "enhance"),
                (tr!("Auto Contrast"), "autolight"),
                (tr!("Auto Color"), "autocolor"),
            ] {
                if ui.button(label).clicked() {
                    stack.0.push(Filter::effect(id, []));
                    commit = true;
                }
            }
        });

        ui.separator();
        ui.heading(tr!("Tuning"));
        // The sliders edit the newest Tuning edit while it is on top of the
        // stack; after any other edit they start a new one, as Picasa does.
        let on_top = stack.0.last().is_some_and(|f| f.id == "finetune2");
        let current = stack.0.last().filter(|_| on_top);
        let mut values = [0usize, 1, 2, 4].map(|i| current.and_then(|f| f.float(i)).unwrap_or(0.0));
        let neutral = current.and_then(|f| f.color(3)).unwrap_or(0);
        let mut changed = false;
        for (value, (label, range)) in values.iter_mut().zip([
            (tr!("Fill Light"), 0.0..=1.0),
            (tr!("Highlights"), 0.0..=0.48),
            (tr!("Shadows"), 0.0..=0.48),
            (tr!("Color Temperature"), -1.0..=1.0),
        ]) {
            let response = ui.add(egui::Slider::new(value, range).text(label).show_value(false));
            changed |= response.changed();
            // Save when a drag ends, or right away for keyboard and accessibility changes.
            commit |= response.drag_stopped() || (response.changed() && !response.dragged());
        }
        if changed {
            let [fill, highlights, shadows, temperature] = values;
            let tuning = Filter::effect(
                "finetune2",
                [
                    Value::Float(fill),
                    Value::Float(highlights),
                    Value::Float(shadows),
                    Value::Color(neutral),
                    Value::Float(temperature),
                ],
            );
            match stack.0.last_mut() {
                Some(last) if on_top => *last = tuning,
                _ => stack.0.push(tuning),
            }
        }

        ui.separator();
        ui.heading(tr!("Effects"));
        let mut open_effect = None;
        ui.horizontal_wrapped(|ui| {
            for spec in effects::SPECS.iter().filter(|s| s.group == Group::Picasa) {
                if ui.button(crate::i18n::tr(spec.label)).clicked() {
                    open_effect = Some(spec);
                }
            }
        });
        egui::CollapsingHeader::new(tr!("Creative Kit")).default_open(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for spec in effects::SPECS.iter().filter(|s| s.group == Group::Creative && s.id != "ReanimatedEyeColor")
                {
                    if ui.button(crate::i18n::tr(spec.label)).clicked() {
                        open_effect = Some(spec);
                    }
                }
            });
        });
        if let Some(spec) = open_effect {
            if spec.params.is_empty() {
                stack.0.push(effects::to_filter(spec, &[]));
                commit = true;
            } else {
                self.draft = None;
                self.tool = Some((file.path.clone(), Tool::Effect { spec, params: effects::defaults(spec) }));
                return None;
            }
        }

        ui.separator();
        let count = stack.0.len();
        let hidden = stack
            .0
            .iter()
            .filter(|f| !adjust::is_supported(&f.id) && !matches!(f.id.as_str(), "moviestart" | "movieend"));
        let hidden = hidden.count();
        ui.label(match count {
            0 => tr!("No edits yet. The photo is perfect, apparently.").to_owned(),
            n => trn!("{n} edit", "{n} edits", n),
        });
        if hidden > 0 {
            ui.label(egui::RichText::new(tr!("{n} not shown yet (kept in the file)", n = hidden)).weak())
                .on_hover_text(tr!("Pico keeps edits it cannot draw yet, so Picasa still sees them."));
        }
        let mut history = None;
        ui.horizontal(|ui| {
            let undo = ui.add_enabled(!committed.0.is_empty(), egui::Button::new(tr!("Undo")));
            if undo.on_hover_text(tr!("Remove the last edit (Ctrl+Z)")).clicked() {
                history = self.undo(file);
            }
            let redo = ui.add_enabled(!self.redo.is_empty(), egui::Button::new(tr!("Redo")));
            if redo.on_hover_text(tr!("Bring the edit back (Ctrl+Y)")).clicked() {
                history = self.redo(file);
            }
        });
        if ui.add_enabled(count > 0, egui::Button::new(tr!("Undo All Edits"))).clicked() {
            // Redo brings them back one by one, oldest first.
            self.redo.extend(committed.0.iter().rev().cloned());
            self.draft = None;
            history = Some(ViewerAction::SetFilters(FilterStack::default()));
        }

        if history.is_some() {
            return history;
        }
        if commit {
            self.draft = None;
            if stack != committed {
                self.redo.clear();
                return Some(ViewerAction::SetFilters(stack));
            }
            return None;
        }
        self.draft = (stack != committed).then(|| (file.path.clone(), stack));
        None
    }

    fn effective_meta(&self, file: &MediaFile) -> PhotoMeta {
        let mut meta = file.meta.clone();
        if let Some((_, stack)) = &self.draft {
            meta.filters = stack.clone();
        }
        match &self.tool {
            Some((_, Tool::Straighten { amount })) if *amount != 0.0 => {
                meta.filters.0.push(tools::straighten_filter(*amount));
            }
            Some((_, Tool::Effect { spec, params })) => meta.filters.0.push(effects::to_filter(spec, params)),
            _ => {}
        }
        meta
    }

    /// Draws the photo and handles the mouse on it: double-click (back to the
    /// library, like in Picasa), the wheel (next or previous photo), Ctrl+wheel
    /// or pinch (zoom) and dragging (look around a zoomed photo). Tools get
    /// the mouse to themselves and always see the whole photo.
    fn picture(&mut self, ui: &mut egui::Ui, meta: &PhotoMeta) -> Option<ViewerAction> {
        let area = ui.max_rect();
        self.photo_area = Some(area);
        let background = ui.interact(area, ui.scope_id().with("backdrop"), egui::Sense::click());
        let mut double_clicked = false;
        let mut action = None;
        ui.painter().rect_filled(area, 0.0, egui::Color32::from_gray(16));
        // Handles are cheap to clone and keep `self` free for the zoom state.
        if let Some((key, texture)) = self.texture.clone() {
            let size = self.shown_size(&key, &texture);
            // Working-copy pixels per photo pixel.
            let base_scale = match &self.base {
                Some((path, _, scale)) if path == &key.0 => *scale,
                _ => 1.0,
            };
            // Small photos are not blown up, except while cropping, where room to aim helps.
            let max_scale = if matches!(self.tool, Some((_, Tool::Crop(_)))) { f32::INFINITY } else { 1.0 };
            let fit = (area.width() / size.x).min(area.height() / size.y).min(max_scale);
            self.fit_zoom = fit * base_scale;
            let zoomable = self.tool.is_none();
            // The zoom the mode asks for; tools show the photo fitted.
            self.zoom = match self.zoom_mode {
                _ if !zoomable => None,
                ZoomMode::Fit => None,
                ZoomMode::Actual => Some(1.0),
                ZoomMode::Custom => Some(self.custom_zoom.unwrap_or(self.fit_zoom)),
            };
            if !zoomable {
                self.pan = egui::Vec2::ZERO;
            }
            let hovered = ui.rect_contains_pointer(area);
            if zoomable && hovered {
                let (zoom_delta, scroll, pointer, command) = ui
                    .input(|i| (i.zoom_delta(), i.smooth_scroll_delta().y, i.pointer.hover_pos(), i.modifiers.command));
                if zoom_delta != 1.0 {
                    let old = self.zoom.unwrap_or(self.fit_zoom);
                    // Keep the point under the pointer in place.
                    let pivot = pointer.map_or(egui::Vec2::ZERO, |p| p - area.center());
                    let target = (old * zoom_delta).clamp(MIN_ZOOM, MAX_ZOOM);
                    self.pan = pivot - (pivot - self.pan) * (target / old);
                    self.custom_zoom = Some(target);
                    self.zoom_mode = ZoomMode::Custom;
                    self.zoom = Some(target);
                } else if !command && scroll != 0.0 {
                    self.wheel += scroll;
                    if self.wheel.abs() >= WHEEL_STEP {
                        action = Some(if self.wheel > 0.0 { ViewerAction::Previous } else { ViewerAction::Next });
                        self.wheel = 0.0;
                    }
                }
            }
            let scale = self.zoom.map_or(fit, |zoom| zoom / base_scale);
            let shown = size * scale;
            let slack = ((shown - area.size()) / 2.0).max(egui::Vec2::ZERO);
            self.pan = self.pan.clamp(-slack, slack);
            let rect = egui::Rect::from_center_size(area.center() + self.pan, shown);
            // The full-resolution picture, once ready, replaces the magnified working copy.
            let detail = self.detail.as_ref().filter(|(detail_key, _)| detail_key == &key && self.zoom.is_some());
            let shown_texture = detail.map_or(texture.id(), |(_, t)| t.id());
            match self.original.as_ref().filter(|(original_key, _)| self.show_original && original_key.0 == key.0) {
                // Before/after: the unedited photo, at the same zoom, around
                // the same centre; a crop makes it bigger than the edited one.
                Some((_, original)) => {
                    let size = original.size_vec2();
                    let fit = (area.width() / size.x).min(area.height() / size.y).min(1.0);
                    let scale = if self.zoom.is_some() { scale } else { fit };
                    let rect = egui::Rect::from_center_size(rect.center(), size * scale);
                    ui.painter().with_clip_rect(area).image(original.id(), rect, grid::full_uv(), egui::Color32::WHITE);
                    let at = area.left_top() + egui::vec2(12.0, 10.0);
                    let font = egui::FontId::proportional(16.0);
                    let galley = ui.painter().layout_no_wrap(tr!("Original").to_owned(), font, egui::Color32::WHITE);
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(at, galley.size()).expand(6.0),
                        4.0,
                        egui::Color32::from_black_alpha(160),
                    );
                    ui.painter().galley(at, galley, egui::Color32::WHITE);
                }
                None => {
                    ui.painter().with_clip_rect(area).image(shown_texture, rect, grid::full_uv(), egui::Color32::WHITE);
                }
            }
            let response =
                ui.interact(rect.intersect(area), ui.scope_id().with("photo"), egui::Sense::click_and_drag());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Photo"));
            double_clicked = response.double_clicked();
            if zoomable && self.zoom.is_some() {
                if response.dragged() {
                    self.pan += response.drag_delta();
                }
                let icon = if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab };
                response.clone().on_hover_cursor(icon);
            }
            match &mut self.tool {
                Some((_, Tool::Crop(crop))) => {
                    crop.interact(&response, rect, size);
                    crop.paint(ui.painter(), rect);
                }
                Some((_, Tool::Straighten { .. })) => tools::paint_grid(ui.painter(), rect),
                Some((_, Tool::Effect { params, .. })) => {
                    let to_screen = |p: egui::Pos2| rect.min + p.to_vec2() * rect.size();
                    for value in params.iter_mut() {
                        if let ParamValue::Point(x, y) = value {
                            if response.clicked()
                                && let Some(pos) = response.interact_pointer_pos()
                            {
                                let shown = ((pos - rect.min) / rect.size()).to_pos2();
                                let source = tools::unrotate_point(shown, meta.rotate);
                                (*x, *y) = (source.x.clamp(0.0, 1.0), source.y.clamp(0.0, 1.0));
                            }
                            let marker = to_screen(tools::rotate_point(egui::pos2(*x, *y), meta.rotate));
                            ui.painter().circle_stroke(marker, 9.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                            ui.painter().circle_stroke(marker, 11.0, egui::Stroke::new(1.0, egui::Color32::BLACK));
                        }
                    }
                }
                None => {}
            }
        } else if let Some(error) = &self.error {
            ui.centered_and_justified(|ui| {
                ui.label(tr!("Pico squinted at this file and gave up:\n{error}", error = error))
            });
        } else if let Some(thumb) = &self.placeholder {
            // Stretched to about the size the photo will take, no bigger than
            // the photo itself.
            let size = thumb.size_vec2();
            let photo_side = self.header_size.as_ref().and_then(|(_, size)| *size).map(|(w, h)| w.max(h) as f32);
            let limit = photo_side.map_or(f32::INFINITY, |side| side / size.max_elem());
            let fit = (area.width() / size.x).min(area.height() / size.y).min(limit);
            let rect = egui::Rect::from_center_size(area.center(), size * fit);
            ui.painter().image(thumb.id(), rect, grid::full_uv(), egui::Color32::WHITE);
        } else {
            ui.centered_and_justified(|ui| ui.spinner());
        }
        self.paint_star_badge(ui, area, meta.star);
        if self.tool.is_none() && (double_clicked || background.double_clicked()) {
            return Some(ViewerAction::Close);
        }
        action
    }

    /// The star in the corner of a starred photo or video. While the star
    /// flies, the badge waits for it to land.
    fn paint_star_badge(&self, ui: &egui::Ui, area: egui::Rect, starred: bool) {
        let now = ui.input(|i| i.time);
        if starred && self.star_flight.as_ref().is_none_or(|flight| flight.landed(now)) {
            grid::paint_star(&ui.painter().with_clip_rect(area), area.left_bottom() + egui::vec2(12.0, -12.0), 28.0);
        }
    }

    /// Renders the photo without its edits in the background, once per photo
    /// that has edits, so "Original" shows it without a wait.
    fn update_original(&mut self, ctx: &egui::Context, file: &MediaFile) {
        if let Some((key, rx)) = &self.original_job
            && let Ok(image) = rx.try_recv()
        {
            let texture = ctx.load_texture("viewer original", image, egui::TextureOptions::LINEAR);
            self.original = Some((key.clone(), texture));
            self.original_job = None;
        }
        let Some((path, base, scale)) = &self.base else { return };
        if path != &file.path
            || file.meta.filters.0.is_empty()
            || self.original.is_some()
            || self.original_job.is_some()
        {
            return;
        }
        let mut meta = file.meta.clone();
        meta.filters = FilterStack::default();
        let key: RenderKey = (file.path.clone(), format!("{}|{}", meta.rotate, meta.filters));
        let (tx, rx) = channel();
        let (base, scale) = (Arc::clone(base), *scale);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let rgba = render::apply_edits_scaled((*base).clone(), &meta, scale).to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()));
            ctx.request_repaint();
        });
        self.original_job = Some((key, rx));
    }

    /// Renders the photo at full resolution in the background when the zoom
    /// magnifies the working copy, so 1:1 shows the real pixels.
    fn update_detail(&mut self, ctx: &egui::Context, file: &MediaFile, meta: &PhotoMeta) {
        if let Some((key, rx)) = &self.detail_job
            && let Ok(image) = rx.try_recv()
        {
            let texture = ctx.load_texture("viewer detail", image, egui::TextureOptions::LINEAR);
            self.detail = Some((key.clone(), texture));
            self.detail_job = None;
        }
        let Some((path, _, base_scale)) = &self.base else { return };
        let magnified = self.zoom.is_some_and(|zoom| zoom > *base_scale);
        if path != &file.path || *base_scale >= 1.0 || !magnified || self.detail_job.is_some() {
            return;
        }
        let wanted: RenderKey = (file.path.clone(), format!("{}|{}", meta.rotate, meta.filters));
        if self.detail.as_ref().is_some_and(|(key, _)| key == &wanted) {
            return;
        }
        let (tx, rx) = channel();
        let (path, kind, meta) = (file.path.clone(), file.kind, meta.clone());
        let max_side = ctx.input(|i| i.max_texture_side) as u32;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let Ok(img) = render::decode(&path, kind, &meta) else { return };
            let mut img = render::apply_edits_scaled(img, &meta, 1.0);
            if img.width().max(img.height()) > max_side {
                img = img.resize(max_side, max_side, FilterType::Triangle);
            }
            let rgba = img.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()));
            ctx.request_repaint();
        });
        self.detail_job = Some((wanted, rx));
    }

    /// Decodes the working copy when the file changes and re-renders whenever
    /// the wanted edits differ from the picture on screen. At most one render
    /// runs at a time; the newest wanted state is picked up when it finishes.
    fn update_rendering(&mut self, ctx: &egui::Context, file: &MediaFile, meta: &PhotoMeta) {
        self.poll_ahead(ctx);
        let have_base = self.base.as_ref().is_some_and(|(path, ..)| path == &file.path);
        let loading_base = self.base_loading.as_ref().is_some_and(|((path, _), _)| path == &file.path);
        if !have_base && !loading_base {
            self.keep_for_return();
            self.texture = None;
            self.error = None;
            if let Some(prepared) = self.ahead.remove(&file.path) {
                self.adopt(&file.path, prepared);
            } else if let Some(job) = self.ahead_jobs.remove(&file.path) {
                self.base_loading = Some(job);
            } else {
                self.base_loading = Some(prepare(ctx, file));
            }
        }
        if let Some((key, rx)) = &self.base_loading
            && let Ok(result) = rx.try_recv()
        {
            let prepared = into_prepared(ctx, key.clone(), result);
            let path = key.0.clone();
            self.base_loading = None;
            self.adopt(&path, prepared);
        }

        if let Some((key, rx, ..)) = &self.rendering
            && let Ok(image) = rx.try_recv()
        {
            let texture = ctx.load_texture("viewer", image, egui::TextureOptions::LINEAR);
            self.texture = Some((key.clone(), texture));
            self.rendering = None;
        }

        let Some((base_path, base, scale)) = &self.base else { return };
        if base_path != &file.path {
            return;
        }
        // Edits not saved yet (a slider moving, a tool open) are previewed on
        // the smaller copy; saved ones get the whole working copy.
        let previewing = meta.filters != file.meta.filters || meta.rotate != file.meta.rotate;
        let small = previewing && base.width().max(base.height()) > PREVIEW_SIDE;
        let mut wanted = render_key(file, meta);
        if small {
            wanted.1.push_str(PREVIEW_MARK);
        }
        if let Some((key, _, was_previewing, _)) = &self.rendering {
            // One preview after another is let finish: it is a single edit
            // (the rest is kept), and showing it keeps the picture moving
            // with the slider. Anything else out of date gives up before its
            // next edit, and the wanted picture starts now.
            if key == &wanted || (*was_previewing && previewing) {
                return;
            }
            self.rendering = None;
        }
        if self.texture.as_ref().is_some_and(|(key, _)| key == &wanted) {
            return;
        }
        if small && self.preview.as_ref().is_none_or(|(path, ..)| path != base_path) {
            let (size, per_px) = preview_size(base);
            self.preview = Some((base_path.clone(), Arc::new(OnceLock::new()), size, per_px));
        }
        let preview =
            self.preview.as_ref().filter(|_| small).map(|(_, cell, size, per_px)| (Arc::clone(cell), *size, *per_px));
        let (tx, rx) = channel();
        let (base, meta, scale, path) = (Arc::clone(base), meta.clone(), *scale, base_path.clone());
        let (kept, slot) = (Arc::clone(&self.kept), usize::from(!small));
        let ctx = ctx.clone();
        let (stop, stopped) = Stop::new();
        std::thread::spawn(move || {
            let scale = preview.as_ref().map_or(scale, |(_, _, per_px)| scale / per_px);
            let copy = || match &preview {
                Some((cell, [w, h], _)) => cell.get_or_init(|| base.resize_exact(*w, *h, FilterType::Triangle)).clone(),
                None => (*base).clone(),
            };
            let made = render_kept(copy, &meta.filters.0, scale, (&kept, slot, &path), &stopped);
            let Some(img) = made.filter(|_| !stopped.load(Ordering::Relaxed)) else { return };
            let _ = tx.send(color_image(render::turn(img, meta.rotate)));
            ctx.request_repaint();
        });
        self.rendering = Some((wanted, rx, previewing, stop));
    }

    /// The size of the picture on screen in working-copy pixels: a preview
    /// is made smaller and shown as big as the full picture.
    fn shown_size(&self, key: &RenderKey, texture: &egui::TextureHandle) -> egui::Vec2 {
        let per_px = match &self.preview {
            Some((path, _, _, per_px)) if key.1.ends_with(PREVIEW_MARK) && path == &key.0 => *per_px,
            _ => 1.0,
        };
        texture.size_vec2() * per_px
    }

    /// Puts a prepared photo on screen.
    fn adopt(&mut self, path: &std::path::Path, prepared: Result<Prepared, String>) {
        match prepared {
            Ok(Prepared { base, scale, texture, key, preview, sharp }) => {
                self.base = Some((path.to_path_buf(), base, scale));
                self.texture = Some(texture);
                if let Some((cell, size, per_px)) = preview {
                    self.preview = Some((path.to_path_buf(), cell, size, per_px));
                }
                // The sharp picture, once made, takes the preview's place.
                if let Some((rx, stop)) = sharp {
                    self.rendering = Some((key, rx, false, stop));
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// Keeps the photo being left (or still being made) among those ahead,
    /// so going back to it is instant.
    fn keep_for_return(&mut self) {
        if let Some((key, rx)) = self.base_loading.take() {
            self.ahead_jobs.insert(key.0.clone(), (key, rx));
        }
        // Its edits being rendered would land on the next photo: the sharp
        // picture of its saved edits goes along, anything else is let go.
        let rendering = self.rendering.take();
        if let (Some((path, base, scale)), Some(texture)) = (self.base.take(), self.texture.take())
            && texture.0.0 == path
        {
            let key = (path.clone(), texture.0.1.trim_end_matches(PREVIEW_MARK).to_owned());
            let preview =
                self.preview.take().filter(|(p, ..)| p == &path).map(|(_, cell, size, per_px)| (cell, size, per_px));
            let sharp =
                rendering.filter(|(k, _, previewing, _)| k == &key && !previewing).map(|(_, rx, _, stop)| (rx, stop));
            self.ahead.insert(path, Ok(Prepared { base, scale, texture, key, preview, sharp }));
        }
    }

    fn poll_ahead(&mut self, ctx: &egui::Context) {
        let done: Vec<PathBuf> = self
            .ahead_jobs
            .iter()
            .filter_map(|(path, (key, rx))| Some((path, into_prepared(ctx, key.clone(), rx.try_recv().ok()?))))
            .map(|(path, prepared)| {
                self.ahead.insert(path.clone(), prepared);
                path.clone()
            })
            .collect();
        for path in done {
            self.ahead_jobs.remove(&path);
        }
    }

    /// Prepares the photos around the one on screen, `neighbours` nearest
    /// first, once that one is ready; lets go of the rest.
    pub fn prepare_ahead(&mut self, ctx: &egui::Context, current: &MediaFile, neighbours: &[&MediaFile]) {
        self.poll_ahead(ctx);
        let wanted = |path: &PathBuf| path == &current.path || neighbours.iter().any(|f| &f.path == path);
        self.ahead_jobs.retain(|path, _| wanted(path));
        self.ahead.retain(|path, prepared| match neighbours.iter().find(|f| &f.path == path) {
            // Edited since it was prepared: made again.
            Some(file) => prepared.as_ref().map_or(true, |p| p.key == render_key(file, &file.meta)),
            None => path == &current.path,
        });
        if self.base_loading.is_some() || self.rendering.is_some() || current.kind != MediaKind::Photo {
            return;
        }
        // Two at a time, so the photo opened next is not kept waiting.
        for file in neighbours.iter().filter(|f| f.kind == MediaKind::Photo) {
            if self.ahead_jobs.len() >= 2 {
                break;
            }
            if !self.ahead.contains_key(&file.path) && !self.ahead_jobs.contains_key(&file.path) {
                self.ahead_jobs.insert(file.path.clone(), prepare(ctx, file));
            }
        }
    }

    /// The picture on screen: whether it is a preview, its texture's size
    /// and its size as shown, in working-copy pixels.
    #[cfg(test)]
    pub fn picture_sizes(&self) -> Option<(bool, egui::Vec2, egui::Vec2)> {
        let (key, texture) = self.texture.as_ref()?;
        Some((key.1.ends_with(PREVIEW_MARK), texture.size_vec2(), self.shown_size(key, texture)))
    }

    /// The starts of edit stacks kept rendered, for the smaller copy and for
    /// the working copy.
    #[cfg(test)]
    pub fn kept(&self) -> [Vec<String>; 2] {
        self.kept.lock().unwrap().clone().map(|kept| {
            kept.into_iter().map(|(path, filters, _)| format!("{}|{}", path.display(), FilterStack(filters))).collect()
        })
    }

    /// The edits of the picture on screen and of the one being rendered.
    #[cfg(test)]
    pub fn picture_edits(&self) -> [Option<String>; 2] {
        [self.texture.as_ref().map(|(key, _)| key.1.clone()), self.rendering.as_ref().map(|(key, ..)| key.1.clone())]
    }

    /// Photos prepared ahead, ready to show.
    #[cfg(test)]
    pub fn prepared_ahead(&self) -> usize {
        self.ahead.values().filter(|p| p.is_ok()).count()
    }

    /// True once the picture for this file and its saved edits is on screen.
    #[cfg(test)]
    pub fn is_loaded(&self, file: &MediaFile) -> bool {
        let wanted = (file.path.clone(), format!("{}|{}", file.meta.rotate, file.meta.filters));
        self.error.is_some()
            || (self.rendering.is_none() && self.texture.as_ref().is_some_and(|(key, _)| key == &wanted))
    }

    #[cfg(test)]
    pub fn tool_state(&self) -> String {
        match &self.tool {
            Some((_, Tool::Crop(crop))) => format!("crop {:?}", crop.rect),
            Some((_, Tool::Straighten { amount })) => format!("straighten {amount}"),
            Some((_, Tool::Effect { spec, params })) => format!("effect {} {params:?}", spec.id),
            None => "none".into(),
        }
    }

    #[cfg(test)]
    pub fn player_state(&self) -> (bool, std::time::Duration, bool) {
        self.player.state()
    }

    /// Whether the unedited photo is ready and being shown.
    #[cfg(test)]
    pub fn showing_original(&self) -> (bool, bool) {
        (self.original.is_some(), self.show_original)
    }

    /// Zoom in percent (`None` while fitting) and whether the
    /// full-resolution picture is on screen.
    #[cfg(test)]
    pub fn zoom_state(&self) -> (Option<f32>, egui::Vec2, bool) {
        let detail = self.zoom.is_some()
            && self.detail.is_some()
            && self.detail.as_ref().map(|(k, _)| k) == self.texture.as_ref().map(|(k, _)| k);
        (self.zoom.map(|z| (z * 100.0).round()), self.pan, detail)
    }

    /// The zoom mode and the user's own zoom, as kept in the settings:
    /// `fit`, `1:1` or `custom`, and a scale (1.0 is 100%).
    pub fn zoom_setting(&self) -> (&'static str, Option<f32>) {
        let mode = match self.zoom_mode {
            ZoomMode::Fit => "fit",
            ZoomMode::Actual => "1:1",
            ZoomMode::Custom => "custom",
        };
        (mode, self.custom_zoom)
    }

    /// Restores what [`Self::zoom_setting`] gave.
    pub fn set_zoom_setting(&mut self, mode: &str, zoom: Option<f32>) {
        self.custom_zoom = zoom.filter(|z| z.is_finite()).map(|z| z.clamp(MIN_ZOOM, MAX_ZOOM));
        self.zoom_mode = match mode {
            "1:1" => ZoomMode::Actual,
            "custom" if self.custom_zoom.is_some() => ZoomMode::Custom,
            _ => ZoomMode::Fit,
        };
    }

    /// The crop aspect to save: its name and whether it is turned to portrait.
    pub fn crop_setting(&self) -> (&'static str, bool) {
        self.crop_shape.setting()
    }

    pub fn set_crop_setting(&mut self, name: &str, portrait: bool) {
        self.crop_shape = tools::CropShape::from_setting(name, portrait);
    }

    pub fn is_full_screen(&self) -> bool {
        self.full_screen
    }

    pub fn set_full_screen(&mut self, on: bool) {
        self.full_screen = on;
    }

    #[cfg(test)]
    pub fn open_edit_panel(&mut self) {
        self.show_edit_panel = true;
    }
}

/// What a texture of `file` with edits `meta` shows.
/// Ends the render key of a picture made from the smaller preview copy.
const PREVIEW_MARK: &str = "|preview";

fn render_key(file: &MediaFile, meta: &PhotoMeta) -> RenderKey {
    (file.path.clone(), format!("{}|{}", meta.rotate, meta.filters))
}

/// Decodes the working copy of `file` and renders its saved edits, in the
/// background.
fn prepare(ctx: &egui::Context, file: &MediaFile) -> (RenderKey, Receiver<PreparedResult>) {
    let (tx, rx) = channel();
    let (path, kind, meta) = (file.path.clone(), file.kind, file.meta.clone());
    // Some GPUs take no textures over 2048 pixels.
    let working_side = WORKING_SIDE.min(ctx.input(|i| i.max_texture_side) as u32);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let img = match render::decode(&path, kind, &meta) {
            Ok(img) => img,
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                ctx.request_repaint();
                return;
            }
        };
        let side = img.width().max(img.height());
        let (img, scale) = if side > working_side {
            (img.resize(working_side, working_side, FilterType::Triangle), working_side as f32 / side as f32)
        } else {
            (img, 1.0)
        };
        let image = color_image(render::apply_edits_scaled(img.clone(), &meta, scale));
        let _ = tx.send(Ok(Made { base: Arc::new(img), scale, image }));
        ctx.request_repaint();
    });
    (render_key(file, &file.meta), rx)
}

fn into_prepared(ctx: &egui::Context, key: RenderKey, result: PreparedResult) -> Result<Prepared, String> {
    let Made { base, scale, image } = result?;
    let texture = ctx.load_texture("viewer", image, egui::TextureOptions::LINEAR);
    Ok(Prepared { base, scale, texture: (key.clone(), texture), key, preview: None, sharp: None })
}

/// The size of the smaller copy of `base`, and how many of `base`'s pixels
/// one of its pixels stands for.
fn preview_size(base: &DynamicImage) -> ([u32; 2], f32) {
    let fit = PREVIEW_SIDE as f32 / base.width().max(base.height()) as f32;
    let size = [base.width(), base.height()].map(|side| ((side as f32 * fit).round() as u32).max(1));
    (size, base.width() as f32 / size[0] as f32)
}

fn color_image(img: DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}

/// Applies `filters` to the copy `copy` makes, starting from the longest
/// start of them already kept for this photo and copy (`slot`), and keeps
/// what it makes: the stack without its last edit (the one usually tried
/// out next) and the whole stack. None when `stop` was set.
fn render_kept(
    copy: impl FnOnce() -> DynamicImage,
    filters: &[Filter],
    scale: f32,
    (kept, slot, path): (&KeptRenders, usize, &std::path::Path),
    stop: &AtomicBool,
) -> Option<DynamicImage> {
    let stopped = || stop.load(Ordering::Relaxed);
    let start = kept.lock().unwrap_or_else(|e| e.into_inner())[slot]
        .iter()
        .filter(|(p, done, _)| p == path && filters.starts_with(done))
        .max_by_key(|(_, done, _)| done.len())
        .map(|(_, done, img)| (done.len(), Arc::clone(img)));
    let (mut done, mut img) = match start {
        Some((done, img)) => (done, (*img).clone()),
        None => (0, copy()),
    };
    let keep = |filters: &[Filter], img: &DynamicImage| {
        let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
        let list = &mut kept[slot];
        list.retain(|(p, f, _)| !(p == path && f == filters));
        if list.len() >= KEPT {
            list.remove(0);
        }
        list.push((path.to_path_buf(), filters.to_vec(), Arc::new(img.clone())));
    };
    let last = filters.len().saturating_sub(1);
    if done < last {
        img = render::apply_filters_until(img, &filters[done..last], scale, stopped)?;
        keep(&filters[..last], &img);
        done = last;
    }
    if done < filters.len() {
        img = render::apply_filters_until(img, &filters[done..], scale, stopped)?;
        keep(filters, &img);
    }
    Some(img)
}

fn from_player(action: PlayerAction) -> ViewerAction {
    match action {
        PlayerAction::SavedFrame(path) => ViewerAction::FileAdded(path),
        PlayerAction::Error(message) => ViewerAction::Status(message),
        PlayerAction::Close => ViewerAction::Close,
    }
}
