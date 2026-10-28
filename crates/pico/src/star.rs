//! The star that flies onto a photo when it is starred, as in Picasa: it
//! leaves the Star button spinning, unfolds face-on over the middle of the
//! photo, then settles into the star badge in the corner.
//!
//! It strikes the middle with sparks and a puff of smoke. Unstarring makes
//! the badge vanish in a puff of smoke.
//!
//! The star is a faceted solid (two cushions on a five-pointed outline with rounded points)
//! lit from the upper left, with a soft shadow beneath so it shows on light
//! and dark photos alike.

use eframe::egui::{self, Color32, Pos2, Vec2};
use std::f32::consts::{PI, TAU};

/// Seconds from the button to the badge.
const DURATION: f64 = 0.7;
/// Seconds the strike's sparks and smoke outlast the flight.
const AFTER: f64 = 0.3;
/// Seconds a star takes to vanish in smoke.
const PUFF: f64 = 0.75;
/// Radius of the badge's star, for `Puff`.
const BADGE: f32 = 10.0;
/// Phases, as fractions of the flight: arrive in the middle, stay, leave.
const ARRIVE: f32 = 0.38;
const LEAVE: f32 = 0.72;
/// Inner corners' distance from the centre, outer points being 1.
const INNER: f32 = 0.45;
/// Height of each side's cushion over the outline.
const DEPTH: f32 = 0.3;
/// Rings of vertices across each cushion.
const RINGS: usize = 8;
/// How far along the edges the points and inner corners are rounded off.
const ROUND_POINT: f32 = 0.08;
const ROUND_INNER: f32 = 0.05;
/// Vertices on each rounded corner.
const ROUND_STEPS: usize = 6;
/// Camera distance, in star radii, for the perspective.
const CAMERA: f32 = 5.0;

pub struct Flight {
    start: f64,
    from: Pos2,
}

impl Flight {
    pub fn new(start: f64, from: Pos2) -> Flight {
        Flight { start, from }
    }

    /// Whether the star has reached the badge (its smoke may still linger).
    pub fn landed(&self, now: f64) -> bool {
        now - self.start >= DURATION
    }

    /// Paints the star at `now`, flying over `area` to the badge whose centre
    /// is `badge`; false once the last smoke has cleared.
    pub fn paint(&self, painter: &egui::Painter, now: f64, area: egui::Rect, badge: Pos2) -> bool {
        let seconds = now - self.start;
        if !(0.0..DURATION + AFTER).contains(&seconds) {
            return false;
        }
        let t = (seconds / DURATION) as f32;
        let big = (area.width().min(area.height()) * 0.22).clamp(40.0, 220.0);
        let small = 12.0;
        // Seconds since the star struck the screen.
        let strike = (seconds - (ARRIVE as f64) * DURATION) as f32;
        // How much bigger it is the moment it strikes, squashed flat on the glass.
        const SQUASH: f32 = 0.14;
        let (centre, radius, height) = if t < ARRIVE {
            let k = t / ARRIVE;
            let e = ease_out(k);
            // A slight arc, so it is thrown rather than slid.
            let lift = Vec2::new(0.0, -0.25 * big * (PI * k).sin());
            // Speeding up towards the viewer until it hits the screen.
            (self.from.lerp(area.center(), e) + lift, small + (big * (1.0 + SQUASH) - small) * k * k, e)
        } else if t < LEAVE {
            // Bounces back off the glass and settles.
            let recoil = SQUASH * (-12.0 * strike).exp() * (35.0 * strike).cos();
            (area.center(), big * (1.0 + recoil), 1.0)
        } else {
            let k = ease_in_out((t - LEAVE) / (1.0 - LEAVE));
            (area.center().lerp(badge, k), big + (BADGE - big) * k, 1.0 - k)
        };
        // Spins in from edge-on and strikes the screen face-on, then wobbles.
        let spin = (t / ARRIVE).min(1.0);
        let yaw = (PI * 2.5) * (1.0 - ease_out(spin));
        let wobble = if strike > 0.0 { 0.25 * (-9.0 * strike).exp() * (28.0 * strike).sin() } else { 0.0 };
        let tilt = 0.35 * (1.0 - spin) + wobble;
        let fade = (t / 0.06).min(1.0) * ((1.0 - t) / 0.05).clamp(0.0, 1.0);

        // The strike in the middle: smoke behind the star, sparks in front.
        smoke(painter, area.center(), big * 1.1, strike / 0.7, 1);
        if fade > 0.0 {
            paint_star(painter, centre, radius, yaw, tilt, height, fade);
        }
        sparks(painter, area.center(), big, strike);
        true
    }
}

