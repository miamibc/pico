//! The thumbnail grid of the current listing (a folder, search results, …).

use crate::thumbs::{Thumb, ThumbCache};
use eframe::egui;
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use std::collections::BTreeSet;

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
}

/// Photos being dragged out of the grid, to a folder, an album or the tray.
#[derive(Clone)]
pub struct PhotoDrag(pub Vec<FileRef>);

pub struct GridView<'a> {
    pub items: &'a [(FileRef, &'a MediaFile)],
    pub thumb_size: f32,
    pub selection: &'a BTreeSet<FileRef>,
    pub current: Option<FileRef>,
    pub captions: CaptionMode,
}

pub fn columns_for(width: f32, thumb_size: f32) -> usize {
    ((width / (thumb_size + 8.0)).floor() as usize).max(1)
}

/// Draws the grid. Besides the action, returns the visible cells' responses,
/// so the caller can attach context menus to them.
pub fn show(
    ui: &mut egui::Ui,
    view: GridView<'_>,
    thumbs: &mut ThumbCache,
) -> (Option<GridAction>, Vec<(FileRef, egui::Response)>) {
    let cell = view.thumb_size + 8.0;
    let text_height = if view.captions == CaptionMode::None { 0.0 } else { 18.0 };
    let columns = columns_for(ui.available_width(), view.thumb_size);
    let rows = view.items.len().div_ceil(columns);
    let mut action = None;
    let mut cells = Vec::new();
    let step = egui::vec2(cell + ui.spacing().item_spacing.x, cell + text_height + ui.spacing().item_spacing.y);
    let band_id = ui.scope_id().with("selection band");
    // Dragging the band past the top or bottom edge scrolls; set last frame.
    // Mouse drags draw the band rather than scrolling.
    let mut area = egui::ScrollArea::vertical()
        .auto_shrink(false)
        .scroll_source(egui::scroll_area::ScrollSource::SCROLL_BAR | egui::scroll_area::ScrollSource::MOUSE_WHEEL);
    if let Some(offset) = ui.data_mut(|d| d.remove_temp::<f32>(band_id.with("scroll"))) {
        area = area.vertical_scroll_offset(offset);
    }

    // Under the thumbnails, so clicks reach them and drags reach the band.
    let band = ui.interact(ui.available_rect_before_wrap(), band_id, egui::Sense::drag());
    let output = area.show_rows(ui, cell + text_height, rows, |ui, row_range| {
        for row in row_range {
            ui.horizontal(|ui| {
                for &(at, file) in view.items.iter().skip(row * columns).take(columns) {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(cell, cell + text_height), egui::Sense::click_and_drag());
                    let image_area = egui::Rect::from_min_size(rect.min, egui::vec2(cell, cell));
                    let inner = image_area.shrink(4.0);
                    let painter = ui.painter_at(rect);
                    let selected = view.selection.contains(&at);
                    if selected {
                        painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
                    }
                    if view.current == Some(at) {
                        painter.rect_stroke(
                            rect.shrink(1.0),
                            4.0,
                            ui.visuals().selection.stroke,
                            egui::StrokeKind::Inside,
                        );
                    }
                    match thumbs.get(&file.path, file.kind, &file.meta) {
                        Thumb::Ready(texture) => {
                            let size = texture.size_vec2();
                            let scale = (view.thumb_size / size.x.max(size.y)).min(1.0);
                            let image_rect = egui::Rect::from_center_size(inner.center(), size * scale);
                            // Hidden photos are shown faded when hidden photos are listed.
                            let tint = if file.meta.hidden {
                                egui::Color32::from_white_alpha(110)
                            } else {
                                egui::Color32::WHITE
                            };
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
                    let name = file.name();
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
                    let response = response.on_hover_text(match &file.meta.caption {
                        Some(caption) => format!("{}\n{caption}", file.name()),
                        None => file.name(),
                    });
                    if response.double_clicked() {
                        action = Some(GridAction::Open(at));
                    } else if response.clicked() {
                        let modifiers = ui.input(|i| i.modifiers);
                        action = Some(GridAction::Click { at, toggle: modifiers.command, range: modifiers.shift });
                    }
                    if response.drag_started() {
                        // A selected photo drags the whole selection along.
                        let photos = if view.selection.contains(&at) {
                            view.selection.iter().copied().collect()
                        } else {
                            vec![at]
                        };
                        response.dnd_set_drag_payload(PhotoDrag(photos));
                    }
                    cells.push((at, response));
                }
            });
        }
    });

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
            .items
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let at = egui::pos2((i % columns) as f32 * step.x, (i / columns) as f32 * step.y);
                egui::Rect::from_min_size(at, egui::vec2(cell, cell + text_height)).intersects(area)
            })
            .map(|(_, &(at, _))| at)
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
                let offset = (output.state.clamped_offset().y + past * 0.5).max(0.0);
                ui.data_mut(|d| d.insert_temp(band_id.with("scroll"), offset));
                ui.ctx().request_repaint();
            }
        }
    }
    (action, cells)
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
