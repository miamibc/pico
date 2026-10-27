//! The collage workspace: settings on the left, live preview in the middle,
//! the pictures ("clips") along the bottom.

use crate::grid;
use eframe::egui;
use pico_core::collage::{self, Background, Border, CollageOptions, Item, Layout};
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// Photos are prepared at this size: plenty for the saved page.
const ITEM_SIDE: u32 = 1600;
const PREVIEW_WIDTH: u32 = 1200;
const SAVE_WIDTH: u32 = 3000;

const ASPECTS: [(&str, f32); 5] =
    [("3 : 2", 1.5), ("4 : 3", 4.0 / 3.0), ("16 : 9", 16.0 / 9.0), ("1 : 1", 1.0), ("A4", 1.414)];

pub enum CollageAction {
    Close,
    /// The collage was written; `wallpaper` asks to set it as the desktop background.
    Saved {
        path: PathBuf,
        wallpaper: bool,
    },
    Status(String),
}

pub struct CollageView {
    files: Vec<MediaFile>,
    /// Prepared photos, in the files' order.
    items: Option<Arc<Vec<Item>>>,
    loading: Option<Receiver<Vec<Item>>>,
    /// Display order of the photos (indices into `items`); "Shuffle" permutes it.
    order: Vec<usize>,
    options: CollageOptions,
    aspect: usize,
    portrait: bool,
    background_photo: bool,
    background_color: [u8; 3],
    /// The highlighted clip in the tray, as an index into `order`.
    picked: Option<usize>,
    preview: Option<(String, egui::TextureHandle)>,
    rendering: Option<(String, Receiver<egui::ColorImage>)>,
    saving: Option<Receiver<Result<(PathBuf, bool), String>>>,
}

