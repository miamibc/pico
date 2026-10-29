//! Interactive edit tools drawn over the photo: Crop and Straighten.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2, pos2};
use pico_core::adjust::MAX_TILT_DEGREES;
use pico_core::effects::{self, Param, ParamValue, Spec};
use pico_core::filters::{Filter, Value};
use pico_core::rect64::Rect64;

pub enum Tool {
    Crop(CropTool),
    Straighten {
        amount: f32,
    },
    /// Adjusting an effect's parameters before adding it to the stack.
    Effect {
        spec: &'static Spec,
        params: Vec<ParamValue>,
    },
}

pub enum ToolAction {
    Apply(Filter),
    Cancel,
}

/// Picasa's crop presets; ratios are width / height in landscape.
const ASPECTS: [(&str, Option<f32>); 7] = [
    (tr_mark!("Manual"), None),
    ("4 x 6", Some(1.5)),
    ("5 x 7", Some(1.4)),
    ("8 x 10", Some(1.25)),
    (tr_mark!("Square"), Some(1.0)),
    ("16 x 9", Some(16.0 / 9.0)),
    (tr_mark!("As shot"), Some(0.0)), // resolved to the photo's own ratio
];

#[derive(Default)]
pub struct CropTool {
    /// Selection in unit coordinates of the photo as displayed (after rotation).
    pub rect: Option<Rect>,
    aspect: usize,
    portrait: bool,
    drag: Option<Drag>,
}

enum Drag {
    New(Pos2),
    Move(Vec2),
}

impl CropTool {
    /// Width / height of the selection in unit space, if the aspect is locked.
    fn unit_ratio(&self, image_px: Vec2) -> Option<f32> {
        let mut ratio = ASPECTS[self.aspect].1?;
        if ratio == 0.0 {
            ratio = image_px.x / image_px.y;
        } else if self.portrait == (ratio >= 1.0) {
            ratio = 1.0 / ratio;
        }
        Some(ratio * image_px.y / image_px.x)
    }

    /// Handles dragging on the photo: drag outside the selection to draw a new
    /// one, inside to move it.
    pub fn interact(&mut self, response: &egui::Response, image_rect: Rect, image_px: Vec2) {
        let to_unit = |p: Pos2| {
            let v = (p - image_rect.min) / image_rect.size();
            pos2(v.x.clamp(0.0, 1.0), v.y.clamp(0.0, 1.0))
        };
        // Anchor at the press, not where egui recognised the drag a few pixels later.
        let press = response.ctx.input(|i| i.pointer.press_origin());
        if response.drag_started()
            && let Some(p) = press.or(response.interact_pointer_pos())
        {
            let p = to_unit(p);
            self.drag = Some(match self.rect {
                Some(rect) if rect.contains(p) => Drag::Move(p - rect.min),
                _ => Drag::New(p),
            });
        }
        if response.dragged()
            && let Some(p) = response.interact_pointer_pos()
        {
            let p = to_unit(p);
            match self.drag {
                Some(Drag::New(anchor)) => self.rect = Some(self.fit(anchor, p, image_px)),
                Some(Drag::Move(offset)) => {
                    if let Some(rect) = self.rect {
                        let size = rect.size();
                        let min = (p - offset).clamp(Pos2::ZERO, pos2(1.0 - size.x, 1.0 - size.y));
                        self.rect = Some(Rect::from_min_size(min, size));
                    }
                }
                None => {}
            }
        }
        if response.drag_stopped() {
            self.drag = None;
            if self.rect.is_some_and(|r| r.width() < 0.01 || r.height() < 0.01) {
                self.rect = None;
            }
        }
    }

    /// The rectangle from `anchor` towards `to`, honouring the locked aspect
    /// and staying inside the photo.
    fn fit(&self, anchor: Pos2, to: Pos2, image_px: Vec2) -> Rect {
        let Some(ratio) = self.unit_ratio(image_px) else { return Rect::from_two_pos(anchor, to) };
        let (dx, dy) = (to.x - anchor.x, to.y - anchor.y);
        let mut w = dx.abs().max(dy.abs() * ratio);
        // Room left in the drag direction limits the size.
        let room_x = if dx < 0.0 { anchor.x } else { 1.0 - anchor.x };
        let room_y = if dy < 0.0 { anchor.y } else { 1.0 - anchor.y };
        w = w.min(room_x).min(room_y * ratio);
        let h = w / ratio;
        let corner = pos2(anchor.x + w.copysign(dx), anchor.y + h.copysign(dy));
        Rect::from_two_pos(anchor, corner)
    }

    /// Re-shapes the current selection after the aspect changes.
    fn refit(&mut self, image_px: Vec2) {
        if let Some(rect) = self.rect {
            let fitted = self.fit(rect.min, rect.max, image_px);
            self.rect = Some(fitted);
        }
    }

