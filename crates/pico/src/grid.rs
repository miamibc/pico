//! The thumbnail grid of the current listing (a folder, search results, …).

use crate::thumbs::{Thumb, ThumbCache};
use eframe::egui;
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const MIN_THUMB: f32 = 64.0;
pub const MAX_THUMB: f32 = 256.0;

/// A file in the library: (folder index, file index).
pub type FileRef = (usize, usize);

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptionMode {
    #[default]
    None,
    Filename,
    Caption,
    Tags,
}

impl CaptionMode {
    /// The name in the settings file.
    pub fn key(self) -> &'static str {
        match self {
            CaptionMode::None => "none",
            CaptionMode::Filename => "filename",
            CaptionMode::Caption => "caption",
            CaptionMode::Tags => "tags",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "none" => CaptionMode::None,
            "filename" => CaptionMode::Filename,
            "caption" => CaptionMode::Caption,
            "tags" => CaptionMode::Tags,
            _ => return None,
        })
    }

    pub const ALL: [(CaptionMode, &'static str); 4] = [
        (CaptionMode::None, tr_mark!("None")),
        (CaptionMode::Filename, tr_mark!("Filename")),
        (CaptionMode::Caption, tr_mark!("Caption")),
        (CaptionMode::Tags, tr_mark!("Tags")),
    ];

    fn text(self, file: &MediaFile) -> Option<String> {
        match self {
            CaptionMode::None => None,
            CaptionMode::Filename => Some(file.name()),
            CaptionMode::Caption => Some(file.meta.caption.clone().unwrap_or_default()),
            CaptionMode::Tags => Some(file.meta.keywords.join(", ")),
        }
    }
}

pub enum GridAction {
    Click {
        at: FileRef,
        toggle: bool,
        range: bool,
    },
    Open(FileRef),
    /// A selection rectangle is being dragged over these photos; `done` once
    /// the button is let go. With `toggle` (Ctrl) it adds to the selection.
    Band {
        hits: Vec<FileRef>,
        toggle: bool,
        done: bool,
    },
    /// Photos and missing photos' tiles dragged within section `section` were
    /// let go before its tile `to` (or after the last); only with
    /// `GridView::reorder`.
    Reorder {
        section: usize,
        photos: Vec<FileRef>,
        missing: Vec<usize>,
        to: usize,
    },
}

/// Photos being dragged out of the grid, to a folder, an album or the tray.
#[derive(Clone)]
pub struct PhotoDrag(pub Vec<FileRef>, pub Vec<usize>);

/// A run of photos in the grid, under an optional header (a folder, album,
/// month or place in the ribbon of them all).
pub struct GridSection<'a> {
    pub header: Option<SectionHeader>,
    pub items: Vec<(FileRef, &'a MediaFile)>,
    /// Album photos that are not in the library, shown as black tiles among
    /// the photos; by place.
    pub missing: Vec<MissingPhoto>,
}

impl GridSection<'_> {
    /// Tiles in all: photos and missing ones.
    fn len(&self) -> usize {
        self.items.len() + self.missing.len()
    }

    /// The photo (its index in `items`) or missing photo at tile `slot`.
    fn slot(&self, slot: usize) -> Result<usize, &MissingPhoto> {
        match self.missing.iter().find(|m| m.at == slot) {
            Some(missing) => Err(missing),
            None => Ok(slot - self.missing.iter().filter(|m| m.at < slot).count()),
        }
    }

    /// The tile of the photo at `item` in `items`.
    fn slot_of(&self, item: usize) -> usize {
        let mut slot = item;
        for missing in &self.missing {
            if missing.at <= slot {
                slot += 1;
            }
        }
        slot
    }
}

/// An album photo Pico cannot show.
pub struct MissingPhoto {
    /// Its tile among the section's tiles, in order.
    pub at: usize,
    pub name: String,
    /// Its folder, when the photo is there but the folder is not in Pico;
    /// `None` when it was moved or deleted.
    pub folder: Option<PathBuf>,
}

pub struct SectionHeader {
    pub title: String,
    /// Count and dates, after the title.
    pub detail: String,
    /// The folder description, on a line of its own.
    pub note: Option<String>,
}

