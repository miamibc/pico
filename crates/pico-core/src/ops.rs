//! Image building blocks shared by the effects: a floating-point RGBA image and
//! the operations Picnik-style effect recipes are made of (blur, blend modes,
//! masks, curves, color matrices, glow, noise).
//!
//! These are Pico's own implementations of well-known techniques; the effect
//! recipes in `picnik.rs` describe how Picnik (Apache-2.0) combined them.

use image::{DynamicImage, RgbaImage};

/// Linear RGBA with channels in 0..=1 (not premultiplied).
#[derive(Clone, Debug, PartialEq)]
pub struct Img {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
}

impl Img {
    pub fn from_dynamic(img: &DynamicImage) -> Self {
        let rgba = img.to_rgba8();
        let px = rgba.pixels().map(|p| p.0.map(|c| c as f32 / 255.0)).collect();
        Self { width: rgba.width() as usize, height: rgba.height() as usize, px }
    }

    pub fn to_dynamic(&self) -> DynamicImage {
        let mut out = RgbaImage::new(self.width as u32, self.height as u32);
        for (dst, src) in out.pixels_mut().zip(&self.px) {
            dst.0 = src.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
        DynamicImage::ImageRgba8(out)
    }

    pub fn filled(width: usize, height: usize, color: [f32; 4]) -> Self {
        Self { width, height, px: vec![color; width * height] }
    }

    pub fn at(&self, x: usize, y: usize) -> [f32; 4] {
        self.px[y * self.width + x]
    }

    pub fn map(&mut self, f: impl Fn([f32; 4]) -> [f32; 4]) {
        for p in &mut self.px {
            *p = f(*p);
        }
    }

    /// Calls `f(x, y, pixel)` for every pixel.
    pub fn map_xy(&mut self, f: impl Fn(usize, usize, [f32; 4]) -> [f32; 4]) {
        let width = self.width;
        for (i, p) in self.px.iter_mut().enumerate() {
            *p = f(i % width, i / width, *p);
        }
    }

    pub fn max_side(&self) -> f32 {
        self.width.max(self.height) as f32
    }
}

pub fn luma(p: [f32; 4]) -> f32 {
    // Picnik's luminance weights for desaturation.
    0.3086 * p[0] + 0.6094 * p[1] + 0.0820 * p[2]
}

pub fn rgb(color: u32) -> [f32; 4] {
    [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff]
        .map(|c| c as f32 / 255.0)
        .into_iter()
        .chain([1.0])
        .collect::<Vec<_>>()
        .try_into()
        .expect("4 channels")
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), lerp(a[3], b[3], t)]
}

// ---------------------------------------------------------------- blur

/// Flash-style blur: `passes` box blurs of width `size_x` by `size_y` pixels
/// (Flash's quality 3 means three passes, which approximates a Gaussian).
/// Pixels outside the image repeat the edge.
pub fn blur(img: &Img, size_x: f32, size_y: f32, passes: usize) -> Img {
    let mut out = img.clone();
    for _ in 0..passes {
        out = box_pass(&out, size_x, true);
        out = box_pass(&out, size_y, false);
    }
    out
}

fn box_pass(img: &Img, size: f32, horizontal: bool) -> Img {
    let radius = ((size - 1.0) / 2.0).max(0.0);
    if radius < 0.5 {
        return img.clone();
    }
    let r = radius.round() as isize;
    let (len, lines) = if horizontal { (img.width, img.height) } else { (img.height, img.width) };
    let idx = |line: usize, i: usize| if horizontal { line * img.width + i } else { i * img.width + line };
    let mut out = img.clone();
    let window = (2 * r + 1) as f32;
    for line in 0..lines {
        let sample = |i: isize| img.px[idx(line, i.clamp(0, len as isize - 1) as usize)];
        let mut sum = [0.0f32; 4];
        for i in -r..=r {
            let p = sample(i);
            for c in 0..4 {
                sum[c] += p[c];
            }
        }
        for i in 0..len {
            out.px[idx(line, i)] = sum.map(|s| s / window);
            let (add, sub) = (sample(i as isize + r + 1), sample(i as isize - r));
            for c in 0..4 {
                sum[c] += add[c] - sub[c];
            }
        }
    }
    out
}

