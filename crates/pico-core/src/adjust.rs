//! Pixel implementations of Picasa's edit filters.
//!
//! Parameter meanings and ranges come from Picasa's `filterdesc.xml` (see
//! `docs/research/picasa-filter-layout.tsv`); the algorithms are Pico's own,
//! aiming for the same intent rather than bit-identical output. Filters Pico
//! does not implement yet return the image unchanged, and stay in the INI.

use crate::effects;
use crate::filters::Filter;
use crate::ops::Img;
use image::{DynamicImage, RgbaImage};

/// Default values from `filterdesc.xml`, used when a filter omits parameters
/// (Picasa writes e.g. a bare `unsharp=1;` from its menu command).
const UNSHARP2_DEFAULT: f32 = 0.6;
const SAT_DEFAULT: f32 = 0.1618;

/// Straighten (`tilt`) stores the slider position in -1..=1. Picasa's angle
/// scale is not confirmed; this maximum matches the tool's feel.
pub const MAX_TILT_DEGREES: f32 = 12.0;

/// Fraction of pixels ignored at each end when auto-levelling, so a few hot
/// pixels do not cancel the stretch.
const AUTO_CLIP: f32 = 0.005;

/// Whether Pico knows how to render this filter id.
pub fn is_supported(id: &str) -> bool {
    effects::spec(id).is_some_and(|s| s.id != "ReanimatedEyeColor")
        || matches!(
            id,
            "crop64"
                | "finetune"
                | "finetune2"
                | "fill"
                | "autolight"
                | "autocontrast"
                | "autocolor"
                | "enhance"
                | "bw"
                | "sepia"
                | "warm"
                | "sat"
                | "unsharp"
                | "unsharp2"
                | "tilt"
        )
}

/// Applies one edit. `scale` is the rendered size relative to the original
/// photo, so pixel-sized parameters look the same on previews.
pub fn apply(img: DynamicImage, filter: &Filter, scale: f32) -> DynamicImage {
    if !filter.enabled() {
        return img;
    }
    let value = |i: usize, default: f32| filter.float(i).unwrap_or(default);
    match filter.id.as_str() {
        "crop64" => crop(img, filter),
        "finetune" | "finetune2" => {
            let neutral = filter.color(3).filter(|&c| c & 0x00ff_ffff != 0);
            map_pixels(img, tuning(value(0, 0.0), value(1, 0.0), value(2, 0.0), value(4, 0.0), neutral))
        }
        "fill" => map_pixels(img, tuning(value(0, 0.0), 0.0, 0.0, 0.0, None)),
        "autolight" | "autocontrast" => auto_levels(img, false),
        "autocolor" => auto_levels(img, true),
        "enhance" => auto_levels(auto_levels(img, true), false),
        "bw" => map_rgb(img, |[r, g, b]| {
            let y = luma(r, g, b);
            [y, y, y]
        }),
        "sepia" => map_rgb(img, |[r, g, b]| {
            let y = luma(r, g, b);
            [y * 1.07 + 0.03, y * 0.95 + 0.01, y * 0.78]
        }),
        "warm" => map_rgb(img, |[r, g, b]| [r * 1.06 + 0.01, g * 1.01, b * 0.90]),
        "sat" => {
            let amount = 1.0 + value(0, SAT_DEFAULT);
            map_rgb(img, |[r, g, b]| {
                let y = luma(r, g, b);
                [y + (r - y) * amount, y + (g - y) * amount, y + (b - y) * amount]
            })
        }
        "unsharp" | "unsharp2" => sharpen(img, value(0, UNSHARP2_DEFAULT)),
        "tilt" => straighten(img, value(0, 0.0) * MAX_TILT_DEGREES),
        id => match effects::spec(id) {
            Some(spec) => {
                let params = effects::read_params(spec, filter);
                effects::render(spec, Img::from_dynamic(&img), &params, scale).to_dynamic()
            }
            None => img,
        },
    }
}