pub struct GridView<'a> {
    pub sections: &'a [GridSection<'a>],
    pub thumb_size: f32,
    pub selection: &'a BTreeSet<FileRef>,
    pub current: Option<FileRef>,
    pub captions: CaptionMode,
    /// Scroll so this section's header is at the top.
    pub scroll_to_section: Option<usize>,
    /// Scroll just enough to show this photo.
    pub reveal: Option<FileRef>,
    /// The section the photo to reveal is looked for first; a photo can be
    /// in several (albums).
    pub home: Option<usize>,
    /// Photos can be dragged into a new order (an album's Custom Order).
    pub reorder: bool,
    /// The wheel scrolls with weight (`Kinetic`); else as egui scrolls.
    pub kinetic: bool,
}

pub struct GridOutput {
    pub action: Option<GridAction>,
    /// The section of the photo clicked or opened.
    pub action_section: Option<usize>,
    /// The visible cells' responses, for context menus.
    pub cells: Vec<(FileRef, egui::Response)>,
    /// The section at the top of the view, and the scroll offset.
    pub top_section: Option<usize>,
    pub offset: f32,
    /// Whether the grid itself scrolled this frame (to a section or photo).
    pub programmatic: bool,
    /// The photo under the pointer.
    pub hovered: Option<FileRef>,
    /// "Add Folder" was clicked on a missing photo's tile.
    pub add_folder: Option<PathBuf>,
}

/// How many cells fit across `width`, with `spacing` between them.
pub fn columns_for(width: f32, thumb_size: f32, spacing: f32) -> usize {
    (((width + spacing) / (thumb_size + 8.0 + spacing)).floor() as usize).max(1)
}

const HEADER_HEIGHT: f32 = 44.0;
const NOTE_HEIGHT: f32 = 18.0;

/// Where everything goes, in content coordinates (top of the grid at y = 0).
struct Layout {
    columns: usize,
    cell: egui::Vec2,
    step: egui::Vec2,
    /// Per section: top of its header (or first row) and of its first row.
    tops: Vec<(f32, f32)>,
    height: f32,
}

impl Layout {
    fn new(view: &GridView<'_>, width: f32, spacing: egui::Vec2) -> Self {
        let text_height = if view.captions == CaptionMode::None { 0.0 } else { 18.0 };
        let cell = egui::vec2(view.thumb_size + 8.0, view.thumb_size + 8.0 + text_height);
        let step = cell + spacing;
        let columns = columns_for(width, view.thumb_size, spacing.x);
        let mut y = 0.0;
        let mut tops = Vec::new();
        for section in view.sections {
            let header = section
                .header
                .as_ref()
                .map_or(0.0, |h| HEADER_HEIGHT + if h.note.is_some() { NOTE_HEIGHT } else { 0.0 });
            tops.push((y, y + header));
            y += header + section.len().div_ceil(columns) as f32 * step.y;
        }
        Self { columns, cell, step, tops, height: y }
    }

    /// The section at height `y`.
    fn section_at(&self, y: f32) -> Option<usize> {
        self.tops.iter().rposition(|(top, _)| *top <= y)
    }

    fn cell_rect(&self, section: usize, index: usize) -> egui::Rect {
        let at = egui::pos2(
            (index % self.columns) as f32 * self.step.x,
            self.tops[section].1 + (index / self.columns) as f32 * self.step.y,
        );
        egui::Rect::from_min_size(at, self.cell)
    }
}