// ---------------------------------------------------------------- blending

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Blend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Add,
    Subtract,
    Difference,
    HardLight,
}

impl Blend {
    /// Flash `BlendMode` numbering, used by recipes that store the mode as a number.
    pub fn from_flash(n: f32) -> Self {
        match n.round() as i32 {
            3 => Blend::Multiply,
            4 => Blend::Screen,
            5 => Blend::Lighten,
            6 => Blend::Darken,
            7 => Blend::Difference,
            8 => Blend::Add,
            9 => Blend::Subtract,
            13 => Blend::Overlay,
            14 => Blend::HardLight,
            _ => Blend::Normal,
        }
    }

    fn channel(self, base: f32, top: f32) -> f32 {
        match self {
            Blend::Normal => top,
            Blend::Multiply => base * top,
            Blend::Screen => 1.0 - (1.0 - base) * (1.0 - top),
            Blend::Overlay => Blend::HardLight.channel(top, base),
            Blend::HardLight => {
                if top < 0.5 {
                    2.0 * base * top
                } else {
                    1.0 - 2.0 * (1.0 - base) * (1.0 - top)
                }
            }
            Blend::Darken => base.min(top),
            Blend::Lighten => base.max(top),
            Blend::Add => (base + top).min(1.0),
            Blend::Subtract => (base - top).max(0.0),
            Blend::Difference => (base - top).abs(),
        }
    }
}

/// Puts `top` over `base` with a blend mode, an overall opacity and an optional
/// per-pixel mask (0 keeps `base`, 1 applies `top` fully). Keeps `base`'s alpha.
pub fn blend(base: &mut Img, top: &Img, mode: Blend, alpha: f32, mask: Option<&[f32]>) {
    for (i, (b, t)) in base.px.iter_mut().zip(&top.px).enumerate() {
        let amount = alpha * t[3] * mask.map_or(1.0, |m| m[i]);
        if amount <= 0.0 {
            continue;
        }
        for c in 0..3 {
            b[c] = lerp(b[c], mode.channel(b[c], t[c]).clamp(0.0, 1.0), amount.min(1.0));
        }
    }
}

/// A circular gradient: `inner` value inside `inner_radius` of the center,
/// `outer` beyond `outer_radius`, with a smooth ramp in between (Picnik's
/// circular gradient mask).
pub struct Ring {
    pub cx: f32,
    pub cy: f32,
    pub inner_radius: f32,
    pub outer_radius: f32,
    pub inner: f32,
    pub outer: f32,
}

pub fn circular_mask(w: usize, h: usize, ring: Ring) -> Vec<f32> {
    let span = (ring.outer_radius - ring.inner_radius).max(1e-3);
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
            let d = ((x - ring.cx).powi(2) + (y - ring.cy).powi(2)).sqrt();
            let t = ((d - ring.inner_radius) / span).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            lerp(ring.inner, ring.outer, t)
        })
        .collect()
}

// ---------------------------------------------------------------- color

/// Applies a 4x5 matrix (rows R, G, B, A; last column is an offset in 0..=255
/// units, as in Flash color matrices).
pub fn color_matrix(img: &mut Img, m: &[[f32; 5]; 4]) {
    img.map(|p| {
        let mut out = [0.0; 4];
        for (row, o) in m.iter().zip(&mut out) {
            *o = (row[0] * p[0] + row[1] * p[1] + row[2] * p[2] + row[3] * p[3] + row[4] / 255.0).clamp(0.0, 1.0);
        }
        out
    });
}

/// Brightness, contrast and saturation in Picnik's -100..=100 slider units,
/// plus a hue rotation in degrees.
#[derive(Default, Clone, Copy)]
pub struct Adjust {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub hue: f32,
}