    pub fn paint(&self, painter: &egui::Painter, image_rect: Rect) {
        let Some(unit) = self.rect else {
            painter.text(
                image_rect.center(),
                egui::Align2::CENTER_CENTER,
                tr!("Drag over the photo to choose what stays"),
                egui::FontId::proportional(18.0),
                Color32::WHITE,
            );
            return;
        };
        let rect = Rect::from_min_max(
            image_rect.min + unit.min.to_vec2() * image_rect.size(),
            image_rect.min + unit.max.to_vec2() * image_rect.size(),
        );
        let shade = Color32::from_black_alpha(160);
        let outer = image_rect;
        for part in [
            Rect::from_min_max(outer.min, pos2(outer.max.x, rect.min.y)),
            Rect::from_min_max(pos2(outer.min.x, rect.max.y), outer.max),
            Rect::from_min_max(pos2(outer.min.x, rect.min.y), pos2(rect.min.x, rect.max.y)),
            Rect::from_min_max(pos2(rect.max.x, rect.min.y), pos2(outer.max.x, rect.max.y)),
        ] {
            painter.rect_filled(part, 0.0, shade);
        }
        let thirds = Stroke::new(1.0, Color32::from_white_alpha(90));
        for i in 1..3 {
            let t = i as f32 / 3.0;
            let x = rect.min.x + rect.width() * t;
            let y = rect.min.y + rect.height() * t;
            painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], thirds);
            painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], thirds);
        }
        painter.rect_stroke(rect, 0.0, Stroke::new(2.0, Color32::WHITE), egui::StrokeKind::Outside);
    }

    pub fn controls(&mut self, ui: &mut egui::Ui, image_px: Vec2, rotate: u8) -> Option<ToolAction> {
        ui.heading(tr!("Crop"));
        ui.label(tr!("Drag over the photo. Everything outside the box goes away (not really: Undo brings it back)."));
        let before = (self.aspect, self.portrait);
        egui::ComboBox::from_label(tr!("Aspect")).selected_text(crate::i18n::tr(ASPECTS[self.aspect].0)).show_ui(
            ui,
            |ui| {
                for (i, (name, _)) in ASPECTS.iter().enumerate() {
                    ui.selectable_value(&mut self.aspect, i, crate::i18n::tr(name));
                }
            },
        );
        ui.add_enabled(
            ASPECTS[self.aspect].1.is_some_and(|r| r != 1.0),
            egui::Checkbox::new(&mut self.portrait, tr!("Portrait")),
        );
        if before != (self.aspect, self.portrait) {
            self.refit(image_px);
        }
        let mut action = None;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.rect.is_some(), egui::Button::new(tr!("Apply"))).on_hover_text("Enter").clicked() {
                action = self.apply(rotate);
            }
            if ui.button(tr!("Cancel")).on_hover_text("Esc").clicked() {
                action = Some(ToolAction::Cancel);
            }
            if ui.add_enabled(self.rect.is_some(), egui::Button::new(tr!("Reset"))).clicked() {
                self.rect = None;
            }
        });
        action
    }

    /// The `crop64` edit for the selection. Picasa applies the quarter-turn
    /// rotation after the edit stack, so the selection is mapped back from the
    /// rotated view to the unrotated photo.
    pub fn apply(&self, rotate: u8) -> Option<ToolAction> {
        let rect = self.rect?;
        let source = Rect::from_two_pos(unrotate_point(rect.min, rotate), unrotate_point(rect.max, rotate));
        let rect64 = Rect64::from_unit([source.min.x, source.min.y, source.max.x, source.max.y]);
        Some(ToolAction::Apply(Filter::new("crop64", vec!["1".to_owned(), rect64.to_string()])))
    }
}

/// Maps a unit point on the photo as displayed back to the unrotated photo,
/// since Picasa applies the quarter-turn rotation after the edit stack.
pub fn unrotate_point(p: Pos2, rotate: u8) -> Pos2 {
    match rotate % 4 {
        1 => pos2(p.y, 1.0 - p.x),
        2 => pos2(1.0 - p.x, 1.0 - p.y),
        3 => pos2(1.0 - p.y, p.x),
        _ => p,
    }
}

/// The inverse of [`unrotate_point`].
pub fn rotate_point(p: Pos2, rotate: u8) -> Pos2 {
    (0..(4 - rotate % 4) % 4).fold(p, |p, _| unrotate_point(p, 1))
}