/// The badge's star vanishing in smoke when a photo is unstarred.
pub struct Puff {
    start: f64,
}

impl Puff {
    pub fn new(start: f64) -> Puff {
        Puff { start }
    }

    /// Paints the puff at `now` over the badge centred on `at`; false once
    /// the smoke has cleared.
    pub fn paint(&self, painter: &egui::Painter, now: f64, at: Pos2) -> bool {
        let k = ((now - self.start) / PUFF) as f32;
        if !(0.0..1.0).contains(&k) {
            return false;
        }
        // The star swells a little and fades while the smoke covers it.
        let fade = (1.0 - k / 0.35).max(0.0);
        if fade > 0.0 {
            paint_star(painter, at, BADGE * (1.0 + 0.5 * k), 0.0, 0.0, 0.0, fade);
        }
        smoke(painter, at, 42.0, k, 2);
        true
    }
}

fn ease_out(k: f32) -> f32 {
    1.0 - (1.0 - k).powi(3)
}

fn ease_in_out(k: f32) -> f32 {
    if k < 0.5 { 4.0 * k * k * k } else { 1.0 - (-2.0 * k + 2.0).powi(3) / 2.0 }
}

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normalize(a: V3) -> V3 {
    let n = dot(a, a).sqrt().max(1e-6);
    [a[0] / n, a[1] / n, a[2] / n]
}