fn crop(img: DynamicImage, filter: &Filter) -> DynamicImage {
    let Some(rect) = filter.crop() else { return img };
    let [left, top, right, bottom] = rect.to_unit();
    let (w, h) = (img.width() as f32, img.height() as f32);
    let x = (left * w).round() as u32;
    let y = (top * h).round() as u32;
    let cw = ((right - left) * w).round() as u32;
    let ch = ((bottom - top) * h).round() as u32;
    if cw == 0 || ch == 0 || x + cw > img.width() || y + ch > img.height() {
        return img;
    }
    img.crop_imm(x, y, cw, ch)
}

fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

/// Per-channel tone curves as lookup tables: `[red, green, blue]`.
type Curves = [[u8; 256]; 3];

/// The "Tuning" tab: fill light lifts shadows, the highlights and shadows
/// sliders move the white and black points, color temperature shifts red
/// against blue, and a picked neutral color is balanced to gray.
fn tuning(fill: f32, highlights: f32, shadows: f32, temperature: f32, neutral: Option<u32>) -> Curves {
    let white = (1.0 - highlights.clamp(0.0, 0.48)).max(1e-3);
    let black = shadows.clamp(0.0, 0.48);
    let fill = fill.clamp(0.0, 1.0);
    let mut gains = [1.0 + 0.25 * temperature, 1.0, 1.0 - 0.25 * temperature];
    if let Some(color) = neutral {
        let rgb = [(color >> 16) & 0xff, (color >> 8) & 0xff, color & 0xff].map(|c| (c as f32).max(1.0));
        let gray = (rgb[0] + rgb[1] + rgb[2]) / 3.0;
        for (gain, c) in gains.iter_mut().zip(rgb) {
            *gain *= gray / c;
        }
    }
    curves(|channel, x| {
        let x = (x * gains[channel]).clamp(0.0, 1.0);
        // Fill light: a gamma-like lift that leaves white where it is.
        let x = 1.0 - (1.0 - x).powf(1.0 + 1.5 * fill);
        ((x - black) / (white - black).max(1e-3)).clamp(0.0, 1.0)
    })
}

fn curves(f: impl Fn(usize, f32) -> f32) -> Curves {
    let mut out = [[0u8; 256]; 3];
    for (channel, table) in out.iter_mut().enumerate() {
        for (i, v) in table.iter_mut().enumerate() {
            *v = to_u8(f(channel, i as f32 / 255.0));
        }
    }
    out
}

fn map_pixels(img: DynamicImage, curves: Curves) -> DynamicImage {
    let mut rgba = img.into_rgba8();
    for p in rgba.pixels_mut() {
        for c in 0..3 {
            p.0[c] = curves[c][p.0[c] as usize];
        }
    }
    DynamicImage::ImageRgba8(rgba)
}

