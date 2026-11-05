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
        /// The handle on the photo being dragged.
        grab: Option<Grab>,
    },
}

pub enum ToolAction {
    Apply(Filter),
    Cancel,
}

/// How a crop's sides relate.
#[derive(Clone, Copy, PartialEq)]
enum Ratio {
    Free,
    /// Width / height in landscape.
    Fixed(f32),
    /// The photo's own.
    AsShot,
    /// The screen's.
    Screen,
}

/// Picasa 3's crop presets: print sizes, then screens and frames.
const ASPECTS: [(&str, Ratio); 11] = [
    (tr_mark!("Manual"), Ratio::Free),
    ("4 x 6", Ratio::Fixed(1.5)),
    ("5 x 7", Ratio::Fixed(1.4)),
    ("8 x 10", Ratio::Fixed(1.25)),
    (tr_mark!("Square"), Ratio::Fixed(1.0)),
    ("4:3", Ratio::Fixed(4.0 / 3.0)),
    ("16:10", Ratio::Fixed(1.6)),
    ("16:9", Ratio::Fixed(16.0 / 9.0)),
    ("5:3", Ratio::Fixed(5.0 / 3.0)),
    (tr_mark!("As shot"), Ratio::AsShot),
    (tr_mark!("As the screen"), Ratio::Screen),
];

/// The aspect picked for crops, kept from photo to photo and between runs.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct CropShape {
    aspect: usize,
    portrait: bool,
}

impl CropShape {
    /// The aspect's name as saved in the settings, and whether it is turned
    /// to portrait.
    pub fn setting(self) -> (&'static str, bool) {
        (ASPECTS[self.aspect].0, self.portrait)
    }

    /// Back from [`Self::setting`]; an unknown name is Manual.
    pub fn from_setting(name: &str, portrait: bool) -> Self {
        Self { aspect: ASPECTS.iter().position(|(n, _)| *n == name).unwrap_or(0), portrait }
    }
}

#[derive(Default)]
pub struct CropTool {
    /// Selection in unit coordinates of the photo as displayed (after rotation).
    pub rect: Option<Rect>,
    aspect: usize,
    portrait: bool,
    /// The screen's size, for the aspect that follows it.
    pub screen: Vec2,
    drag: Option<Drag>,
}

enum Drag {
    New(Pos2),
    Move(Vec2),
    /// An edge or a corner pulled: `x` is -1 for the left edge, 1 for the
    /// right one, 0 for neither; `y` likewise for top and bottom.
    Resize {
        x: i8,
        y: i8,
        start: Rect,
    },
}

/// How close to an edge, in screen pixels, a press grabs it.
const GRAB: f32 = 8.0;

impl CropTool {
    pub fn new(shape: CropShape) -> Self {
        Self { aspect: shape.aspect, portrait: shape.portrait, ..Default::default() }
    }

    pub fn shape(&self) -> CropShape {
        CropShape { aspect: self.aspect, portrait: self.portrait }
    }

    /// Width / height of the selection in unit space, if the aspect is locked.
    fn unit_ratio(&self, image_px: Vec2) -> Option<f32> {
        let ratio = match ASPECTS[self.aspect].1 {
            Ratio::Free => return None,
            Ratio::AsShot => image_px.x / image_px.y,
            Ratio::Fixed(r) => r,
            Ratio::Screen if self.screen.min_elem() > 0.0 => self.screen.max_elem() / self.screen.min_elem(),
            Ratio::Screen => 16.0 / 9.0,
        };
        let ratio = if ASPECTS[self.aspect].1 != Ratio::AsShot && self.portrait { 1.0 / ratio } else { ratio };
        Some(ratio * image_px.y / image_px.x)
    }

    /// The selection's edges or corner near the screen point `p`, as the
    /// `x` and `y` of [`Drag::Resize`].
    fn edges_at(&self, image_rect: Rect, p: Pos2) -> Option<(i8, i8)> {
        let unit = self.rect?;
        let rect = Rect::from_min_max(
            image_rect.min + unit.min.to_vec2() * image_rect.size(),
            image_rect.min + unit.max.to_vec2() * image_rect.size(),
        );
        if !rect.expand(GRAB).contains(p) {
            return None;
        }
        let side = |p: f32, min: f32, max: f32| {
            let (to_min, to_max) = ((p - min).abs(), (p - max).abs());
            if to_min.min(to_max) > GRAB {
                0
            } else if to_min < to_max {
                -1
            } else {
                1
            }
        };
        let edges = (side(p.x, rect.min.x, rect.max.x), side(p.y, rect.min.y, rect.max.y));
        (edges != (0, 0)).then_some(edges)
    }