/// Controls for an effect's parameters, built from its spec.
pub fn effect_controls(ui: &mut egui::Ui, spec: &Spec, params: &mut [ParamValue]) -> Option<ToolAction> {
    ui.heading(crate::i18n::tr(spec.label));
    for (param, value) in spec.params.iter().zip(params.iter_mut()) {
        match (*param, value) {
            (Param::Point, ParamValue::Point(..)) => {
                ui.label(tr!("Click the photo to place the center."));
            }
            (Param::Slider { label, min, max, .. }, ParamValue::Number(v)) => {
                ui.add(egui::Slider::new(v, min..=max).text(crate::i18n::tr(label)));
            }
            (Param::Check { label, .. }, ParamValue::Number(v)) => {
                let mut on = *v != 0.0;
                if ui.checkbox(&mut on, crate::i18n::tr(label)).changed() {
                    *v = if on { 1.0 } else { 0.0 };
                }
            }
            (Param::Color { label, .. }, ParamValue::Color(c)) => {
                ui.horizontal(|ui| {
                    let mut rgb = [(*c >> 16) as u8, (*c >> 8) as u8, *c as u8];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        *c = 0xff00_0000 | (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
                    }
                    ui.label(crate::i18n::tr(label));
                });
            }
            (Param::Rotation, ParamValue::Rotation(r)) => {
                ui.horizontal(|ui| {
                    ui.label(tr!("Direction"));
                    for (turns, arrow) in
                        [(0, tr!("From top")), (1, tr!("From right")), (2, tr!("From bottom")), (3, tr!("From left"))]
                    {
                        ui.selectable_value(r, turns, arrow);
                    }
                });
            }
            _ => {}
        }
    }
    let mut action = None;
    ui.horizontal(|ui| {
        if ui.button(tr!("Apply")).on_hover_text("Enter").clicked() {
            action = Some(ToolAction::Apply(effects::to_filter(spec, params)));
        }
        if ui.button(tr!("Cancel")).on_hover_text("Esc").clicked() {
            action = Some(ToolAction::Cancel);
        }
        if ui.button(tr!("Defaults")).clicked() {
            params.copy_from_slice(&effects::defaults(spec));
        }
    });
    action
}

pub fn straighten_filter(amount: f32) -> Filter {
    Filter::effect("tilt", [Value::Float(amount), Value::Float(0.0)])
}

pub fn straighten_controls(ui: &mut egui::Ui, amount: &mut f32) -> Option<ToolAction> {
    ui.heading(tr!("Straighten"));
    ui.label(tr!("Line up the horizon with the grid."));
    let mut degrees = *amount * MAX_TILT_DEGREES;
    ui.add(egui::Slider::new(&mut degrees, -MAX_TILT_DEGREES..=MAX_TILT_DEGREES).text(tr!("Angle")).suffix("°"));
    *amount = degrees / MAX_TILT_DEGREES;
    let mut action = None;
    ui.horizontal(|ui| {
        if ui.add_enabled(*amount != 0.0, egui::Button::new(tr!("Apply"))).on_hover_text("Enter").clicked() {
            action = Some(ToolAction::Apply(straighten_filter(*amount)));
        }
        if ui.button(tr!("Cancel")).on_hover_text("Esc").clicked() {
            action = Some(ToolAction::Cancel);
        }
    });
    action
}

pub fn paint_grid(painter: &egui::Painter, image_rect: Rect) {
    let stroke = Stroke::new(1.0, Color32::from_white_alpha(70));
    for i in 1..8 {
        let t = i as f32 / 8.0;
        let x = image_rect.min.x + image_rect.width() * t;
        let y = image_rect.min.y + image_rect.height() * t;
        painter.line_segment([pos2(x, image_rect.min.y), pos2(x, image_rect.max.y)], stroke);
        painter.line_segment([pos2(image_rect.min.x, y), pos2(image_rect.max.x, y)], stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::vec2;

    fn crop_of(rect: Rect, rotate: u8) -> String {
        let tool = CropTool { rect: Some(rect), ..Default::default() };
        match tool.apply(rotate) {
            Some(ToolAction::Apply(filter)) => filter.to_string(),
            _ => panic!("no crop"),
        }
    }

    #[test]
    fn maps_selection_back_through_rotation() {
        // Top-left quarter of the displayed photo.
        let quarter = Rect::from_min_max(pos2(0.0, 0.0), pos2(0.5, 0.5));
        // 0.5 * 65535 rounds to 0x8000.
        assert_eq!(crop_of(quarter, 0), "crop64=1,80008000;");
        // Shown rotated clockwise, the displayed top-left is the source bottom-left.
        assert_eq!(crop_of(quarter, 1), "crop64=1,80008000ffff;");
        assert_eq!(crop_of(quarter, 2), "crop64=1,80008000ffffffff;");
        // Shown rotated counterclockwise, it is the source top-right.
        assert_eq!(crop_of(quarter, 3), "crop64=1,80000000ffff8000;");
    }

    #[test]
    fn rotation_mapping_round_trips() {
        let p = pos2(0.2, 0.7);
        for r in 0..4 {
            let back = rotate_point(unrotate_point(p, r), r);
            assert!((back - p).length() < 1e-6, "rotate {r}: {back:?}");
        }
        // Clockwise view: the displayed top-right corner is the source top-left.
        assert_eq!(unrotate_point(pos2(1.0, 0.0), 1), pos2(0.0, 0.0));
    }

    #[test]
    fn locked_aspect_follows_photo_pixels() {
        let tool = CropTool { aspect: 4, ..Default::default() }; // Square
        let image_px = vec2(400.0, 200.0);
        let rect = tool.fit(pos2(0.0, 0.0), pos2(0.9, 0.9), image_px);
        // Square in pixels: unit width is half the unit height on a 2:1 photo.
        assert!((rect.width() * 400.0 - rect.height() * 200.0).abs() < 0.5, "{rect:?}");
        assert!(rect.max.y <= 1.0 && rect.max.x <= 1.0);
    }
}