impl CollageView {
    pub fn new(ctx: &egui::Context, files: Vec<MediaFile>, title: String) -> Self {
        let files: Vec<MediaFile> = files.into_iter().filter(|f| f.kind != MediaKind::Video).collect();
        let (tx, rx) = channel();
        let to_load = files.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let items = to_load
                .iter()
                .filter_map(|file| {
                    let image = pico_core::export::render_full(file).ok()?.thumbnail(ITEM_SIDE, ITEM_SIDE);
                    Some(Item { image, name: file.name(), caption: file.meta.caption.clone() })
                })
                .collect();
            let _ = tx.send(items);
            ctx.request_repaint();
        });
        let order = (0..files.len()).collect();
        Self {
            files,
            items: None,
            loading: Some(rx),
            order,
            options: CollageOptions { title, ..Default::default() },
            aspect: 0,
            portrait: false,
            background_photo: false,
            background_color: [255, 255, 255],
            picked: None,
            preview: None,
            rendering: None,
            saving: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<CollageAction> {
        self.poll(ui.ctx());
        let mut action = None;
        egui::Panel::left("collage_settings").resizable(false).default_size(250.0).show(ui, |ui| {
            action = self.settings(ui);
        });
        egui::Panel::bottom("collage_tray").show(ui, |ui| self.tray(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let area = ui.max_rect();
            ui.painter().rect_filled(area, 0.0, egui::Color32::from_gray(30));
            match &self.preview {
                Some((_, texture)) => {
                    let size = texture.size_vec2();
                    let scale = ((area.width() - 40.0) / size.x).min((area.height() - 40.0) / size.y);
                    let rect = egui::Rect::from_center_size(area.center(), size * scale);
                    ui.painter().image(texture.id(), rect, grid::full_uv(), egui::Color32::WHITE);
                }
                None => {
                    ui.centered_and_justified(|ui| ui.spinner());
                }
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) && !ui.ctx().egui_wants_keyboard_input() {
            action = Some(CollageAction::Close);
        }
        self.update_preview(ui.ctx());
        action
    }

    fn settings(&mut self, ui: &mut egui::Ui) -> Option<CollageAction> {
        let mut action = None;
        ui.heading("Picture Collage");
        egui::ComboBox::from_label("Layout").selected_text(layout_name(self.options.layout)).show_ui(ui, |ui| {
            for (layout, name) in Layout::ALL {
                ui.selectable_value(&mut self.options.layout, layout, name);
            }
        });
        ui.label(egui::RichText::new(layout_hint(self.options.layout)).weak());
        ui.separator();
        ui.label("Page Format");
        ui.horizontal_wrapped(|ui| {
            for (i, (name, _)) in ASPECTS.iter().enumerate() {
                ui.selectable_value(&mut self.aspect, i, *name);
            }
        });
        ui.checkbox(&mut self.portrait, "Portrait");
        let ratio = ASPECTS[self.aspect].1;
        self.options.aspect = if self.portrait { 1.0 / ratio } else { ratio };
        ui.separator();
        ui.label("Background");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.background_photo, false, "Solid color");
            ui.color_edit_button_srgb(&mut self.background_color);
        });
        let can_use_photo = self.picked.is_some() || matches!(self.options.background, Background::Photo(_));
        if ui.add_enabled(can_use_photo, egui::RadioButton::new(self.background_photo, "Picked picture")).clicked() {
            self.background_photo = true;
        }
        let [r, g, b] = self.background_color;
        self.options.background = match (self.background_photo, self.picked) {
            (true, Some(p)) => Background::Photo(self.order[p]),
            (true, None) => self.options.background,
            (false, _) => Background::Color((r as u32) << 16 | (g as u32) << 8 | b as u32),
        };
        ui.separator();
        ui.label("Picture Borders");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.options.border, Border::None, "None");
            ui.radio_value(&mut self.options.border, Border::White, "White");
            ui.radio_value(&mut self.options.border, Border::InstantCamera, "Instant");
        });
        ui.checkbox(&mut self.options.shadows, "Draw Shadows");
        ui.checkbox(&mut self.options.captions, "Show Captions").on_hover_text("On the Instant camera border");
        let mut spacing = self.options.spacing * 1000.0;
        ui.add(egui::Slider::new(&mut spacing, 0.0..=50.0).text("Grid Spacing"));
        self.options.spacing = spacing / 1000.0;
        if self.options.layout == Layout::ContactSheet {
            ui.label("Title");
            ui.text_edit_singleline(&mut self.options.title);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Shuffle Pictures").on_hover_text("Randomize the order of the pictures").clicked() {
                self.order = collage::shuffled(&self.order, self.options.seed.wrapping_add(self.order.len() as u64));
                self.options.seed = self.options.seed.wrapping_add(1);
            }
        });
        ui.separator();
        let ready = self.items.is_some() && self.saving.is_none() && !self.order.is_empty();
        if ui.add_enabled(ready, egui::Button::new("Create Collage")).on_hover_text("Save as a JPG").clicked() {
            self.save(ui.ctx(), false);
        }
        if ui
            .add_enabled(ready, egui::Button::new("Desktop Background"))
            .on_hover_text("Save, then use as wallpaper")
            .clicked()
        {
            self.save(ui.ctx(), true);
        }
        if self.saving.is_some() {
            crate::app::progress_window(ui.ctx(), "Saving the Collage", "Gluing pictures…", None);
        }
        if ui.button("Close").on_hover_text("Esc").clicked() {
            action = Some(CollageAction::Close);
        }
        if let Some(rx) = &self.saving
            && let Ok(result) = rx.try_recv()
        {
            self.saving = None;
            action = Some(match result {
                Ok((path, wallpaper)) => CollageAction::Saved { path, wallpaper },
                Err(e) => CollageAction::Status(format!("Could not save the collage: {e}")),
            });
        }
        action
    }

    fn tray(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(format!("Clips ({})", self.order.len()));
            let picked = self.picked;
            if ui.add_enabled(picked.is_some(), egui::Button::new("Remove")).clicked()
                && let Some(p) = picked
            {
                self.order.remove(p);
                self.picked = None;
            }
            if ui.add_enabled(picked.is_some(), egui::Button::new("Set as Background")).clicked()
                && let Some(p) = picked
            {
                self.background_photo = true;
                self.options.background = Background::Photo(self.order[p]);
            }
            if ui.add_enabled(picked.is_some(), egui::Button::new("Set as Frame Center")).clicked()
                && let Some(p) = picked
            {
                self.options.layout = Layout::FrameMosaic;
                self.options.center = p;
            }
        });
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (p, &i) in self.order.iter().enumerate() {
                    let name = self.files[i].name();
                    if ui.selectable_label(self.picked == Some(p), &name).clicked() {
                        self.picked = if self.picked == Some(p) { None } else { Some(p) };
                    }
                }
            });
        });
    }

    fn poll(&mut self, _ctx: &egui::Context) {
        if let Some(rx) = &self.loading
            && let Ok(items) = rx.try_recv()
        {
            // Some files may have failed to load; keep files and items aligned.
            if items.len() != self.files.len() {
                let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
                self.files.retain(|f| names.contains(&f.name()));
                self.order = (0..items.len()).collect();
            }
            self.items = Some(Arc::new(items));
            self.loading = None;
        }
    }

    fn effective_options(&self, width: u32) -> CollageOptions {
        let mut options = self.options.clone();
        options.width = width;
        // Indices in the options refer to the shown order.
        if let Background::Photo(item) = options.background {
            options.background =
                self.order.iter().position(|&i| i == item).map_or(Background::Color(0xffffff), Background::Photo);
        }
        options.center = options.center.min(self.order.len().saturating_sub(1));
        options
    }

    fn update_preview(&mut self, ctx: &egui::Context) {
        if let Some((key, rx)) = &self.rendering
            && let Ok(image) = rx.try_recv()
        {
            let texture = ctx.load_texture("collage", image, egui::TextureOptions::LINEAR);
            self.preview = Some((key.clone(), texture));
            self.rendering = None;
        }
        let Some(items) = &self.items else { return };
        if self.rendering.is_some() || self.order.is_empty() {
            return;
        }
        let options = self.effective_options(PREVIEW_WIDTH);
        let key = format!("{options:?}{:?}", self.order);
        if self.preview.as_ref().is_some_and(|(k, _)| k == &key) {
            return;
        }
        let (tx, rx) = channel();
        let (items, order, ctx) = (Arc::clone(items), self.order.clone(), ctx.clone());
        std::thread::spawn(move || {
            let refs: Vec<&Item> = order.iter().map(|&i| &items[i]).collect();
            let page = collage::render(&refs, &options);
            let size = [page.width() as usize, page.height() as usize];
            let _ = tx.send(egui::ColorImage::from_rgba_unmultiplied(size, page.as_raw()));
            ctx.request_repaint();
        });
        self.rendering = Some((key, rx));
    }

    /// Writes the collage as `Collage N.jpg` in the first photo's folder.
    fn save(&mut self, ctx: &egui::Context, wallpaper: bool) {
        let Some(items) = &self.items else { return };
        let Some(dir) = self.order.first().and_then(|&i| self.files[i].path.parent()).map(|p| p.to_path_buf()) else {
            return;
        };
        let target = (1..).map(|n| dir.join(format!("Collage {n}.jpg"))).find(|p| !p.exists()).expect("a free name");
        let options = self.effective_options(SAVE_WIDTH);
        let (tx, rx) = channel();
        let (items, order, ctx) = (Arc::clone(items), self.order.clone(), ctx.clone());
        std::thread::spawn(move || {
            let refs: Vec<&Item> = order.iter().map(|&i| &items[i]).collect();
            let page = collage::render(&refs, &options);
            let rgb = pico_core::image::DynamicImage::ImageRgba8(page).to_rgb8();
            let result = rgb.save(&target).map(|()| (target, wallpaper)).map_err(|e| e.to_string());
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.saving = Some(rx);
    }

    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.items.is_some() && self.rendering.is_none() && self.preview.is_some()
    }

    #[cfg(test)]
    pub fn set_layout(&mut self, layout: Layout) {
        self.options.layout = layout;
    }
}

fn layout_name(layout: Layout) -> &'static str {
    Layout::ALL.iter().find(|(l, _)| *l == layout).map_or("", |(_, n)| n)
}

fn layout_hint(layout: Layout) -> &'static str {
    match layout {
        Layout::Mosaic => "Pictures fitted edge to edge",
        Layout::FrameMosaic => "A mosaic around one big picture",
        Layout::Grid => "Neat rows and columns",
        Layout::ContactSheet => "Thumbnails with names and a header",
        Layout::MultipleExposure => "Pictures laid over one another",
    }
}