fn map_rgb(img: DynamicImage, f: impl Fn([f32; 3]) -> [f32; 3]) -> DynamicImage {
    let mut rgba = img.into_rgba8();
    for p in rgba.pixels_mut() {
        let rgb = f([p.0[0], p.0[1], p.0[2]].map(|c| c as f32 / 255.0));
        for (channel, value) in p.0.iter_mut().zip(rgb) {
            *channel = to_u8(value);
        }
    }
    DynamicImage::ImageRgba8(rgba)
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Stretches levels so the darkest and brightest pixels reach black and white.
/// Per channel, this also removes color casts (Auto Color); on luminance it only
/// fixes contrast (Auto Contrast).
fn auto_levels(img: DynamicImage, per_channel: bool) -> DynamicImage {
    let rgba = img.into_rgba8();
    let mut histograms = [[0u32; 256]; 3];
    for p in rgba.pixels() {
        if per_channel {
            for c in 0..3 {
                histograms[c][p.0[c] as usize] += 1;
            }
        } else {
            let y = to_u8(luma(p.0[0] as f32, p.0[1] as f32, p.0[2] as f32) / 255.0);
            histograms[0][y as usize] += 1;
        }
    }
    if !per_channel {
        histograms[1] = histograms[0];
        histograms[2] = histograms[0];
    }
    let bounds = histograms.map(|h| percentile_bounds(&h));
    let curves = curves(|channel, x| {
        let (low, high) = bounds[channel];
        (x - low) / (high - low)
    });
    map_pixels(DynamicImage::ImageRgba8(rgba), curves)
}

/// Returns the clipped (low, high) levels as fractions, or (0, 1) when the
/// channel is too flat to stretch meaningfully.
fn percentile_bounds(histogram: &[u32; 256]) -> (f32, f32) {
    let total: u32 = histogram.iter().sum();
    let clip = (total as f32 * AUTO_CLIP) as u32;
    let mut seen = 0;
    let low = histogram.iter().position(|&n| {
        seen += n;
        seen > clip
    });
    seen = 0;
    let high = histogram.iter().rposition(|&n| {
        seen += n;
        seen > clip
    });
    match (low, high) {
        (Some(low), Some(high)) if high > low + 8 => (low as f32 / 255.0, high as f32 / 255.0),
        _ => (0.0, 1.0),
    }
}

/// Rotates by `degrees` (clockwise) and zooms in just enough that no empty
/// corners show, keeping the original size, like Picasa's Straighten.
pub fn straighten(img: DynamicImage, degrees: f32) -> DynamicImage {
    if degrees.abs() < 0.01 {
        return img;
    }
    let src = img.into_rgba8();
    let (w, h) = (src.width() as f32, src.height() as f32);
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (sin_abs, cos_abs) = (sin.abs(), cos.abs());
    // The exact fit puts the corners on the border; 1% more keeps them clear of
    // the outermost pixels, which bilinear sampling would blend in.
    let zoom = (cos_abs + h / w * sin_abs).max(cos_abs + w / h * sin_abs) * 1.01;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let mut out = RgbaImage::new(src.width(), src.height());
    for (x, y, p) in out.enumerate_pixels_mut() {
        let (dx, dy) = ((x as f32 + 0.5 - cx) / zoom, (y as f32 + 0.5 - cy) / zoom);
        // Inverse rotation: where in the source this output pixel comes from.
        let sx = cos * dx + sin * dy + cx - 0.5;
        let sy = -sin * dx + cos * dy + cy - 0.5;
        *p = bilinear(&src, sx, sy);
    }
    DynamicImage::ImageRgba8(out)
}

fn bilinear(img: &RgbaImage, x: f32, y: f32) -> image::Rgba<u8> {
    let max_x = img.width() as f32 - 1.0;
    let max_y = img.height() as f32 - 1.0;
    let (x, y) = (x.clamp(0.0, max_x), y.clamp(0.0, max_y));
    let (x0, y0) = (x.floor(), y.floor());
    let (x1, y1) = ((x0 + 1.0).min(max_x), (y0 + 1.0).min(max_y));
    let (fx, fy) = (x - x0, y - y0);
    let at = |x: f32, y: f32| img.get_pixel(x as u32, y as u32).0.map(|c| c as f32);
    let (a, b, c, d) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
    let mut out = [0u8; 4];
    for i in 0..4 {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (d[i] - c[i]) * fx;
        out[i] = (top + (bottom - top) * fy).round() as u8;
    }
    image::Rgba(out)
}

/// Unsharp mask: adds back the difference between the image and its blur.
fn sharpen(img: DynamicImage, amount: f32) -> DynamicImage {
    let original: RgbaImage = img.into_rgba8();
    let blurred = image::imageops::blur(&original, 1.2);
    let mut out = original.clone();
    for (o, b) in out.pixels_mut().zip(blurred.pixels()) {
        for c in 0..3 {
            let v = o.0[c] as f32 + amount * (o.0[c] as f32 - b.0[c] as f32);
            o.0[c] = v.clamp(0.0, 255.0).round() as u8;
        }
    }
    DynamicImage::ImageRgba8(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterStack;
    use image::Rgba;

    fn one(id_and_params: &str) -> Filter {
        FilterStack::parse(id_and_params).0.remove(0)
    }

    fn solid(rgb: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([rgb[0], rgb[1], rgb[2], 255])))
    }

    fn pixel(img: &DynamicImage) -> [u8; 3] {
        let p = img.to_rgba8().get_pixel(0, 0).0;
        [p[0], p[1], p[2]]
    }

    #[test]
    fn black_and_white_removes_color() {
        let [r, g, b] = pixel(&apply(solid([200, 50, 10]), &one("bw=1;"), 1.0));
        assert!(r == g && g == b);
    }

    #[test]
    fn negative_saturation_desaturates_fully() {
        let [r, g, b] = pixel(&apply(solid([200, 50, 10]), &one("sat=1,-1.000000;"), 1.0));
        assert!(r.abs_diff(g) <= 1 && g.abs_diff(b) <= 1);
    }

    #[test]
    fn warmify_shifts_toward_red() {
        let [r, _, b] = pixel(&apply(solid([128, 128, 128]), &one("warm=1;"), 1.0));
        assert!(r > 128 && b < 128);
    }

    #[test]
    fn tuning_sliders_move_black_point_and_temperature() {
        let darker = pixel(&apply(solid([60, 60, 60]), &one("finetune2=1,0,0,0.2,00000000,0;"), 1.0));
        assert!(darker[0] < 60, "shadows slider raises the black point: {darker:?}");
        let lifted = pixel(&apply(solid([60, 60, 60]), &one("fill=1,0.5;"), 1.0));
        assert!(lifted[0] > 60, "fill light brightens shadows: {lifted:?}");
        let warmer = pixel(&apply(solid([128, 128, 128]), &one("finetune2=1,0,0,0,00000000,0.5;"), 1.0));
        assert!(warmer[0] > warmer[2], "positive temperature is warmer: {warmer:?}");
    }

    #[test]
    fn neutral_picker_balances_cast() {
        // The picked "gray" is bluish, so blue must come down relative to red.
        let out = pixel(&apply(solid([120, 128, 150]), &one("finetune2=1,0,0,0,ff788096,0;"), 1.0));
        assert!(out[0].abs_diff(out[2]) <= 2, "{out:?}");
    }

    #[test]
    fn auto_contrast_stretches_dull_images() {
        let mut img = RgbaImage::new(10, 1);
        for (x, p) in img.pixels_mut().enumerate() {
            let v = 100 + x as u8 * 5;
            *p = Rgba([v, v, v, 255]);
        }
        let out = apply(DynamicImage::ImageRgba8(img), &one("autolight=1;"), 1.0).to_rgba8();
        assert_eq!(out.get_pixel(0, 0).0[0], 0);
        assert_eq!(out.get_pixel(9, 0).0[0], 255);
    }

    #[test]
    fn straighten_keeps_size_and_fills_corners() {
        // A white image with a black frame: after rotating, the zoom must push the
        // frame and the empty area outside the image beyond the corners.
        let mut img = RgbaImage::from_pixel(300, 200, Rgba([255, 255, 255, 255]));
        for (x, y, p) in img.enumerate_pixels_mut() {
            if x == 0 || y == 0 || x == 299 || y == 199 {
                *p = Rgba([0, 0, 0, 255]);
            }
        }
        let out = apply(DynamicImage::ImageRgba8(img), &one("tilt=1,0.691700,0.000000;"), 1.0).to_rgba8();
        assert_eq!(out.dimensions(), (300, 200));
        for (x, y) in [(0, 0), (299, 0), (0, 199), (299, 199), (150, 100)] {
            assert!(out.get_pixel(x, y).0[0] > 200, "pixel {x},{y} is {:?}", out.get_pixel(x, y));
        }
    }

    #[test]
    fn unknown_and_disabled_filters_change_nothing() {
        let img = solid([1, 2, 3]);
        assert_eq!(pixel(&apply(img.clone(), &one("FutureFilter=1,1,2;"), 1.0)), [1, 2, 3]);
        assert_eq!(pixel(&apply(img, &one("bw=0;"), 1.0)), [1, 2, 3]);
    }
}
