//! Full-size view of one photo with the edit panel.
//!
//! The photo is decoded once into a working copy; edits are then rendered
//! from that copy in the background, so slider changes preview quickly.

use crate::grid;
use crate::player::{Player, PlayerAction};
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
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// Longest side of the working copy; larger photos are downscaled for display.
const WORKING_SIDE: u32 = 2560;

pub enum ViewerAction {
    Previous,
    Next,
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

/// A decoded working copy and its scale, or why decoding failed.
type BaseResult = Result<(DynamicImage, f32), String>;

/// What a texture shows: the file plus the edits it was rendered with.
type RenderKey = (PathBuf, String);

#[derive(Default)]
pub struct Viewer {
    /// Working copy and its size relative to the original photo.
    base: Option<(PathBuf, Arc<DynamicImage>, f32)>,
    base_loading: Option<(PathBuf, Receiver<BaseResult>)>,
    texture: Option<(RenderKey, egui::TextureHandle)>,
    rendering: Option<(RenderKey, Receiver<egui::ColorImage>)>,
    error: Option<String>,
    caption: String,
    caption_for: Option<PathBuf>,
    /// Uncommitted edits while a slider is being dragged.
    draft: Option<(PathBuf, FilterStack)>,
    show_edit_panel: bool,
    /// The open Crop or Straighten tool and the file it belongs to.
    tool: Option<(PathBuf, Tool)>,
    player: Player,
}

impl Viewer {
    pub fn show(&mut self, ui: &mut egui::Ui, file: &MediaFile, position: (usize, usize)) -> Option<ViewerAction> {
        if self.draft.as_ref().is_some_and(|(path, _)| path != &file.path) {
            self.draft = None;
        }
        if self.tool.as_ref().is_some_and(|(path, _)| path != &file.path) {
            self.tool = None;
        }
        if self.caption_for.as_ref() != Some(&file.path) {
            self.caption = file.meta.caption.clone().unwrap_or_default();
            self.caption_for = Some(file.path.clone());
        }
        let mut action = None;

        egui::Panel::bottom("viewer_tools").show(ui, |ui| {
            if let Some(a) = self.toolbar(ui, file, position) {
                action = Some(a);
            }
        });
        if file.kind == MediaKind::Video {
            // Videos get the player instead of the photo editor.
            let player_action = self.player.show(ui, file);
            return action
                .or(player_action.map(|a| match a {
                    PlayerAction::SetFilters(stack) => ViewerAction::SetFilters(stack),
                    PlayerAction::SavedFrame(path) => ViewerAction::FileAdded(path),
                    PlayerAction::Error(message) => ViewerAction::Status(message),
                }))
                .or_else(|| self.navigation_keys(ui));
        }
        if self.show_edit_panel || self.tool.is_some() {
            egui::Panel::left("edit_panel").resizable(false).default_size(240.0).show(ui, |ui| {
                if let Some(a) = self.edit_panel(ui, file) {
                    action = Some(a);
                }
            });
        }

        let meta = self.effective_meta(file);
        self.update_rendering(ui.ctx(), file, &meta);
        let double_clicked = egui::CentralPanel::no_frame().show(ui, |ui| self.picture(ui, &meta)).inner;
        if action.is_none() && double_clicked {
            return Some(ViewerAction::Close);
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
            action = self.navigation_keys(ui);
        }
        action
    }