/// The star turned by `yaw` about the vertical and `tilt` about the
/// horizontal, `height` (0 on the photo, 1 high above it) setting the shadow.
fn paint_star(painter: &egui::Painter, centre: Pos2, radius: f32, yaw: f32, tilt: f32, height: f32, fade: f32) {
    let turn = |[x, y, z]: V3| {
        let (x, z) = (x * yaw.cos() + z * yaw.sin(), z * yaw.cos() - x * yaw.sin());
        let (y, z) = (y * tilt.cos() - z * tilt.sin(), z * tilt.cos() + y * tilt.sin());
        [x, y, z]
    };
    // Model y points down, as on screen; z towards the viewer.
    let project = |p: V3| centre + Vec2::new(p[0], p[1]) * radius * CAMERA / (CAMERA - p[2]);
    let outline = rounded_outline();
    let n = outline.len();
    let rim: Vec<V3> = outline.iter().map(|p| turn([p.x, p.y, 0.0])).collect();

    // Shadow: the outline dropped onto the photo below and to the right,
    // further and softer the higher the star is.
    let offset = Vec2::new(0.06, 0.1) * radius * (0.4 + 1.6 * height);
    let dropped: Vec<Pos2> = rim.iter().map(|&p| project(p) + offset).collect();
    let shadow_centre = project([0.0, 0.0, 0.0]) + offset;
    for (grow, alpha) in [(1.16, 0.06), (1.1, 0.08), (1.05, 0.1), (1.0, 0.14)] {
        let alpha = alpha * fade * (1.0 - 0.4 * height);
        let points = dropped.iter().map(|&p| shadow_centre + (p - shadow_centre) * grow);
        fan(painter, shadow_centre, points, Color32::from_black_alpha((alpha * 255.0) as u8));
    }

    // Each side is a cushion over the outline, z = DEPTH * sqrt(1 - s²) at
    // s of the way out, lit per corner so it shades smoothly.
    let light = normalize([-0.45, -0.6, 0.65]);
    let half = normalize([light[0], light[1], light[2] + 1.0]);
    let alpha = (fade * 255.0) as u8;
    let mut mesh = egui::Mesh::default();
    // Per side: rings from the middle out, each with a vertex per outline point.
    let mut points: Vec<V3> = Vec::new();
    let mut normals: Vec<V3> = Vec::new();
    for side in [1.0, -1.0] {
        for ring in 0..=RINGS {
            let s = ring as f32 / RINGS as f32;
            let rise = (1.0 - s * s).sqrt();
            for j in 0..n {
                let p = outline[j];
                let along = outline[(j + 1) % n] - outline[(j + n - 1) % n];
                // Across the cushion, scaled by `rise` so it stays finite at the rim.
                let across = [p.x * rise, p.y * rise, -side * DEPTH * s];
                let mut normal = normalize(cross([along.x, along.y, 0.0], across));
                if dot(normal, [p.x, p.y, side * DEPTH]) < 0.0 {
                    normal = normal.map(|v| -v);
                }
                points.push(turn([p.x * s, p.y * s, side * DEPTH * rise]));
                normals.push(turn(normal));
            }
        }
    }
    for (point, normal) in points.iter().zip(&normals) {
        let diffuse = dot(*normal, light).max(0.0);
        let shine = dot(*normal, half).max(0.0).powi(30);
        let shade = |base: f32| ((base * (0.38 + 0.72 * diffuse) + 255.0 * 0.85 * shine).min(255.0)) as u8;
        let color = Color32::from_rgba_unmultiplied(shade(255.0), shade(190.0), shade(30.0), alpha);
        mesh.colored_vertex(project(*point), color);
    }
    // Triangles facing the camera, far ones first.
    let mut triangles: Vec<(f32, [u32; 3])> = Vec::new();
    for side in 0..2 {
        for ring in 0..RINGS {
            for j in 0..n {
                let at = |r: usize, k: usize| ((side * (RINGS + 1) + r) * n + k % n) as u32;
                let quad = [at(ring, j), at(ring + 1, j), at(ring + 1, j + 1), at(ring, j + 1)];
                for tri in [[quad[0], quad[1], quad[2]], [quad[0], quad[2], quad[3]]] {
                    let corners = tri.map(|i| points[i as usize]);
                    let middle = [0, 1, 2].map(|axis| corners.iter().map(|c| c[axis]).sum::<f32>() / 3.0);
                    let facing = [0, 1, 2].map(|axis| tri.iter().map(|&i| normals[i as usize][axis]).sum::<f32>());
                    if dot(facing, sub([0.0, 0.0, CAMERA], middle)) > 0.0 {
                        triangles.push((middle[2], tri));
                    }
                }
            }
        }
    }
    triangles.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, [a, b, c]) in triangles {
        mesh.add_triangle(a, b, c);
    }
    painter.add(egui::Shape::mesh(mesh));
    // A dark edge keeps the outline crisp on yellow and white photos.
    let edge: Vec<Pos2> = rim.iter().map(|&p| project(p)).collect();
    let stroke = egui::Stroke::new(1.0, Color32::from_rgba_unmultiplied(120, 70, 0, (fade * 160.0) as u8));
    painter.add(egui::Shape::closed_line(edge, stroke));
}

/// The five-pointed outline with its points and inner corners rounded off.
fn rounded_outline() -> Vec<Pos2> {
    let corners: Vec<Pos2> = (0..10)
        .map(|i| {
            let angle = -PI / 2.0 + i as f32 * PI / 5.0;
            let r = if i % 2 == 0 { 1.0 } else { INNER };
            Pos2::new(r * angle.cos(), r * angle.sin())
        })
        .collect();
    let mut out = Vec::new();
    for i in 0..10 {
        let (prev, corner, next) = (corners[(i + 9) % 10], corners[i], corners[(i + 1) % 10]);
        let cut = if i % 2 == 0 { ROUND_POINT } else { ROUND_INNER };
        let a = corner + (prev - corner).normalized() * cut;
        let b = corner + (next - corner).normalized() * cut;
        // A quadratic curve from edge to edge, bent by the corner.
        for step in 0..ROUND_STEPS {
            let t = step as f32 / ROUND_STEPS as f32;
            let p = a.to_vec2() * (1.0 - t) * (1.0 - t) + corner.to_vec2() * 2.0 * t * (1.0 - t) + b.to_vec2() * t * t;
            out.push(p.to_pos2());
        }
    }
    out
}