/// Draws the grid; only what is in view is laid out and painted.
pub fn show(ui: &mut egui::Ui, view: GridView<'_>, thumbs: &mut ThumbCache) -> GridOutput {
    let layout = Layout::new(&view, ui.available_width(), ui.spacing().item_spacing);
    let mut action = None;
    let mut action_section = None;
    let mut cells = Vec::new();
    let mut hovered = None;
    let mut add_folder = None;
    let band_id = ui.scope_id().with("selection band");
    let view_id = band_id.with("view");
    // Offset and height of the view last frame.
    let (last_offset, last_height) = ui.data(|d| d.get_temp::<(f32, f32)>(view_id)).unwrap_or((0.0, 0.0));

    // Mouse drags draw the band rather than scrolling; the wheel is
    // `Kinetic`'s, if it is on.
    let source = if view.kinetic {
        egui::scroll_area::ScrollSource::SCROLL_BAR
    } else {
        egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL
    };
    let mut area = egui::ScrollArea::vertical().auto_shrink(false).scroll_source(source);
    // Dragging the band past the top or bottom edge scrolls; set last frame.
    let mut offset = ui.data_mut(|d| d.remove_temp::<f32>(band_id.with("scroll")));
    if let Some(section) = view.scroll_to_section.filter(|&s| s < layout.tops.len()) {
        offset = Some(layout.tops[section].0);
    }
    let home = view.home.filter(|&s| s < view.sections.len());
    if let Some(at) = view.reveal
        && let Some(rect) = home.into_iter().chain(0..view.sections.len()).find_map(|s| {
            let section = &view.sections[s];
            section.items.iter().position(|(r, _)| *r == at).map(|i| layout.cell_rect(s, section.slot_of(i)))
        })
        && last_height > 0.0
    {
        if rect.top() < last_offset {
            offset = Some(rect.top());
        } else if rect.bottom() > last_offset + last_height {
            offset = Some(rect.bottom() - last_height);
        }
    }
    let programmatic = offset.is_some();
    // Never past the end: the scroll area would show that for a frame and
    // then jump back.
    let end = if last_height > 0.0 { (layout.height - last_height).max(0.0) } else { f32::INFINITY };
    let kinetic_id = band_id.with("kinetic");
    let mut kinetic: Kinetic = ui.data(|d| d.get_temp(kinetic_id)).unwrap_or_default();
    let wheel = if ui.rect_contains_pointer(ui.available_rect_before_wrap()) { wheel(ui) } else { (0.0, 0.0) };
    let (time, dt) = ui.input(|i| (i.time, i.stable_dt.min(0.05)));
    let (offset, stretch) = match offset {
        Some(offset) => (Some(kinetic.jump(offset.clamp(0.0, end))), 0.0),
        None if end.is_finite() && view.kinetic => kinetic.step(last_offset, end, wheel, time, dt),
        None => (None, 0.0),
    };
    if kinetic.moving(end) {
        ui.ctx().request_repaint();
    }
    ui.data_mut(|d| d.insert_temp(kinetic_id, kinetic));
    if let Some(offset) = offset {
        area = area.vertical_scroll_offset(offset);
    }

    // Under the thumbnails, so clicks reach them and drags reach the band.
    let band = ui.interact(ui.available_rect_before_wrap(), band_id, egui::Sense::drag());
    // The section a reorder drag started in: tiles only move within it.
    let drag_section_id = band_id.with("drag section");
    let output = area.show_viewport(ui, |ui, viewport| {
        let (content, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), layout.height), egui::Sense::hover());
        // Pulled past an end, everything moves along.
        let origin = content.min.to_vec2() + egui::vec2(0.0, stretch);
        for (s, section) in view.sections.iter().enumerate() {
            let (top, rows_top) = layout.tops[s];
            let bottom = layout.tops.get(s + 1).map_or(layout.height, |t| t.0);
            if bottom < viewport.top() || top > viewport.bottom() {
                continue;
            }
            if let Some(header) = &section.header {
                let rect = egui::Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(content.width(), rows_top));
                paint_header(ui, header, rect.translate(origin));
            }
            let first_row = ((viewport.top() - rows_top) / layout.step.y).floor().max(0.0) as usize;
            let last_row = ((viewport.bottom() - rows_top) / layout.step.y).ceil().max(0.0) as usize;
            let range = (first_row * layout.columns)..((last_row + 1) * layout.columns).min(section.len());
            for slot in range {
                let rect = layout.cell_rect(s, slot).translate(origin);
                let i = match section.slot(slot) {
                    Ok(i) => i,
                    Err(missing) => {
                        match paint_missing(ui, rect, view.thumb_size, (s, missing)) {
                            MissingTile::AddFolder => add_folder = missing.folder.clone(),
                            MissingTile::Dragged => ui.data_mut(|d| {
                                d.insert_temp(drag_section_id, s);
                            }),
                            MissingTile::Idle => {}
                        }
                        continue;
                    }
                };
                let (at, file) = section.items[i];
                let response = ui.interact(rect, ui.scope_id().with(("cell", s, at)), egui::Sense::click_and_drag());
                paint_cell(ui, &view, thumbs, rect, at, file);
                let name = file.name();
                let selected = view.selection.contains(&at);
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
                let mut tip = file.name();
                if let Some(caption) = &file.meta.caption {
                    tip = format!("{tip}\n{caption}");
                }
                let marks: Vec<&str> = badges(file).iter().map(|(_, meaning)| *meaning).collect();
                if !marks.is_empty() {
                    tip = format!("{tip}\n{}", marks.join(", "));
                }
                let response = response.on_hover_text(tip);
                if response.hovered() {
                    hovered = Some(at);
                }
                if response.double_clicked() {
                    action = Some(GridAction::Open(at));
                    action_section = Some(s);
                } else if response.clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    action = Some(GridAction::Click { at, toggle: modifiers.command, range: modifiers.shift });
                    action_section = Some(s);
                }
                if response.drag_started() {
                    ui.data_mut(|d| d.insert_temp(drag_section_id, s));
                    // A selected photo drags the whole selection along.
                    let photos = if selected { view.selection.iter().copied().collect() } else { vec![at] };
                    response.dnd_set_drag_payload(PhotoDrag(photos, Vec::new()));
                }
                cells.push((at, response));
            }
        }
        if view.reorder
            && let Some(drag) = egui::DragAndDrop::payload::<PhotoDrag>(ui.ctx())
            && let Some(pos) = ui.ctx().pointer_latest_pos().filter(|&p| ui.clip_rect().contains(p))
            && let Some(s) = layout.section_at((pos - origin).y)
            && ui.data(|d| d.get_temp::<usize>(drag_section_id)) == Some(s)
        {
            let len = view.sections[s].len();
            let gap = drop_index(&layout, s, len, pos - origin);
            if len > 0 {
                paint_insertion(ui, &layout, s, gap, origin);
            }
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                let (photos, missing) = (drag.0.clone(), drag.1.clone());
                action = Some(GridAction::Reorder { section: s, photos, missing, to: gap.0 });
            }
        }
    });
    let offset = output.state.clamped_offset().y;
    ui.data_mut(|d| d.insert_temp(view_id, (offset, output.inner_rect.height())));
    let top_section = layout.section_at(offset + 1.0);

    // The band lives in content coordinates, so it keeps photos that scrolled away.
    let viewport = output.inner_rect;
    let origin = viewport.min - output.state.clamped_offset();
    if band.drag_started()
        && let Some(pos) = band.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(band_id, pos - origin));
    }
    let start = ui.data(|d| d.get_temp::<egui::Vec2>(band_id));
    let moving = band.dragged() || band.drag_stopped();
    // egui counts a press on a drag-only area as a drag at once; a plain click
    // must stay a click, so the band starts only after the pointer moves.
    let active_id = band_id.with("active");
    let pos = ui.ctx().pointer_latest_pos();
    if let (Some(start), Some(pos), true) = (start, pos, moving)
        && !ui.data(|d| d.get_temp::<bool>(active_id).unwrap_or(false))
        && (pos - origin - start).length() > 6.0
    {
        ui.data_mut(|d| d.insert_temp(active_id, true));
    }
    if !moving {
        ui.data_mut(|d| d.remove_temp::<bool>(active_id));
    }
    let active = ui.data(|d| d.get_temp::<bool>(active_id).unwrap_or(false));
    if let (Some(start), Some(pos), true) = (start, pos, moving && active && action.is_none()) {
        let area = egui::Rect::from_two_pos(start.to_pos2(), (pos - origin).to_pos2());
        let hits = view
            .sections
            .iter()
            .enumerate()
            .flat_map(|(s, section)| {
                section.items.iter().enumerate().map(move |(i, item)| (s, section.slot_of(i), item.0))
            })
            .filter(|&(s, i, _)| layout.cell_rect(s, i).intersects(area))
            .map(|(_, _, at)| at)
            .collect();
        let screen = area.translate(origin.to_vec2()).intersect(viewport);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, band_id));
        let color = ui.visuals().selection.bg_fill;
        painter.rect_filled(screen, 2.0, color.gamma_multiply(0.25));
        painter.rect_stroke(screen, 2.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
        let toggle = ui.input(|i| i.modifiers.command);
        let done = band.drag_stopped();
        action = Some(GridAction::Band { hits, toggle, done });
        if done {
            ui.data_mut(|d| {
                d.remove_temp::<egui::Vec2>(band_id);
                d.remove_temp::<bool>(active_id);
            });
        } else {
            let past =
                if pos.y < viewport.top() { pos.y - viewport.top() } else { (pos.y - viewport.bottom()).max(0.0) };
            if past != 0.0 {
                let offset = (offset + past * 0.5).max(0.0);
                ui.data_mut(|d| d.insert_temp(band_id.with("scroll"), offset));
                ui.ctx().request_repaint();
            }
        }
    }
    GridOutput { action, action_section, cells, top_section, offset, programmatic, hovered, add_folder }
}