    /// Keys shared by photos and videos, ignored while a text field has focus.
    fn navigation_keys(&self, ui: &egui::Ui) -> Option<ViewerAction> {
        if ui.ctx().egui_wants_keyboard_input() {
            return None;
        }
        ui.input(|i| {
            if i.key_pressed(egui::Key::Escape) {
                Some(ViewerAction::Close)
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
        ui.horizontal(|ui| {
            if ui.button("⬅ Back to Library").on_hover_text("Esc").clicked() {
                action = Some(ViewerAction::Close);
            }
            if file.kind != MediaKind::Video {
                ui.toggle_value(&mut self.show_edit_panel, "Edit").on_hover_text("Show the edit panel");
            }
            if ui.button("▶ Slideshow").on_hover_text("Ctrl+4").clicked() {
                action = Some(ViewerAction::Slideshow);
            }
            ui.separator();
            if ui.button("◀").on_hover_text("Previous (Left arrow)").clicked() {
                action = Some(ViewerAction::Previous);
            }
            ui.label(format!("{} / {}", position.0 + 1, position.1));
            if ui.button("▶").on_hover_text("Next (Right arrow)").clicked() {
                action = Some(ViewerAction::Next);
            }
            ui.separator();
            let star = if file.meta.star { "★ Starred" } else { "☆ Star" };
            if ui.button(star).on_hover_text("S").clicked() {
                action = Some(ViewerAction::ToggleStar);
            }
            if ui.button("⟲").on_hover_text("Rotate counterclockwise (Ctrl+Shift+R)").clicked() {
                action = Some(ViewerAction::Rotate(-1));
            }
            if ui.button("⟳").on_hover_text("Rotate clockwise (Ctrl+R)").clicked() {
                action = Some(ViewerAction::Rotate(1));
            }
            ui.separator();
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.caption)
                    .hint_text("Add a caption. Future you will thank you.")
                    .desired_width(f32::INFINITY),
            );
            let committed = edit.lost_focus() || (edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            let caption = Some(self.caption.trim().to_owned()).filter(|c| !c.is_empty());
            if committed && caption != file.meta.caption {
                action = Some(ViewerAction::SetCaption(caption));
            }
        });
        action
    }

    /// Ends the open tool, turning an applied tool into a new edit on the stack.
    fn finish_tool(&mut self, action: ToolAction, file: &MediaFile) -> Option<ViewerAction> {
        self.tool = None;
        match action {
            ToolAction::Apply(filter) => {
                let mut stack = file.meta.filters.clone();
                stack.0.push(filter);
                Some(ViewerAction::SetFilters(stack))
            }
            ToolAction::Cancel => None,
        }
    }

    fn edit_panel(&mut self, ui: &mut egui::Ui, file: &MediaFile) -> Option<ViewerAction> {
        let image_px = self.texture.as_ref().map_or(egui::vec2(1.0, 1.0), |(_, t)| t.size_vec2());
        let tool_action = match &mut self.tool {
            Some((_, Tool::Crop(crop))) => Some(crop.controls(ui, image_px, file.meta.rotate)),
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

        ui.heading("Basic Fixes");
        ui.horizontal_wrapped(|ui| {
            if ui.button("Crop").clicked() {
                self.draft = None;
                self.tool = Some((file.path.clone(), Tool::Crop(CropTool::default())));
            }
            if ui.button("Straighten").clicked() {
                self.draft = None;
                self.tool = Some((file.path.clone(), Tool::Straighten { amount: 0.0 }));
            }
            for (label, id) in
                [("I'm Feeling Lucky", "enhance"), ("Auto Contrast", "autolight"), ("Auto Color", "autocolor")]
            {
                if ui.button(label).clicked() {
                    stack.0.push(Filter::effect(id, []));
                    commit = true;
                }
            }
        });

        ui.separator();
        ui.heading("Tuning");
        // The sliders edit the newest Tuning edit while it is on top of the
        // stack; after any other edit they start a new one, as Picasa does.
        let on_top = stack.0.last().is_some_and(|f| f.id == "finetune2");
        let current = stack.0.last().filter(|_| on_top);
        let mut values = [0usize, 1, 2, 4].map(|i| current.and_then(|f| f.float(i)).unwrap_or(0.0));
        let neutral = current.and_then(|f| f.color(3)).unwrap_or(0);
        let mut changed = false;
        for (value, (label, range)) in values.iter_mut().zip([
            ("Fill Light", 0.0..=1.0),
            ("Highlights", 0.0..=0.48),
            ("Shadows", 0.0..=0.48),
            ("Color Temperature", -1.0..=1.0),
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
        ui.heading("Effects");
        let mut open_effect = None;
        ui.horizontal_wrapped(|ui| {
            for spec in effects::SPECS.iter().filter(|s| s.group == Group::Picasa) {
                if ui.button(spec.label).clicked() {
                    open_effect = Some(spec);
                }
            }
        });
        egui::CollapsingHeader::new("Creative Kit").default_open(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for spec in effects::SPECS.iter().filter(|s| s.group == Group::Creative && s.id != "ReanimatedEyeColor")
                {
                    if ui.button(spec.label).clicked() {
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
            0 => "No edits yet. The photo is perfect, apparently.".to_owned(),
            1 => "1 edit".to_owned(),
            n => format!("{n} edits"),
        });
        if hidden > 0 {
            ui.label(egui::RichText::new(format!("{hidden} not shown yet (kept in the file)")).weak())
                .on_hover_text("Pico keeps edits it cannot draw yet, so Picasa still sees them.");
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(count > 0, egui::Button::new("Undo")).on_hover_text("Remove the last edit").clicked() {
                stack.0.pop();
                commit = true;
            }
            if ui.add_enabled(count > 0, egui::Button::new("Undo All Edits")).clicked() {
                stack.0.clear();
                commit = true;
            }
        });

        if commit {
            self.draft = None;
            return (stack != committed).then_some(ViewerAction::SetFilters(stack));
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

    /// Draws the photo; returns whether it was double-clicked outside a tool,
    /// which takes the user back to the library like in Picasa.
    fn picture(&mut self, ui: &mut egui::Ui, meta: &PhotoMeta) -> bool {
        let area = ui.max_rect();
        let background = ui.interact(area, ui.scope_id().with("backdrop"), egui::Sense::click());
        let mut double_clicked = false;
        ui.painter().rect_filled(area, 0.0, egui::Color32::from_gray(16));
        if let Some((_, texture)) = &self.texture {
            let size = texture.size_vec2();
            // Small photos are not blown up, except while cropping, where room to aim helps.
            let max_scale = if matches!(self.tool, Some((_, Tool::Crop(_)))) { f32::INFINITY } else { 1.0 };
            let scale = (area.width() / size.x).min(area.height() / size.y).min(max_scale);
            let rect = egui::Rect::from_center_size(area.center(), size * scale);
            ui.painter().image(texture.id(), rect, grid::full_uv(), egui::Color32::WHITE);
            let response = ui.interact(rect, ui.scope_id().with("photo"), egui::Sense::click_and_drag());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Photo"));
            double_clicked = response.double_clicked();
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
            ui.centered_and_justified(|ui| ui.label(format!("Pico squinted at this file and gave up:\n{error}")));
        } else {
            ui.centered_and_justified(|ui| ui.spinner());
        }
        if meta.star {
            grid::paint_star(ui.painter(), area.left_bottom() + egui::vec2(12.0, -12.0), 28.0);
        }
        self.tool.is_none() && (double_clicked || background.double_clicked())
    }

    /// Decodes the working copy when the file changes and re-renders whenever
    /// the wanted edits differ from the picture on screen. At most one render
    /// runs at a time; the newest wanted state is picked up when it finishes.
    fn update_rendering(&mut self, ctx: &egui::Context, file: &MediaFile, meta: &PhotoMeta) {
        let have_base = self.base.as_ref().is_some_and(|(path, ..)| path == &file.path);
        let loading_base = self.base_loading.as_ref().is_some_and(|(path, _)| path == &file.path);
        if !have_base && !loading_base {
            self.texture = None;
            self.error = None;
            let (tx, rx) = channel();
            let (path, kind, meta) = (file.path.clone(), file.kind, file.meta.clone());
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = render::decode(&path, kind, &meta)
                    .map(|img| {
                        let side = img.width().max(img.height());
                        if side > WORKING_SIDE {
                            (
                                img.resize(WORKING_SIDE, WORKING_SIDE, FilterType::Triangle),
                                WORKING_SIDE as f32 / side as f32,
                            )
                        } else {
                            (img, 1.0)
                        }
                    })
                    .map_err(|e| e.to_string());
                let _ = tx.send(result);
                ctx.request_repaint();
            });
            self.base_loading = Some((file.path.clone(), rx));
        }
        if let Some((path, rx)) = &self.base_loading
            && let Ok(result) = rx.try_recv()
        {
            match result {
                Ok((img, scale)) => self.base = Some((path.clone(), Arc::new(img), scale)),
                Err(e) => self.error = Some(e),
            }
            self.base_loading = None;
        }

        if let Some((key, rx)) = &self.rendering
            && let Ok(image) = rx.try_recv()
        {
            let texture = ctx.load_texture("viewer", image, egui::TextureOptions::LINEAR);
            self.texture = Some((key.clone(), texture));
            self.rendering = None;
        }

        let Some((base_path, base, scale)) = &self.base else { return };
        if base_path != &file.path || self.rendering.is_some() {
            return;
        }
        let wanted: RenderKey = (file.path.clone(), format!("{}|{}", meta.rotate, meta.filters));
        if self.texture.as_ref().is_some_and(|(key, _)| key == &wanted) {
            return;
        }
        let (tx, rx) = channel();
        let (base, meta, scale) = (Arc::clone(base), meta.clone(), *scale);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let rgba = render::apply_edits_scaled((*base).clone(), &meta, scale).to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()));
            ctx.request_repaint();
        });
        self.rendering = Some((wanted, rx));
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

    #[cfg(test)]
    pub fn open_edit_panel(&mut self) {
        self.show_edit_panel = true;
    }
}