/// A repeatable pseudo-random number in 0..1 for particle `i`.
fn noise(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

/// Sparks thrown out `age` seconds ago from a star of `size` hitting `centre`.
fn sparks(painter: &egui::Painter, centre: Pos2, size: f32, age: f32) {
    const LIFE: f32 = 0.7;
    // Air drag, per second: the sparks start fast and soon hang in the air.
    const DRAG: f32 = 4.5;
    if !(0.0..LIFE).contains(&age) {
        return;
    }
    let gravity = 450.0 * size / 120.0;
    for i in 0..36 {
        let life = LIFE * (0.55 + 0.45 * noise(i, 1));
        if age >= life {
            continue;
        }
        let angle = TAU * (i as f32 + noise(i, 2)) / 36.0;
        let speed = size * (3.0 + 4.0 * noise(i, 3));
        let dir = Vec2::angled(angle);
        // Out from the star's edge, slowing and falling.
        let at = |a: f32| {
            centre + dir * (size * 0.55 + speed * (1.0 - (-DRAG * a).exp()) / DRAG) + Vec2::Y * gravity * a * a
        };
        let head = at(age);
        let tail = at((age - 0.045).max(0.0));
        let k = age / life;
        // White hot, then yellow, then orange as it dies.
        let color = Color32::from_rgba_unmultiplied(
            255,
            (250.0 - 120.0 * k) as u8,
            (220.0 * (1.0 - k).powi(3)) as u8,
            ((1.0 - k) * 255.0) as u8,
        );
        painter.line_segment([tail, head], egui::Stroke::new(2.2 * (1.0 - 0.6 * k), color));
        painter.circle_filled(head, 1.6 * (1.0 - k) + 0.4, color);
    }
}

/// A puff of smoke `k` (0 to 1) through its life, about `size` across.
fn smoke(painter: &egui::Painter, centre: Pos2, size: f32, k: f32, salt: u32) {
    if !(0.0..1.0).contains(&k) {
        return;
    }
    let grow = 1.0 - (1.0 - k).powi(2);
    // Gray reads on dark and light photos alike.
    let alpha = 0.55 * (1.0 - k).powi(2) * (k / 0.08).min(1.0);
    for i in 0..14 {
        let angle = TAU * (i as f32 + noise(i, salt * 7)) / 14.0;
        let reach = size * (0.25 + 0.6 * noise(i, salt * 7 + 1)) * grow;
        // Puffs drift outwards and rise.
        let at = centre + Vec2::angled(angle) * reach - Vec2::Y * size * 0.35 * k;
        let radius = size * (0.18 + 0.14 * noise(i, salt * 7 + 2)) * (0.5 + 0.9 * grow);
        let gray = (150.0 + 60.0 * noise(i, salt * 7 + 3)) as u8;
        for (r, a) in [(1.0, 0.35), (0.7, 0.45), (0.4, 0.5)] {
            let a = (alpha * a * 255.0) as u8;
            painter.circle_filled(at, radius * r, Color32::from_rgba_unmultiplied(gray, gray, gray, a));
        }
    }
}

/// Fills a polygon that is star-shaped around `centre`.
fn fan(painter: &egui::Painter, centre: Pos2, points: impl Iterator<Item = Pos2>, color: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, color);
    for point in points {
        mesh.colored_vertex(point, color);
    }
    let n = mesh.vertices.len() as u32 - 1;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    painter.add(egui::Shape::mesh(mesh));
}