/// The wheel this frame, in points the content moves down: (from a mouse
/// wheel, in steps; from fingers on a touchpad, smoothly).
fn wheel(ui: &egui::Ui) -> (f32, f32) {
    let line = ui.ctx().options(|o| o.input_options.line_scroll_speed);
    ui.input(|i| {
        let mut wheel = (0.0, 0.0);
        for event in &i.events {
            if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
                // Ctrl+wheel zooms the thumbnails instead.
                if modifiers.command || modifiers.ctrl {
                    continue;
                }
                match unit {
                    egui::MouseWheelUnit::Line => wheel.0 += delta.y * line,
                    egui::MouseWheelUnit::Page => wheel.0 += delta.y * i.viewport_rect().height(),
                    egui::MouseWheelUnit::Point => wheel.1 += delta.y,
                }
            }
        }
        wheel
    })
}

/// Scrolling with weight: the wheel throws the grid, which glides on a
/// little and slows down; past the top or bottom it stretches and springs
/// back. Offsets are as the scroll area's, `end` the largest.
#[derive(Clone, Copy, Default)]
struct Kinetic {
    /// Where the grid is, past an end while it springs back.
    position: f32,
    /// Points per second, the way the offset grows.
    velocity: f32,
    /// The offset given to the scroll area last frame.
    set: Option<f32>,
    /// When fingers on a touchpad last moved it.
    touched: f64,
}