pub fn adjust(img: &mut Img, a: Adjust) {
    let saturation = 1.0 + a.saturation / 100.0;
    let contrast = if a.contrast >= 0.0 { 1.0 + a.contrast / 50.0 } else { 1.0 + a.contrast / 100.0 };
    let brightness = a.brightness / 100.0 * 0.5;
    let (sin, cos) = a.hue.to_radians().sin_cos();
    img.map(|p| {
        let mut c = [p[0], p[1], p[2]];
        if a.hue != 0.0 {
            c = rotate_hue(c, sin, cos);
        }
        let y = luma(p);
        for v in &mut c {
            *v = y + (*v - y) * saturation;
            *v = (*v - 0.5) * contrast + 0.5 + brightness;
        }
        [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0), p[3]]
    });
}

/// Hue rotation around the gray axis.
fn rotate_hue(c: [f32; 3], sin: f32, cos: f32) -> [f32; 3] {
    let k = 1.0 / 3.0;
    let s = (1.0f32 / 3.0).sqrt();
    let m = |a: f32, b: f32| cos + (1.0 - cos) * k * a + s * sin * b;
    let r = c[0] * m(1.0, 0.0) + c[1] * (k * (1.0 - cos) - s * sin) + c[2] * (k * (1.0 - cos) + s * sin);
    let g = c[0] * (k * (1.0 - cos) + s * sin) + c[1] * m(1.0, 0.0) + c[2] * (k * (1.0 - cos) - s * sin);
    let b = c[0] * (k * (1.0 - cos) - s * sin) + c[1] * (k * (1.0 - cos) + s * sin) + c[2] * m(1.0, 0.0);
    [r, g, b]
}

/// Desaturates as if shot through a colored filter (white means plain B&W).
pub fn black_and_white(img: &mut Img, filter: u32) {
    let f = rgb(filter);
    let w = [0.3086 * f[0], 0.6094 * f[1], 0.0820 * f[2]];
    let sum = (w[0] + w[1] + w[2]).max(1e-6);
    let w = w.map(|v| v / sum);
    img.map(|p| {
        let y = (w[0] * p[0] + w[1] * p[1] + w[2] * p[2]).clamp(0.0, 1.0);
        [y, y, y, p[3]]
    });
}

/// Recolors with `color`, keeping each pixel's luminance.
pub fn tint(img: &mut Img, color: u32) {
    let c = rgb(color);
    let cl = luma(c).max(1e-3);
    img.map(|p| {
        let y = luma(p);
        // Scale the tint to the pixel's brightness; past the tint's own
        // brightness, blend towards white so highlights stay highlights.
        let out = if y <= cl {
            [c[0] * y / cl, c[1] * y / cl, c[2] * y / cl]
        } else {
            let t = (y - cl) / (1.0 - cl).max(1e-3);
            [lerp(c[0], 1.0, t), lerp(c[1], 1.0, t), lerp(c[2], 1.0, t)]
        };
        [out[0], out[1], out[2], p[3]]
    });
}

