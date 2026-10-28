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
        (CaptionMode::None, "None"),
        (CaptionMode::Filename, "Filename"),
        (CaptionMode::Caption, "Caption"),
        (CaptionMode::Tags, "Tags"),
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
    /// Photos and missing photos' tiles dragged within the grid were let go
    /// before tile `to` (or after the last); only with `GridView::reorder`.
    Reorder {
        photos: Vec<FileRef>,
        missing: Vec<usize>,
        to: usize,
    },
}

/// Photos being dragged out of the grid, to a folder, an album or the tray.
#[derive(Clone)]
pub struct PhotoDrag(pub Vec<FileRef>, pub Vec<usize>);

/// A run of photos in the grid, under an optional header (a folder in the
/// ribbon of all folders).
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
    /// Photos can be dragged into a new order (an album's Custom Order).
    pub reorder: bool,
}

pub struct GridOutput {
    pub action: Option<GridAction>,
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
    let mut cells = Vec::new();
    let mut hovered = None;
    let mut add_folder = None;
    let band_id = ui.scope_id().with("selection band");
    let view_id = band_id.with("view");
    // Offset and height of the view last frame.
    let (last_offset, last_height) = ui.data(|d| d.get_temp::<(f32, f32)>(view_id)).unwrap_or((0.0, 0.0));

    // Mouse drags draw the band rather than scrolling.
    let mut area = egui::ScrollArea::vertical()
        .auto_shrink(false)
        .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL);
    // Dragging the band past the top or bottom edge scrolls; set last frame.
    let mut offset = ui.data_mut(|d| d.remove_temp::<f32>(band_id.with("scroll")));
    if let Some(section) = view.scroll_to_section.filter(|&s| s < layout.tops.len()) {
        offset = Some(layout.tops[section].0);
    }
    if let Some(at) = view.reveal
        && let Some(rect) = view.sections.iter().enumerate().find_map(|(s, section)| {
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
    if let Some(offset) = offset {
        // Never past the end: the scroll area would show that for a frame
        // and then jump back.
        let end = if last_height > 0.0 { (layout.height - last_height).max(0.0) } else { f32::INFINITY };
        area = area.vertical_scroll_offset(offset.clamp(0.0, end));
    }

    // Under the thumbnails, so clicks reach them and drags reach the band.
    let band = ui.interact(ui.available_rect_before_wrap(), band_id, egui::Sense::drag());
    let output = area.show_viewport(ui, |ui, viewport| {
        let (content, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), layout.height), egui::Sense::hover());
        let origin = content.min.to_vec2();
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
                        if paint_missing(ui, rect, view.thumb_size, missing) {
                            add_folder = missing.folder.clone();
                        }
                        continue;
                    }
                };
                let (at, file) = section.items[i];
                let response = ui.interact(rect, ui.scope_id().with(("cell", at)), egui::Sense::click_and_drag());
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
                } else if response.clicked() {
                    let modifiers = ui.input(|i| i.modifiers);
                    action = Some(GridAction::Click { at, toggle: modifiers.command, range: modifiers.shift });
                }
                if response.drag_started() {
                    // A selected photo drags the whole selection along.
                    let photos = if selected { view.selection.iter().copied().collect() } else { vec![at] };
                    response.dnd_set_drag_payload(PhotoDrag(photos, Vec::new()));
                }
                cells.push((at, response));
            }
        }
        if view.reorder
            && let Some(section) = view.sections.first()
            && let Some(drag) = egui::DragAndDrop::payload::<PhotoDrag>(ui.ctx())
            && let Some(pos) = ui.ctx().pointer_latest_pos().filter(|&p| ui.clip_rect().contains(p))
        {
            let gap = drop_index(&layout, section.len(), pos - origin);
            if section.len() > 0 {
                paint_insertion(ui, &layout, gap, origin);
            }
            if ui.input(|i| i.pointer.any_released()) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                action = Some(GridAction::Reorder { photos: drag.0.clone(), missing: drag.1.clone(), to: gap.0 });
            }
        }
    });
    let offset = output.state.clamped_offset().y;
    ui.data_mut(|d| d.insert_temp(view_id, (offset, output.inner_rect.height())));
    let top_section = layout.tops.iter().rposition(|(top, _)| *top <= offset + 1.0);

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
    GridOutput { action, cells, top_section, offset, programmatic, hovered, add_folder }
}

/// A black tile for an album photo Pico cannot show: which file, and the
/// folder to add; true when its "Add Folder" button was clicked.
fn paint_missing(ui: &mut egui::Ui, rect: egui::Rect, thumb_size: f32, missing: &MissingPhoto) -> bool {
    let image_area = egui::Rect::from_min_size(rect.min, egui::vec2(thumb_size + 8.0, thumb_size + 8.0)).shrink(4.0);
    ui.painter().rect_filled(image_area, 2.0, egui::Color32::BLACK);
    // Short on the tile: the folder's own name; its whole path on hover.
    let (text, hover) = match &missing.folder {
        Some(folder) => {
            let name = folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into());
            (format!("Not in Pico:\n{name}"), format!("Its folder is not in Pico:\n{}", folder.display()))
        }
        None => ("Moved or deleted".to_owned(), "Moved or deleted".to_owned()),
    };
    // Dragged like a photo, to arrange the album.
    let tile = ui.interact(image_area, ui.scope_id().with(("missing", missing.at)), egui::Sense::drag());
    tile.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &missing.name));
    if tile.drag_started() {
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
    let button = egui::Button::new("Add Folder");
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
    missing.folder.is_some()
        && ui
            .put(
                egui::Rect::from_center_size(egui::pos2(image_area.center().x, y + button_size.y / 2.0), button_size),
                button,
            )
            .on_hover_text("Add this folder to Pico")
            .clicked()
}

/// Which gap between photos a drop at `at` (content coordinates) goes to:
/// before the photo at the index, and whether its bar shows after the photo
/// before (at the end of a row, or of all).
fn drop_index(layout: &Layout, len: usize, at: egui::Pos2) -> (usize, bool) {
    let row = ((at.y - layout.tops[0].1) / layout.step.y).floor().max(0.0) as usize;
    let column = (at.x / layout.step.x).round().clamp(0.0, layout.columns as f32) as usize;
    let to = (row * layout.columns + column).min(len);
    (to, to > 0 && (column == layout.columns || to == len))
}

/// A bar in the gap a dragged photo would land in.
fn paint_insertion(ui: &egui::Ui, layout: &Layout, (to, after): (usize, bool), origin: egui::Vec2) {
    let gap = layout.step.x - layout.cell.x;
    let (x, cell) = if after {
        let cell = layout.cell_rect(0, to - 1);
        (cell.right() + gap / 2.0, cell)
    } else {
        let cell = layout.cell_rect(0, to);
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
        badges.push(("✏", "Edited"));
    }
    if file.gps.is_some() {
        badges.push(("📍", "Has a location"));
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