/// How fast a glide slows down, per second.
const FRICTION: f32 = 5.0;
/// A wheel step glides about 1.6 times its own length.
const THROW: f32 = 1.6 * FRICTION;
/// How stiff the spring at the ends is, per second.
const SPRING: f32 = 14.0;
/// How far the grid can be pulled past an end, at most.
const STRETCH: f32 = 120.0;

impl Kinetic {
    fn moving(&self, end: f32) -> bool {
        self.velocity != 0.0 || self.position < 0.0 || self.position > end
    }

    /// Stops at `offset`, where the grid scrolls itself.
    fn jump(&mut self, offset: f32) -> f32 {
        *self = Kinetic { position: offset, set: Some(offset), ..Kinetic::default() };
        offset
    }

    /// Moves on by `dt` seconds with `wheel` from [`wheel`]: the offset for the
    /// scroll area (none to leave it be) and how far the content is pulled
    /// down past an end (up, when negative).
    fn step(&mut self, actual: f32, end: f32, wheel: (f32, f32), time: f64, dt: f32) -> (Option<f32>, f32) {
        // At rest, or the scroll bar was dragged: start from where it is
        // (the view may have changed size meanwhile).
        let moved_away = self.set.is_some_and(|set| (set - actual).abs() > 1.0);
        if moved_away || self.set.is_none() {
            *self = Kinetic { position: actual, touched: self.touched, ..Kinetic::default() };
        }
        let (thrown, dragged) = wheel;
        if thrown != 0.0 {
            // A throw the other way stops the glide first.
            if self.velocity * thrown > 0.0 {
                self.velocity = 0.0;
            }
            self.velocity -= thrown * THROW;
        }
        let bound = self.position.clamp(0.0, end);
        if dragged != 0.0 {
            // Fingers move it directly, harder past an end; when they let go
            // it glides on as fast as they moved.
            let step = -dragged * if bound == self.position { 1.0 } else { 0.5 };
            self.position += step;
            self.velocity = (self.velocity + step / dt.max(0.001)) / 2.0;
            self.touched = time;
        } else if time - self.touched > 0.05 {
            let over = self.position - bound;
            if over != 0.0 {
                let pull = -SPRING * SPRING * over - 2.0 * SPRING * self.velocity;
                self.velocity += pull * dt;
                self.position += self.velocity * dt;
                let left = self.position - bound;
                if left * over < 0.0 || (left.abs() < 0.5 && self.velocity.abs() < 20.0) {
                    self.position = bound;
                    self.velocity = 0.0;
                }
            } else {
                self.position += self.velocity * dt;
                self.velocity *= (-FRICTION * dt).exp();
                if self.velocity.abs() < 10.0 {
                    self.velocity = 0.0;
                }
            }
        }
        if !self.moving(end) && thrown == 0.0 && dragged == 0.0 && self.set.is_none() {
            return (None, 0.0);
        }
        let at = self.position.clamp(0.0, end);
        let over = self.position - at;
        self.set = (self.moving(end) || at != actual).then_some(at);
        (Some(at), -STRETCH * over / (STRETCH + over.abs()))
    }
}