/// Maps luminance through a list of `(position 0..=1, color)` stops.
pub fn gradient_map(img: &mut Img, stops: &[(f32, [f32; 3])]) {
    img.map(|p| {
        let y = luma(p).clamp(0.0, 1.0);
        let i = stops.iter().position(|(pos, _)| *pos >= y).unwrap_or(stops.len() - 1).max(1);
        let ((p0, c0), (p1, c1)) = (stops[i - 1], stops[i]);
        let t = ((y - p0) / (p1 - p0).max(1e-6)).clamp(0.0, 1.0);
        [lerp(c0[0], c1[0], t), lerp(c0[1], c1[1], t), lerp(c0[2], c1[2], t), p[3]]
    });
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

// ---------------------------------------------------------------- curves

/// A natural cubic spline through `(x, y)` points in 0..=255 units, as used by
/// curves dialogs.
pub struct Spline {
    xs: Vec<f32>,
    ys: Vec<f32>,
    y2: Vec<f32>,
}

impl Spline {
    pub fn new(points: &[(f32, f32)]) -> Self {
        let xs: Vec<f32> = points.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = points.iter().map(|p| p.1).collect();
        let n = xs.len();
        let mut y2 = vec![0.0; n];
        let mut u = vec![0.0; n];
        for i in 1..n.saturating_sub(1) {
            let sig = (xs[i] - xs[i - 1]) / (xs[i + 1] - xs[i - 1]);
            let p = sig * y2[i - 1] + 2.0;
            y2[i] = (sig - 1.0) / p;
            let d = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]) - (ys[i] - ys[i - 1]) / (xs[i] - xs[i - 1]);
            u[i] = (6.0 * d / (xs[i + 1] - xs[i - 1]) - sig * u[i - 1]) / p;
        }
        for k in (0..n.saturating_sub(1)).rev() {
            y2[k] = y2[k] * y2[k + 1] + u[k];
        }
        Self { xs, ys, y2 }
    }

    pub fn at(&self, x: f32) -> f32 {
        let n = self.xs.len();
        if n < 2 {
            return x;
        }
        let x = x.clamp(self.xs[0], self.xs[n - 1]);
        let hi = self.xs.iter().position(|&v| v >= x).unwrap_or(n - 1).max(1);
        let lo = hi - 1;
        let h = self.xs[hi] - self.xs[lo];
        if h <= 0.0 {
            return self.ys[lo];
        }
        let a = (self.xs[hi] - x) / h;
        let b = (x - self.xs[lo]) / h;
        a * self.ys[lo]
            + b * self.ys[hi]
            + ((a.powi(3) - a) * self.y2[lo] + (b.powi(3) - b) * self.y2[hi]) * h * h / 6.0
    }
}

/// Applies a master curve, then optional per-channel curves (points in 0..=255).
pub fn curves(img: &mut Img, master: &[(f32, f32)], channels: [&[(f32, f32)]; 3]) {
    let master = (!master.is_empty()).then(|| Spline::new(master));
    let channels = channels.map(|c| (!c.is_empty()).then(|| Spline::new(c)));
    let mut tables = [[0.0f32; 256]; 3];
    for (c, table) in tables.iter_mut().enumerate() {
        for (i, v) in table.iter_mut().enumerate() {
            let mut y = i as f32;
            if let Some(m) = &master {
                y = m.at(y);
            }
            if let Some(s) = &channels[c] {
                y = s.at(y);
            }
            *v = (y.clamp(0.0, 255.0)).round() / 255.0;
        }
    }
    img.map(|p| {
        let look = |c: usize| tables[c][(p[c].clamp(0.0, 1.0) * 255.0).round() as usize];
        [look(0), look(1), look(2), p[3]]
    });
}

/// Stretches each channel so its darkest and brightest values reach 0 and 1
/// (ignoring the extreme 0.5%), like Picnik's Auto-Fix.
pub fn auto_fix(img: &mut Img) {
    let mut bounds = [(0.0f32, 1.0f32); 3];
    for (c, bound) in bounds.iter_mut().enumerate() {
        let mut hist = [0u32; 256];
        for p in &img.px {
            hist[(p[c].clamp(0.0, 1.0) * 255.0) as usize] += 1;
        }
        let clip = (img.px.len() as f32 * 0.005) as u32;
        let (mut lo, mut hi, mut seen) = (0, 255, 0);
        for (i, n) in hist.iter().enumerate() {
            seen += n;
            if seen > clip {
                lo = i;
                break;
            }
        }
        seen = 0;
        for (i, n) in hist.iter().enumerate().rev() {
            seen += n;
            if seen > clip {
                hi = i;
                break;
            }
        }
        if hi > lo + 8 {
            *bound = (lo as f32 / 255.0, hi as f32 / 255.0);
        }
    }
    img.map(|p| {
        let f = |c: usize| ((p[c] - bounds[c].0) / (bounds[c].1 - bounds[c].0)).clamp(0.0, 1.0);
        [f(0), f(1), f(2), p[3]]
    });
}