    /// Handles dragging on the photo: drag an edge or a corner of the
    /// selection to resize it, inside to move it, elsewhere to draw a new one.
    pub fn interact(&mut self, response: &egui::Response, image_rect: Rect, image_px: Vec2) {
        let to_unit = |p: Pos2| {
            let v = (p - image_rect.min) / image_rect.size();
            pos2(v.x.clamp(0.0, 1.0), v.y.clamp(0.0, 1.0))
        };
        // The pointer shows what a press would grab.
        let cursor = match &self.drag {
            Some(Drag::Resize { x, y, .. }) => Some(resize_cursor(*x, *y)),
            Some(Drag::Move(_)) => Some(egui::CursorIcon::Grabbing),
            Some(Drag::New(_)) => None,
            None => response.hover_pos().and_then(|p| match self.edges_at(image_rect, p) {
                Some((x, y)) => Some(resize_cursor(x, y)),
                None => self.rect.filter(|r| r.contains(to_unit(p))).map(|_| egui::CursorIcon::Grab),
            }),
        };
        if let Some(cursor) = cursor {
            response.ctx.set_cursor_icon(cursor);
        }
        // Anchor at the press, not where egui recognised the drag a few pixels later.
        let press = response.ctx.input(|i| i.pointer.press_origin());
        if response.drag_started()
            && let Some(screen) = press.or(response.interact_pointer_pos())
        {
            let p = to_unit(screen);
            self.drag = Some(match (self.rect, self.edges_at(image_rect, screen)) {
                (Some(start), Some((x, y))) => Drag::Resize { x, y, start },
                (Some(rect), None) if rect.contains(p) => Drag::Move(p - rect.min),
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
                Some(Drag::Resize { x, y, start }) => self.rect = Some(self.resize(start, x, y, p, image_px)),
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

    /// `start` with the edges `x` and `y` (as in [`Drag::Resize`]) pulled to
    /// `p`. A locked aspect holds: a corner keeps the opposite corner in
    /// place, an edge keeps the opposite edge and the middle of the others.
    fn resize(&self, start: Rect, x: i8, y: i8, p: Pos2, image_px: Vec2) -> Rect {
        let Some(ratio) = self.unit_ratio(image_px) else {
            let (mut a, mut b) = (start.min, start.max);
            match x {
                -1 => a.x = p.x,
                1 => b.x = p.x,
                _ => {}
            }
            match y {
                -1 => a.y = p.y,
                1 => b.y = p.y,
                _ => {}
            }
            return Rect::from_two_pos(a, b);
        };
        let opposite = |side: i8, min: f32, max: f32| if side < 0 { max } else { min };
        if x != 0 && y != 0 {
            let anchor = pos2(opposite(x, start.min.x, start.max.x), opposite(y, start.min.y, start.max.y));
            return self.fit(anchor, p, image_px);
        }
        // One edge: its size follows the pointer, the other side's follows the
        // aspect, centred where it was and kept inside the photo.
        let span = |anchor: f32, to: f32, limit: f32| {
            let room = if to < anchor { anchor } else { 1.0 - anchor };
            let size = (to - anchor).abs().min(room).min(limit);
            let end = anchor + size.copysign(to - anchor);
            (anchor.min(end), anchor.max(end))
        };
        let centred = |size: f32, centre: f32| {
            let min = (centre - size / 2.0).clamp(0.0, 1.0 - size);
            (min, min + size)
        };
        if x != 0 {
            let (x0, x1) = span(opposite(x, start.min.x, start.max.x), p.x, ratio);
            let (y0, y1) = centred((x1 - x0) / ratio, start.center().y);
            Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
        } else {
            let (y0, y1) = span(opposite(y, start.min.y, start.max.y), p.y, 1.0 / ratio);
            let (x0, x1) = centred((y1 - y0) * ratio, start.center().x);
            Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
        }
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
            let hint = tr!("Drag over the photo to choose what stays");
            let text = |offset: Vec2, color: Color32| {
                let (at, font) = (image_rect.center() + offset, egui::FontId::proportional(18.0));
                painter.text(at, egui::Align2::CENTER_CENTER, hint, font, color);
            };
            // A soft dark shadow, about a 3-pixel blur around the letters,
            // so the hint reads on light photos too: copies on rings around
            // it, faint ones farther out.
            for (radius, alpha) in [(1.0, 60), (2.0, 34), (3.0, 16)] {
                for i in 0..12 {
                    let angle = i as f32 * std::f32::consts::TAU / 12.0;
                    text(Vec2::angled(angle) * radius, Color32::from_black_alpha(alpha));
                }
            }
            text(Vec2::ZERO, Color32::WHITE);
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
        // Corners to take hold of; the edges between them pull too.
        for corner in [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()] {
            painter.rect_filled(Rect::from_center_size(corner, Vec2::splat(8.0)), 1.0, Color32::WHITE);
        }
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
        let turns = matches!(ASPECTS[self.aspect].1, Ratio::Screen)
            || matches!(ASPECTS[self.aspect].1, Ratio::Fixed(r) if r != 1.0);
        ui.add_enabled(turns, egui::Checkbox::new(&mut self.portrait, tr!("Portrait")));
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

/// The pointer over an edge (`x` or `y` alone) or a corner of the selection.
fn resize_cursor(x: i8, y: i8) -> egui::CursorIcon {
    match (x, y) {
        (_, 0) => egui::CursorIcon::ResizeHorizontal,
        (0, _) => egui::CursorIcon::ResizeVertical,
        _ if x == y => egui::CursorIcon::ResizeNwSe,
        _ => egui::CursorIcon::ResizeNeSw,
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

/// A handle on the photo for an effect's point, size or gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grab {
    /// The point the effect is centred on, or the middle of a gradient.
    Center,
    /// The edge of the circle the effect keeps clear (Size, Focal Size).
    Ring,
    /// Where a gradient starts: tinted fully up to here.
    Start,
    /// Where a gradient ends: no tint from here.
    End,
}

/// How near, in screen pixels, a press takes hold of a handle.
const HANDLE_REACH: f32 = 10.0;

/// The circle of an effect (see [`has_ring`]): the index of the value that
/// sizes it, and its radius on screen. `diagonal` is the photo's on screen,
/// `per_px` screen pixels per pixel of the photo.
fn ring(spec: &Spec, params: &[ParamValue], diagonal: f32, per_px: f32) -> Option<(usize, f32)> {
    let value = |i: usize| params.get(i).map(|p| p.number());
    let half = diagonal / 2.0;
    match spec.id {
        // Size -1..=1 is a radius of 0..=1 half diagonals.
        "radblur" | "radsat" => Some((1, (value(1)? + 1.0) / 2.0 * half)),
        "focalpixelate" => Some((2, value(2)? / 2.0 * half)),
        // Focal Size in pixels of the photo.
        "PicnikFocalPixelate" | "FocalZoom" => Some((2, value(2)? * per_px)),
        _ => None,
    }
}

/// Whether the effect keeps a circle around its centre, sized by a slider.
fn has_ring(spec: &Spec) -> bool {
    matches!(spec.id, "radblur" | "radsat" | "focalpixelate" | "PicnikFocalPixelate" | "FocalZoom")
}

/// The value at the ring's index that gives a radius of `radius` on screen.
fn ring_value(spec: &Spec, radius: f32, diagonal: f32, per_px: f32) -> f32 {
    let half = diagonal / 2.0;
    match spec.id {
        "radblur" | "radsat" => radius / half * 2.0 - 1.0,
        "focalpixelate" => radius / half * 2.0,
        _ => radius / per_px,
    }
}

/// A graduated tint along one side of the photo: `rotation` quarter turns
/// from the top. Positions along it run 0..1 from that side.
struct Gradient {
    rotation: i32,
    /// Full tint up to here, none from `end` on.
    start: f32,
    end: f32,
    /// Where across the gradient its handles sit.
    across: f32,
}

impl Gradient {
    fn read(params: &[ParamValue]) -> Option<Self> {
        let (ParamValue::Point(x, y), ParamValue::Number(feather), ParamValue::Rotation(rotation)) =
            (*params.first()?, *params.get(1)?, *params.get(4)?)
        else {
            return None;
        };
        let (at, across) = match rotation.rem_euclid(4) {
            1 => (1.0 - x, y),
            2 => (1.0 - y, x),
            3 => (x, y),
            _ => (y, x),
        };
        Some(Self { rotation: rotation.rem_euclid(4), start: at - feather / 2.0, end: at + feather / 2.0, across })
    }

    fn write(&self, params: &mut [ParamValue]) {
        let at = ((self.start + self.end) / 2.0).clamp(0.0, 1.0);
        let point = match self.rotation {
            1 => (1.0 - at, self.across),
            2 => (self.across, 1.0 - at),
            3 => (at, self.across),
            _ => (self.across, at),
        };
        params[0] = ParamValue::Point(point.0, point.1);
        params[1] = ParamValue::Number((self.end - self.start).clamp(0.0, 1.0));
        params[4] = ParamValue::Rotation(self.rotation);
    }

    /// The photo point (unit, before the quarter turns) at `along` and
    /// `across` the gradient.
    fn point(&self, along: f32, across: f32) -> Pos2 {
        match self.rotation {
            1 => pos2(1.0 - along, across),
            2 => pos2(across, 1.0 - along),
            3 => pos2(along, across),
            _ => pos2(across, along),
        }
    }

    /// How far along the gradient a photo point is.
    fn along(&self, p: Pos2) -> f32 {
        match self.rotation {
            1 => 1.0 - p.x,
            2 => 1.0 - p.y,
            3 => p.x,
            _ => p.y,
        }
    }

    /// Moves the start or the end to `along`; past the other one, the tint
    /// turns to come from the opposite side. Feather stays within 0..=1.
    fn pull(&mut self, which: Grab, along: f32) {
        let (moved, other) = if which == Grab::Start { (along, self.end) } else { (along, self.start) };
        let moved = moved.clamp(other - 1.0, other + 1.0);
        let (start, end) = if which == Grab::Start { (moved, other) } else { (other, moved) };
        if start <= end {
            (self.start, self.end) = (start, end);
        } else {
            // Seen from the other side, positions run the other way.
            self.rotation = (self.rotation + 2) % 4;
            (self.start, self.end) = (1.0 - start, 1.0 - end);
        }
    }
}

/// Draws the handles of an effect being adjusted over the photo shown in
/// `rect`, and lets the mouse move them: the centre, the circle around it,
/// or a gradient's start and end. A click elsewhere puts the centre there,
/// as in Picasa. `per_px` is screen pixels per pixel of the photo.
#[allow(clippy::too_many_arguments)]
pub fn effect_handles(
    painter: &egui::Painter,
    response: &egui::Response,
    rect: Rect,
    rotate: u8,
    per_px: f32,
    spec: &Spec,
    params: &mut [ParamValue],
    grab: &mut Option<Grab>,
) {
    let Some(point_at) = params.iter().position(|p| matches!(p, ParamValue::Point(..))) else { return };
    let to_screen = |p: Pos2| rect.min + rotate_point(p, rotate).to_vec2() * rect.size();
    let to_photo = |p: Pos2| {
        let shown = ((p - rect.min) / rect.size()).to_pos2();
        let p = unrotate_point(shown, rotate);
        pos2(p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0))
    };
    let diagonal = rect.size().length();
    let gradient = (spec.id == "dir_tint").then(|| Gradient::read(params)).flatten();
    let center = match params[point_at] {
        ParamValue::Point(x, y) => to_screen(pos2(x, y)),
        _ => return,
    };
    let circle = ring(spec, params, diagonal, per_px);
    // The gradient's lines across the photo, and their handles.
    let line = |g: &Gradient, along: f32| [to_screen(g.point(along, 0.0)), to_screen(g.point(along, 1.0))];
    let handle = |g: &Gradient, along: f32| to_screen(g.point(along, g.across));
    // Up and down the screen, or across it.
    let upright = gradient.as_ref().is_some_and(|g| (g.rotation + rotate as i32) % 2 == 0);

    let near = |p: Pos2| -> Option<Grab> {
        if let Some(g) = &gradient {
            let off = |along: f32| {
                let [a, b] = line(g, along);
                if upright { (p.y - a.y).abs() } else { (p.x - a.x).abs().min((p.x - b.x).abs()) }
            };
            if p.distance(handle(g, (g.start + g.end) / 2.0)) <= HANDLE_REACH {
                return Some(Grab::Center);
            }
            for (which, along) in [(Grab::Start, g.start), (Grab::End, g.end)] {
                if off(along) <= HANDLE_REACH {
                    return Some(which);
                }
            }
            return None;
        }
        if p.distance(center) <= HANDLE_REACH {
            return Some(Grab::Center);
        }
        circle.filter(|(_, r)| (p.distance(center) - r).abs() <= HANDLE_REACH).map(|_| Grab::Ring)
    };

    let press = response.ctx.input(|i| i.pointer.press_origin());
    if response.drag_started() {
        // A drag away from the handles takes the centre along.
        *grab = Some(press.and_then(near).unwrap_or(Grab::Center));
    }
    let pointer = response.interact_pointer_pos();
    let moved_to = match (response.clicked(), response.dragged(), *grab) {
        (true, _, _) => pointer.map(|p| (Grab::Center, p)),
        (_, true, Some(which)) => pointer.map(|p| (which, p)),
        _ => None,
    };
    if let Some((which, p)) = moved_to {
        match (which, (spec.id == "dir_tint").then(|| Gradient::read(params)).flatten()) {
            (Grab::Start | Grab::End, Some(mut g)) => {
                g.pull(which, g.along(to_photo(p)));
                g.write(params);
            }
            (Grab::Center, Some(mut g)) => {
                let half = (g.end - g.start) / 2.0;
                let at = g.along(to_photo(p));
                (g.start, g.end) = (at - half, at + half);
                g.across = match g.rotation % 2 {
                    0 => to_photo(p).x,
                    _ => to_photo(p).y,
                };
                g.write(params);
            }
            (Grab::Ring, _) => {
                if let Some((index, _)) = circle {
                    let value = ring_value(spec, p.distance(center), diagonal, per_px);
                    let (min, max) = match spec.params.get(index) {
                        Some(Param::Slider { min, max, .. }) => (*min, *max),
                        _ => (f32::MIN, f32::MAX),
                    };
                    params[index] = ParamValue::Number(value.clamp(min, max));
                }
            }
            _ => {
                let at = to_photo(p);
                params[point_at] = ParamValue::Point(at.x, at.y);
            }
        }
    }
    if response.drag_stopped() {
        *grab = None;
    }
    // The pointer tells what a press would take.
    let resize = if upright { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal };
    let cursor = match grab.or_else(|| response.hover_pos().and_then(near)) {
        Some(Grab::Center) => egui::CursorIcon::Move,
        Some(Grab::Start | Grab::End) => resize,
        Some(Grab::Ring) => egui::CursorIcon::ResizeHorizontal,
        None if response.hovered() => egui::CursorIcon::Crosshair,
        None => egui::CursorIcon::Default,
    };
    if cursor != egui::CursorIcon::Default {
        response.ctx.set_cursor_icon(cursor);
    }

    // Drawn from the values as they are now.
    let painter = painter.with_clip_rect(rect);
    let white = Stroke::new(2.0, Color32::WHITE);
    let dark = Stroke::new(4.0, Color32::from_black_alpha(140));
    let square = |at: Pos2| {
        let r = Rect::from_center_size(at, Vec2::splat(9.0));
        painter.rect_filled(r.expand(1.0), 1.0, Color32::from_black_alpha(160));
        painter.rect_filled(r, 1.0, Color32::WHITE);
    };
    let center = match params[point_at] {
        ParamValue::Point(x, y) => to_screen(pos2(x, y)),
        _ => return,
    };
    if let Some(g) = (spec.id == "dir_tint").then(|| Gradient::read(params)).flatten() {
        // The start solid, the end dashed; the middle holds the gradient.
        let [a, b] = line(&g, g.start);
        painter.line_segment([a, b], dark);
        painter.line_segment([a, b], white);
        let [a, b] = line(&g, g.end);
        painter.add(egui::Shape::dashed_line(&[a, b], dark, 8.0, 6.0));
        painter.add(egui::Shape::dashed_line(&[a, b], white, 8.0, 6.0));
        square(handle(&g, g.start));
        square(handle(&g, g.end));
        let middle = handle(&g, (g.start + g.end) / 2.0);
        painter.circle_stroke(middle, 7.0, dark);
        painter.circle_stroke(middle, 7.0, white);
        return;
    }
    if let Some((_, radius)) = ring(spec, params, diagonal, per_px) {
        painter.circle_stroke(center, radius, dark);
        painter.circle_stroke(center, radius, Stroke::new(1.5, Color32::WHITE));
        square(center + Vec2::X * radius);
    }
    painter.circle_stroke(center, 9.0, dark);
    painter.circle_stroke(center, 9.0, white);
}

/// Controls for an effect's parameters, built from its spec.
pub fn effect_controls(ui: &mut egui::Ui, spec: &Spec, params: &mut [ParamValue]) -> Option<ToolAction> {
    ui.heading(crate::i18n::tr(spec.label));
    for (param, value) in spec.params.iter().zip(params.iter_mut()) {
        match (*param, value) {
            (Param::Point, ParamValue::Point(..)) => {
                ui.label(match spec.id {
                    "dir_tint" => tr!(
                        "On the photo, drag the solid line (full tint) and the dashed one (no tint), or the circle between them to move both."
                    ),
                    _ if has_ring(spec) => {
                        tr!("Click or drag on the photo to place the center; drag the circle to change its size.")
                    }
                    _ => tr!("Click or drag on the photo to place the center."),
                });
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

    #[test]
    fn edges_and_corners_resize_the_selection() {
        let start = Rect::from_min_max(pos2(0.2, 0.2), pos2(0.6, 0.6));
        let image_px = vec2(400.0, 400.0);
        let manual = CropTool { rect: Some(start), ..Default::default() };
        // The right edge alone; then the top-left corner.
        assert_eq!(
            manual.resize(start, 1, 0, pos2(0.8, 0.9), image_px),
            Rect::from_min_max(pos2(0.2, 0.2), pos2(0.8, 0.6))
        );
        assert_eq!(
            manual.resize(start, -1, -1, pos2(0.1, 0.0), image_px),
            Rect::from_min_max(pos2(0.1, 0.0), pos2(0.6, 0.6))
        );

        let square = CropTool { rect: Some(start), aspect: 4, ..Default::default() };
        // A corner keeps the opposite one and the aspect.
        let rect = square.resize(start, 1, 1, pos2(0.9, 0.7), image_px);
        assert_eq!(rect.min, start.min);
        assert!((rect.width() - rect.height()).abs() < 1e-5 && (rect.width() - 0.7).abs() < 1e-5, "{rect:?}");
        // An edge keeps the opposite edge and the middle of the other sides.
        let rect = square.resize(start, 1, 0, pos2(0.8, 0.5), image_px);
        assert!((rect.min.x - 0.2).abs() < 1e-5 && (rect.width() - 0.6).abs() < 1e-5, "{rect:?}");
        assert!((rect.height() - 0.6).abs() < 1e-5 && (rect.center().y - 0.4).abs() < 1e-5, "{rect:?}");
        // It stops at the photo's edge: the height cannot pass 1.
        let rect = square.resize(start, 1, 0, pos2(1.0, 0.5), image_px);
        assert!(rect.max.x <= 1.0 && rect.min.y >= 0.0 && rect.max.y <= 1.0, "{rect:?}");
        assert!((rect.width() - rect.height()).abs() < 1e-5, "{rect:?}");
    }

    #[test]
    fn presses_near_the_edges_grab_them() {
        let tool =
            CropTool { rect: Some(Rect::from_min_max(pos2(0.25, 0.25), pos2(0.75, 0.75))), ..Default::default() };
        let image = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
        assert_eq!(tool.edges_at(image, pos2(100.0, 100.0)), Some((-1, -1)));
        assert_eq!(tool.edges_at(image, pos2(305.0, 200.0)), Some((1, 0)));
        assert_eq!(tool.edges_at(image, pos2(200.0, 296.0)), Some((0, 1)));
        assert_eq!(tool.edges_at(image, pos2(200.0, 200.0)), None, "inside moves");
        assert_eq!(tool.edges_at(image, pos2(20.0, 20.0)), None, "outside draws anew");
    }

    #[test]
    fn crop_shape_round_trips_through_settings() {
        let shape = CropShape { aspect: 6, portrait: true };
        assert_eq!(shape.setting(), ("16:10", true));
        assert_eq!(CropShape::from_setting("16:10", true), shape);
        assert_eq!(CropShape::from_setting("no such", false), CropShape::default(), "Manual");
    }

    #[test]
    fn screen_aspect_follows_the_screen_and_turns() {
        let image_px = vec2(300.0, 300.0);
        let screen = ASPECTS.iter().position(|(_, r)| *r == Ratio::Screen).unwrap();
        let mut tool = CropTool { aspect: screen, screen: vec2(1920.0, 1200.0), ..Default::default() };
        assert!((tool.unit_ratio(image_px).unwrap() - 1.6).abs() < 1e-5);
        tool.portrait = true;
        assert!((tool.unit_ratio(image_px).unwrap() - 1.0 / 1.6).abs() < 1e-5);
    }

    fn tint(x: f32, y: f32, feather: f32, rotation: i32) -> Vec<ParamValue> {
        vec![
            ParamValue::Point(x, y),
            ParamValue::Number(feather),
            ParamValue::Number(0.25),
            ParamValue::Color(0xff3a6ea5),
            ParamValue::Rotation(rotation),
        ]
    }

    #[test]
    fn gradient_ends_move_its_point_and_feather() {
        let mut params = tint(0.3, 0.5, 0.25, 0);
        let mut g = Gradient::read(&params).unwrap();
        assert!((g.start - 0.375).abs() < 1e-6 && (g.end - 0.625).abs() < 1e-6);
        g.pull(Grab::End, 0.875);
        g.write(&mut params);
        assert_eq!(params[0], ParamValue::Point(0.3, 0.625), "the middle moved, across stayed");
        assert_eq!(params[1], ParamValue::Number(0.5));

        // From the right: positions run from the right edge.
        let g = Gradient::read(&tint(0.8, 0.4, 0.2, 1)).unwrap();
        assert!((g.start - 0.1).abs() < 1e-6 && (g.end - 0.3).abs() < 1e-6, "{} {}", g.start, g.end);
        assert!(g.point(g.start, g.across).distance(pos2(0.9, 0.4)) < 1e-6);
    }

    #[test]
    fn pulling_the_start_past_the_end_turns_the_tint_around() {
        let mut params = tint(0.5, 0.5, 0.2, 0);
        let mut g = Gradient::read(&params).unwrap();
        g.pull(Grab::Start, 0.8);
        g.write(&mut params);
        // Full tint now from the bottom, down to 0.8 and gone at 0.6.
        assert_eq!(params[4], ParamValue::Rotation(2));
        let g = Gradient::read(&params).unwrap();
        assert!((g.point(g.start, 0.0).y - 0.8).abs() < 1e-5 && (g.point(g.end, 0.0).y - 0.6).abs() < 1e-5);
        // Feather stops at 1.
        let mut g = Gradient::read(&tint(0.5, 0.5, 0.2, 0)).unwrap();
        g.pull(Grab::End, 3.0);
        assert!((g.end - g.start - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rings_turn_into_slider_values_and_back() {
        let spec = effects::spec("radblur").unwrap();
        let params = vec![ParamValue::Point(0.5, 0.5), ParamValue::Number(0.2), ParamValue::Number(0.0)];
        let (index, radius) = ring(spec, &params, 500.0, 1.0).unwrap();
        assert_eq!(index, 1);
        assert!((ring_value(spec, radius, 500.0, 1.0) - 0.2).abs() < 1e-5);
        let zoom = effects::spec("FocalZoom").unwrap();
        let params = effects::defaults(zoom);
        let (index, radius) = ring(zoom, &params, 500.0, 0.5).unwrap();
        assert_eq!((index, radius), (2, 125.0), "250 photo pixels at half size");
        assert!(ring(effects::spec("dir_tint").unwrap(), &params, 500.0, 1.0).is_none());
    }
}