/// What happened to a missing photo's tile this frame.
enum MissingTile {
    Idle,
    /// Its "Add Folder" button was clicked.
    AddFolder,
    /// It started being dragged.
    Dragged,
}

/// A black tile for an album photo Pico cannot show (in section `s`): which
/// file, and the folder to add.
fn paint_missing(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    thumb_size: f32,
    (s, missing): (usize, &MissingPhoto),
) -> MissingTile {
    let image_area = egui::Rect::from_min_size(rect.min, egui::vec2(thumb_size + 8.0, thumb_size + 8.0)).shrink(4.0);
    ui.painter().rect_filled(image_area, 2.0, egui::Color32::BLACK);
    // Short on the tile: the folder's own name; its whole path on hover.
    let (text, hover) = match &missing.folder {
        Some(folder) => {
            let name = folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into());
            (
                tr!("Not in Pico:\n{name}", name = name),
                tr!("Its folder is not in Pico:\n{folder}", folder = folder.display()),
            )
        }
        None => (tr!("Moved or deleted").to_owned(), tr!("Moved or deleted").to_owned()),
    };
    // Dragged like a photo, to arrange the album.
    let tile = ui.interact(image_area, ui.scope_id().with(("missing", s, missing.at)), egui::Sense::drag());
    tile.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &missing.name));
    let dragged = tile.drag_started();
    if dragged {
        tile.dnd_set_drag_payload(PhotoDrag(Vec::new(), vec![missing.at]));
    }
    tile.on_hover_text(format!("{}\n{hover}", missing.name));
    let width = image_area.width() - 12.0;
    let color = egui::Color32::from_gray(200);
    let body = egui::TextStyle::Body.resolve(ui.style());
    // Each line centred, for the text to sit in the middle of the tile.
    let centred = |text: String, font: egui::FontId, color: egui::Color32| {
        let mut job = egui::text::LayoutJob::simple(text, font, color, width);
        job.halign = egui::Align::Center;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let galleys = [centred(missing.name.clone(), body.clone(), egui::Color32::WHITE), centred(text, body, color)];
    let button = egui::Button::new(tr!("Add Folder"));
    let button_size = egui::vec2(width.min(100.0), 24.0);
    let gap = 4.0;
    let height = galleys.iter().map(|g| g.size().y + gap).sum::<f32>()
        + if missing.folder.is_some() { button_size.y } else { -gap };
    // Everything centred in the tile, the text a block above the button.
    let mut y = image_area.center().y - height / 2.0;
    let painter = ui.painter().with_clip_rect(image_area);
    for galley in galleys {
        let x = image_area.center().x;
        y += galley.size().y + gap;
        painter.galley(egui::pos2(x, y - galley.size().y - gap), galley, color);
    }
    let add = missing.folder.is_some()
        && ui
            .put(
                egui::Rect::from_center_size(egui::pos2(image_area.center().x, y + button_size.y / 2.0), button_size),
                button,
            )
            .on_hover_text(tr!("Add this folder to Pico"))
            .clicked();
    match (add, dragged) {
        (true, _) => MissingTile::AddFolder,
        (_, true) => MissingTile::Dragged,
        _ => MissingTile::Idle,
    }
}