// ---------------------------------------------------------------- texture

/// Deterministic pseudo-random noise in `low..=high` (0..=255 units).
pub fn noise(w: usize, h: usize, seed: u32, low: f32, high: f32, gray: bool) -> Img {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(0x9e37_79b9) | 1;
    let mut next = move || {
        // xorshift32
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state as f32 / u32::MAX as f32) * (high - low) / 255.0 + low / 255.0
    };
    let px = (0..w * h)
        .map(|_| {
            if gray {
                let v = next();
                [v, v, v, 1.0]
            } else {
                [next(), next(), next(), 1.0]
            }
        })
        .collect();
    Img { width: w, height: h, px }
}

/// Darkens (or tints) the image towards its edges: Flash's inner glow, which
/// is strongest where the blurred image shape fades out at the borders.
pub fn inner_glow(img: &mut Img, color: u32, blur_x: f32, blur_y: f32, strength: f32, alpha: f32) {
    let (w, h) = (img.width, img.height);
    // Blur a mask that is 1 inside the image and 0 beyond it, padded so the
    // outside actually participates in the blur.
    let pad_x = (blur_x * 1.5).ceil() as usize + 1;
    let pad_y = (blur_y * 1.5).ceil() as usize + 1;
    let (pw, ph) = (w + 2 * pad_x, h + 2 * pad_y);
    let mut mask = Img::filled(pw, ph, [0.0; 4]);
    for y in 0..h {
        for x in 0..w {
            mask.px[(y + pad_y) * pw + x + pad_x] = [1.0; 4];
        }
    }
    let mask = blur(&mask, blur_x, blur_y, 3);
    let c = rgb(color);
    img.map_xy(|x, y, p| {
        let inside = mask.px[(y + pad_y) * pw + x + pad_x][0];
        let glow = ((1.0 - inside) * strength).clamp(0.0, 1.0) * alpha;
        [lerp(p[0], c[0], glow), lerp(p[1], c[1], glow), lerp(p[2], c[2], glow), p[3]]
    });
}

/// Unsharp-style local contrast: adds the difference from a blurred copy.
pub fn local_contrast(img: &mut Img, radius: f32, strength: f32) {
    let blurred = blur(img, radius, radius, 3);
    for (p, b) in img.px.iter_mut().zip(&blurred.px) {
        for c in 0..3 {
            p[c] = (p[c] + (p[c] - b[c]) * strength).clamp(0.0, 1.0);
        }
    }
}

/// Averages `size`-pixel blocks.
pub fn pixelate(img: &mut Img, size: usize) {
    let size = size.max(1);
    let (w, h) = (img.width, img.height);
    for by in (0..h).step_by(size) {
        for bx in (0..w).step_by(size) {
            let (x1, y1) = ((bx + size).min(w), (by + size).min(h));
            let mut sum = [0.0f32; 4];
            for y in by..y1 {
                for x in bx..x1 {
                    let p = img.px[y * w + x];
                    for c in 0..4 {
                        sum[c] += p[c];
                    }
                }
            }
            let n = ((x1 - bx) * (y1 - by)) as f32;
            let avg = sum.map(|s| s / n);
            for y in by..y1 {
                for x in bx..x1 {
                    img.px[y * w + x] = avg;
                }
            }
        }
    }
}

/// Reduces each channel to `steps` levels.
pub fn posterize(img: &mut Img, steps: f32) {
    let steps = steps.max(2.0) - 1.0;
    img.map(|p| {
        let q = |v: f32| (v * steps).round() / steps;
        [q(p[0]), q(p[1]), q(p[2]), p[3]]
    });
}

