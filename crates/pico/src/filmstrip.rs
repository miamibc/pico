//! The strip of neighbouring photos above the viewer, as in Picasa's Edit
//! View. Only the thumbnails in sight are drawn, so long folders stay quick.

use crate::grid::{self, FileRef};
use crate::thumbs::{Thumb, ThumbCache};
use eframe::egui;
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;

/// Height of a cell, thumbnail plus margin.
pub const CELL: f32 = 64.0;

/// Draws the strip and returns the photo clicked, if any. The current photo
/// is scrolled to the middle whenever it changes. `label` names each
/// thumbnail for screen readers, from the file name.
pub fn show(
    ui: &mut egui::Ui,
    items: &[(FileRef, &MediaFile)],
    current: Option<FileRef>,
    thumbs: &mut ThumbCache,
    label: impl Fn(&str) -> String,
) -> Option<FileRef> {
    let centered_on = ui.scope_id().with("filmstrip centered on");
    let mut area = egui::ScrollArea::horizontal().id_salt("filmstrip").auto_shrink([false, true]);
    if let Some(current) = current
        && ui.data(|d| d.get_temp::<FileRef>(centered_on)) != Some(current)
        && let Some(index) = items.iter().position(|(at, _)| *at == current)
    {
        let offset = (index as f32 + 0.5) * CELL - ui.available_width() / 2.0;
        area = area.horizontal_scroll_offset(offset.max(0.0));
        ui.data_mut(|d| d.insert_temp(centered_on, current));
    }

    let mut clicked = None;
    area.show_viewport(ui, |ui, viewport| {
        // A strip shorter than the window sits in the middle.
        let length = items.len() as f32 * CELL;
        let pad = ((ui.available_width() - length) / 2.0).max(0.0);
        let (strip, _) = ui.allocate_exact_size(egui::vec2(length + pad, CELL), egui::Sense::hover());
        let first = ((viewport.min.x - pad) / CELL).floor().max(0.0) as usize;
        let last = (((viewport.max.x - pad) / CELL).ceil().max(0.0) as usize).min(items.len());
        for (i, &(at, file)) in items.iter().enumerate().take(last).skip(first) {
            let left = strip.min.x + pad + i as f32 * CELL;
            let rect = egui::Rect::from_min_size(egui::pos2(left, strip.min.y), egui::vec2(CELL, CELL));
            let response = ui.interact(rect, ui.scope_id().with(("filmstrip", at)), egui::Sense::click());
            let painter = ui.painter_at(rect);
            let inner = rect.shrink(4.0);
            if Some(at) == current {
                painter.rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
            } else if response.hovered() {
                painter.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
            }
            match thumbs.get(&file.path, file.kind, &file.meta) {
                Thumb::Ready(texture) => {
                    let size = texture.size_vec2();
                    let scale = (inner.width() / size.x).min(inner.height() / size.y);
                    let image = egui::Rect::from_center_size(inner.center(), size * scale);
                    painter.image(texture.id(), image, grid::full_uv(), egui::Color32::WHITE);
                }
                Thumb::Pending | Thumb::Failed => {
                    painter.rect_filled(inner, 2.0, ui.visuals().faint_bg_color);
                }
            }
            if file.meta.star {
                grid::paint_star(&painter, inner.left_bottom() + egui::vec2(1.0, -1.0), 12.0);
            }
            if file.kind == MediaKind::Video {
                painter.text(
                    inner.right_bottom() - egui::vec2(2.0, 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    "▶",
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
            let name = file.name();
            let label = label(&name);
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, Some(at) == current, &label));
            if response.on_hover_text(&name).clicked() && Some(at) != current {
                clicked = Some(at);
            }
        }
    });
    clicked
}