/// Which gap between photos a drop at `at` (content coordinates) goes to:
/// before the photo at the index, and whether its bar shows after the photo
/// before (at the end of a row, or of all).
fn drop_index(layout: &Layout, section: usize, len: usize, at: egui::Pos2) -> (usize, bool) {
    let row = ((at.y - layout.tops[section].1) / layout.step.y).floor().max(0.0) as usize;
    let column = (at.x / layout.step.x).round().clamp(0.0, layout.columns as f32) as usize;
    let to = (row * layout.columns + column).min(len);
    (to, to > 0 && (column == layout.columns || to == len))
}

/// A bar in the gap a dragged photo would land in.
fn paint_insertion(ui: &egui::Ui, layout: &Layout, section: usize, (to, after): (usize, bool), origin: egui::Vec2) {
    let gap = layout.step.x - layout.cell.x;
    let (x, cell) = if after {
        let cell = layout.cell_rect(section, to - 1);
        (cell.right() + gap / 2.0, cell)
    } else {
        let cell = layout.cell_rect(section, to);
        (cell.left() - gap / 2.0, cell)
    };
    let line = [egui::pos2(x, cell.top()), egui::pos2(x, cell.bottom())].map(|p| p + origin);
    let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("insertion")));
    painter.line_segment(line, egui::Stroke::new(3.0, ui.visuals().selection.bg_fill));
}

fn paint_header(ui: &egui::Ui, header: &SectionHeader, rect: egui::Rect) {
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    let line = rect.top() + 10.0;
    painter.hline(rect.x_range(), line - 4.0, visuals.widgets.noninteractive.bg_stroke);
    let title = painter.text(
        egui::pos2(rect.left() + 2.0, line + 4.0),
        egui::Align2::LEFT_TOP,
        &header.title,
        egui::FontId::proportional(18.0),
        visuals.strong_text_color(),
    );
    painter.text(
        egui::pos2(title.right() + 12.0, title.bottom() - 2.0),
        egui::Align2::LEFT_BOTTOM,
        &header.detail,
        egui::FontId::proportional(13.0),
        visuals.weak_text_color(),
    );
    if let Some(note) = &header.note {
        painter.text(
            egui::pos2(rect.left() + 2.0, title.bottom() + 4.0),
            egui::Align2::LEFT_TOP,
            note,
            egui::FontId::proportional(13.0),
            visuals.text_color(),
        );
    }
    let label = format!("{}, {}", header.title, header.detail);
    let response = ui.interact(rect, ui.scope_id().with(("header", &header.title)), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, true, &label));
}

/// Small marks in a thumbnail's top-right corner, with what they mean.
fn badges(file: &MediaFile) -> Vec<(&'static str, &'static str)> {
    let mut badges = Vec::new();
    if !file.meta.filters.0.is_empty() {
        badges.push(("✏", tr!("Edited")));
    }
    if file.gps.is_some() {
        badges.push(("📍", tr!("Has a location")));
    }
    badges
}