/// Paints a border of `thickness` pixels around the edges, inside the image,
/// optionally with rounded corners of `radius` pixels in `color`.
pub fn frame(img: &mut Img, color: u32, thickness: f32, corner_radius: f32) {
    let c = rgb(color);
    let (w, h) = (img.width as f32, img.height as f32);
    img.map_xy(|x, y, p| {
        let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
        let edge = x.min(y).min(w - x).min(h - y);
        let mut cover: f32 = if edge < thickness { 1.0 } else { 0.0 };
        if corner_radius > 0.0 {
            // Corners can be at most half the shorter side.
            let r = (corner_radius + thickness).min(w / 2.0).min(h / 2.0);
            let corner_radius = r - thickness;
            let (cx, cy) = (x.clamp(r, w - r), y.clamp(r, h - r));
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
            cover = cover.max((d - corner_radius + 0.5).clamp(0.0, 1.0));
        }
        mix(p, c, cover)
    });
}

/// Grows the canvas by the given margins, filling them with `color`.
pub fn expand(img: &Img, left: usize, top: usize, right: usize, bottom: usize, color: u32) -> Img {
    let (w, h) = (img.width + left + right, img.height + top + bottom);
    let mut out = Img::filled(w, h, rgb(color));
    for y in 0..img.height {
        let src = &img.px[y * img.width..(y + 1) * img.width];
        out.px[(y + top) * w + left..(y + top) * w + left + img.width].copy_from_slice(src);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_keeps_flat_images_flat_and_softens_edges() {
        let flat = Img::filled(20, 10, [0.4, 0.5, 0.6, 1.0]);
        let out = blur(&flat, 5.0, 5.0, 3);
        assert!(out.px.iter().all(|p| (p[0] - 0.4).abs() < 1e-5));

        let mut step = Img::filled(20, 1, [0.0, 0.0, 0.0, 1.0]);
        for x in 10..20 {
            step.px[x] = [1.0; 4];
        }
        let out = blur(&step, 5.0, 1.0, 3);
        assert!(out.at(9, 0)[0] > 0.1 && out.at(10, 0)[0] < 0.9, "edge is softened");
        assert!(out.at(0, 0)[0] < 1e-5 && out.at(19, 0)[0] > 1.0 - 1e-5, "far pixels stay put");
    }

    #[test]
    fn spline_passes_through_points() {
        let s = Spline::new(&[(0.0, 0.0), (29.0, 19.0), (110.0, 150.0), (255.0, 255.0)]);
        for (x, y) in [(0.0, 0.0), (29.0, 19.0), (110.0, 150.0), (255.0, 255.0)] {
            assert!((s.at(x) - y).abs() < 1e-3, "{x} -> {}", s.at(x));
        }
        let invert = Spline::new(&[(0.0, 255.0), (255.0, 0.0)]);
        assert!((invert.at(100.0) - 155.0).abs() < 1e-3);
    }

    #[test]
    fn blend_modes() {
        let mut base = Img::filled(1, 1, [0.5, 0.5, 0.5, 1.0]);
        blend(&mut base, &Img::filled(1, 1, [0.5, 0.5, 0.5, 1.0]), Blend::Multiply, 1.0, None);
        assert!((base.px[0][0] - 0.25).abs() < 1e-6);
        blend(&mut base, &Img::filled(1, 1, [1.0; 4]), Blend::Screen, 0.5, None);
        assert!((base.px[0][0] - 0.625).abs() < 1e-6, "half-strength screen to white");
    }

    #[test]
    fn inner_glow_darkens_edges_not_center() {
        let mut img = Img::filled(100, 60, [1.0; 4]);
        inner_glow(&mut img, 0x000000, 20.0, 20.0, 1.4, 1.0);
        assert!(img.at(0, 0)[0] < 0.6, "corner {}", img.at(0, 0)[0]);
        assert!(img.at(50, 30)[0] > 0.95, "center {}", img.at(50, 30)[0]);
    }

    #[test]
    fn tint_keeps_luminance_order() {
        let mut img = Img { width: 2, height: 1, px: vec![[0.2, 0.2, 0.2, 1.0], [0.8, 0.8, 0.8, 1.0]] };
        tint(&mut img, 0x80cfff);
        assert!(luma(img.px[0]) < luma(img.px[1]));
        assert!(img.px[0][2] > img.px[0][0], "dark pixel is bluish");
    }
}