fn paint_cell(
    ui: &egui::Ui,
    view: &GridView<'_>,
    thumbs: &mut ThumbCache,
    rect: egui::Rect,
    at: FileRef,
    file: &MediaFile,
) {
    let cell = view.thumb_size + 8.0;
    let image_area = egui::Rect::from_min_size(rect.min, egui::vec2(cell, cell));
    let inner = image_area.shrink(4.0);
    let painter = ui.painter_at(rect);
    if view.selection.contains(&at) {
        painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
    }
    if view.current == Some(at) {
        painter.rect_stroke(rect.shrink(1.0), 4.0, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
    }
    // Where the picture itself is, inside the cell.
    let mut picture = inner;
    match thumbs.get(&file.path, file.kind, &file.meta) {
        Thumb::Ready(texture) => {
            let size = texture.size_vec2();
            let scale = (view.thumb_size / size.x.max(size.y)).min(1.0);
            let image_rect = egui::Rect::from_center_size(inner.center(), size * scale);
            picture = image_rect;
            // Hidden photos are shown faded when hidden photos are listed.
            let tint = if file.meta.hidden { egui::Color32::from_white_alpha(110) } else { egui::Color32::WHITE };
            painter.image(texture.id(), image_rect, full_uv(), tint);
        }
        Thumb::Pending => {
            painter.rect_filled(inner, 2.0, ui.visuals().faint_bg_color);
        }
        Thumb::Failed => {
            painter.rect_filled(inner, 2.0, ui.visuals().extreme_bg_color);
            painter.text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::proportional(24.0),
                ui.visuals().weak_text_color(),
            );
        }
    }
    if file.meta.star {
        paint_star(&painter, inner.left_bottom() + egui::vec2(4.0, -4.0), 16.0);
    }
    let mut corner = picture.right_top() + egui::vec2(-12.0, 12.0);
    for (icon, _) in badges(file) {
        painter.circle_filled(corner, 10.0, egui::Color32::from_black_alpha(150));
        painter.text(corner, egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(12.0), egui::Color32::WHITE);
        corner.x -= 22.0;
    }
    if file.kind == MediaKind::Video {
        painter.text(
            inner.right_bottom() + egui::vec2(-4.0, -4.0),
            egui::Align2::RIGHT_BOTTOM,
            "▶",
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
        );
    }
    if let Some(text) = view.captions.text(file) {
        let color = ui.visuals().text_color();
        let galley = painter.layout(text, egui::FontId::proportional(12.0), color, cell - 6.0);
        let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, image_area.max.y);
        painter.galley(pos, galley, color);
    }
}

pub fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

pub fn paint_star(painter: &egui::Painter, left_bottom: egui::Pos2, size: f32) {
    painter.text(
        left_bottom,
        egui::Align2::LEFT_BOTTOM,
        "★",
        egui::FontId::proportional(size),
        egui::Color32::from_rgb(255, 200, 40),
    );
}

#[cfg(test)]
mod tests {
    use super::Kinetic;

    /// Runs `kinetic` for `frames` frames at 60 fps, the first with `wheel`;
    /// the offsets it set and how far it stretched.
    fn run(kinetic: &mut Kinetic, end: f32, wheel: (f32, f32), frames: usize) -> Vec<(f32, f32)> {
        let mut actual = kinetic.position.clamp(0.0, end);
        let mut seen = Vec::new();
        for frame in 0..frames {
            let wheel = if frame == 0 { wheel } else { (0.0, 0.0) };
            let (offset, stretch) = kinetic.step(actual, end, wheel, frame as f64 / 60.0, 1.0 / 60.0);
            actual = offset.unwrap_or(actual);
            seen.push((actual, stretch));
        }
        seen
    }

    #[test]
    fn a_wheel_step_glides_on_and_stops() {
        let mut kinetic = Kinetic::default();
        let seen = run(&mut kinetic, 1000.0, (-40.0, 0.0), 120);
        let last = seen.last().unwrap().0;
        assert!(last > 50.0 && last < 80.0, "{last}");
        assert!(seen.windows(2).all(|w| w[1].0 >= w[0].0), "never back");
        assert!(!kinetic.moving(1000.0));
    }

    #[test]
    fn past_the_top_it_stretches_and_springs_back() {
        let mut kinetic = Kinetic::default();
        let seen = run(&mut kinetic, 1000.0, (200.0, 0.0), 120);
        assert!(seen.iter().all(|&(offset, _)| offset == 0.0));
        let most = seen.iter().map(|s| s.1).fold(0.0, f32::max);
        assert!(most > 10.0 && most < super::STRETCH, "pulled down {most}");
        assert_eq!(seen.last().unwrap().1, 0.0, "back in place");
        assert!(!kinetic.moving(1000.0));
    }
}
