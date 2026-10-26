//! Effects with adjustable parameters: Picasa's own effects and the Picnik
//! "Creative Kit" effects Picasa 3 offered.
//!
//! Each effect has a [`Spec`] listing its parameters in the order they are
//! written into `filters=` (see `docs/research/picasa-filter-layout.tsv`;
//! for Picnik effects numbers come first, then colors). The UI builds its
//! controls from these specs.
//!
//! Picnik recipes follow the effect definitions in the Picnik source
//! (Apache-2.0, `ModCreate/effects/*.mxml`); the pixel operations in `ops.rs`
//! are Pico's own. Picasa's native effects are Pico's interpretation of each
//! parameter's meaning, not a copy of Picasa's code.

use crate::filters::{Filter, Value};
use crate::ops::{self, Adjust, Blend, Img, Ring};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Param {
    /// The effect's center, picked on the photo; two values, 0..=1 each.
    Point,
    Slider {
        label: &'static str,
        min: f32,
        max: f32,
        default: f32,
    },
    /// Stored as 0 or 1.
    Check {
        label: &'static str,
        default: bool,
    },
    Color {
        label: &'static str,
        default: u32,
    },
    /// Quarter turns, 0..=3.
    Rotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    /// Picasa's Effects tab.
    Picasa,
    /// Picnik's Creative Kit effects.
    Creative,
    /// Older Picasa filters that still appear in INI files.
    Legacy,
}

pub struct Spec {
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    pub params: &'static [Param],
}

/// A parameter value as edited in the UI.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Point(f32, f32),
    Number(f32),
    Color(u32),
    Rotation(i32),
}

impl ParamValue {
    pub fn number(self) -> f32 {
        match self {
            ParamValue::Number(v) => v,
            ParamValue::Rotation(r) => r as f32,
            _ => 0.0,
        }
    }

    pub fn color(self) -> u32 {
        if let ParamValue::Color(c) = self { c } else { 0 }
    }

    pub fn point(self) -> (f32, f32) {
        if let ParamValue::Point(x, y) = self { (x, y) } else { (0.5, 0.5) }
    }
}

const fn slider(label: &'static str, min: f32, max: f32, default: f32) -> Param {
    Param::Slider { label, min, max, default }
}

const fn color(label: &'static str, default: u32) -> Param {
    Param::Color { label, default }
}

const FADE: Param = slider("Fade", 0.0, 100.0, 0.0);

use Group::{Creative, Legacy, Picasa};

pub const SPECS: &[Spec] = &[
    // Picasa's Effects tab, in Picasa's order.
    Spec { id: "unsharp2", label: "Sharpen", group: Picasa, params: &[slider("Amount", 0.0, 3.0, 0.6)] },
    Spec { id: "sepia", label: "Sepia", group: Picasa, params: &[] },
    Spec { id: "bw", label: "B&W", group: Picasa, params: &[] },
    Spec { id: "warm", label: "Warmify", group: Picasa, params: &[] },
    Spec { id: "grain2", label: "Film Grain", group: Picasa, params: &[] },
    Spec {
        id: "tint",
        label: "Tint",
        group: Picasa,
        params: &[slider("Color Preservation", 0.0, 255.0, 100.0), color("Pick Color", 0xff7bb4d8)],
    },
    Spec { id: "sat", label: "Saturation", group: Picasa, params: &[slider("Amount", -1.0, 1.0, 0.1618)] },
    Spec {
        id: "radblur",
        label: "Soft Focus",
        group: Picasa,
        params: &[Param::Point, slider("Size", -1.0, 1.0, 0.0), slider("Amount", -1.0, 1.0, 0.0)],
    },
    Spec {
        id: "glow2",
        label: "Glow",
        group: Picasa,
        params: &[slider("Intensity", 0.0, 1.0, 0.65), slider("Radius", 0.0, 10.0, 3.0)],
    },
    Spec { id: "ansel", label: "Filtered B&W", group: Picasa, params: &[color("Pick Color", 0xffff8000)] },
    Spec {
        id: "radsat",
        label: "Focal B&W",
        group: Picasa,
        params: &[Param::Point, slider("Size", -1.0, 1.0, 0.0), slider("Sharpness", 0.0, 1.0, 0.0)],
    },
    Spec {
        id: "dir_tint",
        label: "Graduated Tint",
        group: Picasa,
        params: &[
            Param::Point,
            slider("Feather", 0.0, 1.0, 0.25),
            slider("Shade", 0.0, 1.0, 0.25),
            color("Pick Color", 0xff3a6ea5),
            Param::Rotation,
        ],
    },
    // Picnik Creative Kit.
    Spec { id: "Boost", label: "Boost", group: Creative, params: &[slider("Impact", 0.0, 100.0, 50.0)] },
    Spec {
        id: "Border",
        label: "Border",
        group: Creative,
        params: &[
            slider("Outer Thickness", 0.0, 100.0, 20.0),
            slider("Inner Thickness", 0.0, 100.0, 5.0),
            slider("Corner Radius", 0.0, 500.0, 0.0),
            slider("Caption Height", 0.0, 300.0, 0.0),
            color("Outer Color", 0xff000000),
            color("Inner Color", 0xffffffff),
        ],
    },
    Spec {
        id: "Cinemascope",
        label: "Cinemascope",
        group: Creative,
        params: &[Param::Check { label: "Letterbox", default: true }],
    },
    Spec {
        id: "Comicize",
        label: "Comic Book",
        group: Creative,
        params: &[
            slider("Blur", 0.0, 100.0, 20.0),
            slider("Dot Density", 0.0, 100.0, 50.0),
            slider("Dot Fade", 0.0, 100.0, 50.0),
        ],
    },
    Spec { id: "CrossProcess", label: "Cross Process", group: Creative, params: &[FADE] },
    Spec {
        id: "DropShadow",
        label: "Drop Shadow",
        group: Creative,
        params: &[
            slider("Distance", 0.0, 30.0, 4.0),
            slider("Angle", 0.0, 360.0, 90.0),
            slider("Blur", 0.0, 100.0, 10.0),
            slider("Fade", 0.0, 100.0, 30.0),
            color("Shadow", 0xff000000),
            color("Background", 0xffffffff),
        ],
    },
    Spec {
        id: "PicnikFocalPixelate",
        label: "Focal Pixelate",
        group: Creative,
        params: &[
            Param::Point,
            slider("Pixel Size", 2.0, 100.0, 20.0),
            slider("Focal Size", 10.0, 1000.0, 250.0),
            slider("Edge Hardness", 0.0, 100.0, 50.0),
            FADE,
            Param::Check { label: "Reverse", default: false },
        ],
    },
    Spec {
        id: "FocalZoom",
        label: "Focal Zoom",
        group: Creative,
        params: &[
            Param::Point,
            slider("Zoominess", 1.0, 100.0, 50.0),
            slider("Focal Size", 10.0, 1000.0, 250.0),
            slider("Edge Hardness", 0.0, 100.0, 50.0),
            FADE,
        ],
    },
    Spec {
        id: "PicnikGrain",
        label: "Film Grain (Creative)",
        group: Creative,
        params: &[slider("Grain", 0.0, 50.0, 10.0), Param::Check { label: "Lighten", default: false }],
    },
    Spec {
        id: "HDR",
        label: "HDR-ish",
        group: Creative,
        params: &[slider("Radius", 1.3, 80.0, 20.0), slider("Strength", 1.0, 7.0, 3.0), FADE],
    },
    Spec { id: "HeatMap", label: "Heat Map", group: Creative, params: &[slider("Hue", -180.0, 180.0, 0.0), FADE] },
    Spec {
        id: "Holga",
        label: "Holga-ish",
        group: Creative,
        params: &[slider("Blur Edges", 0.0, 100.0, 70.0), slider("Grain", 0.0, 100.0, 30.0), FADE],
    },
    Spec { id: "Invert", label: "Invert Colors", group: Creative, params: &[] },
    Spec { id: "IR", label: "Infrared Film", group: Creative, params: &[FADE] },
    Spec {
        id: "LocalContrast",
        label: "Local Contrast",
        group: Creative,
        params: &[slider("Radius", 1.3, 40.0, 15.0), slider("Strength", 1.0, 3.0, 1.5)],
    },
    Spec { id: "Lomo", label: "Lomo-ish", group: Creative, params: &[slider("Blur Edges", 0.0, 100.0, 50.0), FADE] },
    Spec {
        id: "Matte",
        label: "Matte",
        group: Creative,
        params: &[slider("Blur", 0.0, 50.0, 40.0), slider("Strength", 1.0, 2.0, 1.2), FADE, color("Color", 0xffffffff)],
    },
    Spec {
        id: "MuseumMatte",
        label: "Museum Matte",
        group: Creative,
        params: &[
            slider("Outer Thickness", 0.0, 100.0, 25.0),
            slider("Inner Thickness", 0.0, 100.0, 40.0),
            color("Outer Color", 0xff1a0e03),
            color("Inner Color", 0xfff0eae4),
        ],
    },
    Spec { id: "Neon", label: "Neon", group: Creative, params: &[FADE, color("Color", 0xffff0000)] },
    Spec {
        id: "NightVision",
        label: "Night Vision",
        group: Creative,
        params: &[slider("Brightness", -50.0, 50.0, 0.0), slider("Contrast", -50.0, 50.0, 0.0), FADE],
    },
    Spec {
        id: "Orton",
        label: "Orton-ish",
        group: Creative,
        params: &[slider("Bloom", 0.0, 50.0, 25.0), slider("Brightness", 0.0, 100.0, 50.0), FADE],
    },
    Spec {
        id: "PencilSketch",
        label: "Pencil Sketch",
        group: Creative,
        params: &[slider("Radius", 1.3, 5.0, 2.0), slider("Strength", 0.0, 200.0, 100.0), FADE],
    },
    Spec {
        id: "Pixelate",
        label: "Pixelate",
        group: Creative,
        params: &[slider("Pixel Size", 2.0, 150.0, 20.0), slider("Blend Mode", 0.0, 9.0, 9.0), FADE],
    },
    Spec {
        id: "Polaroid",
        label: "Polaroid",
        group: Creative,
        params: &[slider("Rotation", -10.0, 10.0, 5.0), color("Background", 0xffe2e2e2)],
    },
    Spec {
        id: "QuantizePalette",
        label: "Posterize",
        group: Creative,
        params: &[slider("Number of Colors", 2.0, 30.0, 8.0), slider("Detail", 0.0, 100.0, 80.0), FADE],
    },
    Spec {
        id: "RoundedEdges",
        label: "Rounded Edges",
        group: Creative,
        params: &[slider("Corner Radius", 0.0, 500.0, 60.0), color("Color", 0xffffffff)],
    },
    Spec {
        id: "Sixties",
        label: "1960's",
        group: Creative,
        params: &[
            Param::Check { label: "Rounded", default: true },
            slider("Fade", 0.0, 100.0, 20.0),
            color("Color", 0xffffffff),
        ],
    },
    Spec {
        id: "Soften",
        label: "Soften",
        group: Creative,
        params: &[slider("Impact", 0.0, 100.0, 50.0), slider("Fade", 0.0, 100.0, 50.0)],
    },
    Spec { id: "PicnikTint", label: "Tint (Creative)", group: Creative, params: &[FADE, color("Color", 0xff80cfff)] },
    Spec {
        id: "TwoTone",
        label: "Duo-Tone",
        group: Creative,
        params: &[
            slider("Brightness", -95.0, 95.0, 0.0),
            slider("Contrast", 0.0, 100.0, 20.0),
            FADE,
            color("Dark Color", 0xff004488),
            color("Light Color", 0xffffff00),
        ],
    },
    Spec {
        id: "Vignette",
        label: "Vignette",
        group: Creative,
        params: &[slider("Blur", 0.0, 50.0, 35.0), slider("Strength", 1.0, 2.0, 1.4), FADE, color("Color", 0xff000000)],
    },
    // Older filters Picasa still reads.
    Spec { id: "grain", label: "Film Grain (Old)", group: Legacy, params: &[] },
    Spec {
        id: "glow",
        label: "Glow (Old)",
        group: Legacy,
        params: &[slider("Intensity", 0.0, 1.0, 0.65), slider("Radius", 0.0, 10.0, 3.0)],
    },
    Spec { id: "unsharp", label: "Sharpen (Old)", group: Legacy, params: &[slider("Amount", 0.0, 1.0, 0.6)] },
    Spec {
        id: "radtint",
        label: "Radial Tint",
        group: Legacy,
        params: &[Param::Point, slider("Feather", 0.0, 1.0, 0.25), color("Color", 0xff3a6ea5)],
    },
    Spec {
        id: "linblur",
        label: "Linear Blur",
        group: Legacy,
        params: &[Param::Point, slider("Amount", 0.0, 10.0, 2.0)],
    },
    Spec {
        id: "colortemp",
        label: "Color Temperature",
        group: Legacy,
        params: &[slider("Cool to Warm", -0.5, 0.5, 0.125), slider("White Shift", 0.0, 1.0, 0.0)],
    },
    Spec {
        id: "shadow",
        label: "Shadow & Highlight",
        group: Legacy,
        params: &[
            slider("Radius", 0.0, 1.0, 0.0),
            slider("Shadow %", 0.0, 1.0, 0.0),
            slider("Highlight %", 0.0, 1.0, 0.0),
        ],
    },
    Spec { id: "blur", label: "Blur", group: Legacy, params: &[slider("Threshold", -0.5, 0.5, 0.1)] },
    Spec { id: "contrast", label: "Contrast", group: Legacy, params: &[slider("Contrast", -0.5, 0.5, 0.1)] },
    Spec { id: "gamma", label: "Gamma Correct", group: Legacy, params: &[slider("Level", -1.0, 1.0, 0.1618)] },
    Spec { id: "backlight", label: "Backlight Fix", group: Legacy, params: &[slider("Amount", 0.0, 1.0, 0.25)] },
    Spec { id: "autobacklight", label: "Fill Light (Auto)", group: Legacy, params: &[] },
    Spec { id: "whitept", label: "Whitepoint", group: Legacy, params: &[color("Whitepoint", 0)] },
    Spec {
        id: "triple",
        label: "Lighting Fixes",
        group: Legacy,
        params: &[
            slider("Brightness", -1.0, 1.0, 0.0),
            slider("Contrast", -0.5, 0.5, 0.0),
            slider("Fill Light", 0.0, 1.0, 0.0),
        ],
    },
    Spec {
        id: "triple2",
        label: "Lighting Fixes",
        group: Legacy,
        params: &[
            slider("Fill Light", 0.0, 1.0, 0.0),
            slider("Black Point", 0.0, 1.0, 0.0),
            slider("White Point", 0.0, 1.0, 1.0),
        ],
    },
    Spec {
        id: "triple3",
        label: "Lighting Fixes",
        group: Legacy,
        params: &[
            slider("Fill Light", 0.0, 1.0, 0.0),
            slider("Highlights", 0.0, 0.48, 0.0),
            slider("Shadows", 0.0, 0.48, 0.0),
        ],
    },
    Spec {
        id: "colorfix",
        label: "Color Fixes",
        group: Legacy,
        params: &[slider("Color Temperature", -0.5, 0.5, 0.0), color("White Point", 0)],
    },
    Spec { id: "dir_sat", label: "Directional Saturation", group: Legacy, params: &DIRECTIONAL },
    Spec { id: "dir_brite", label: "Directional Brightness", group: Legacy, params: &DIRECTIONAL },
    Spec { id: "dir_sharp", label: "Directional Sharpen", group: Legacy, params: &DIRECTIONAL },
    Spec {
        id: "focalpixelate",
        label: "Focal Pixelate (Old)",
        group: Legacy,
        params: &[
            Param::Point,
            slider("Pixel Size", 0.0, 100.0, 15.0),
            slider("Focal Size", 0.0, 2.0, 1.0),
            slider("Edge Hardness", 0.0, 0.95, 0.25),
            slider("Fade", 0.0, 1.0, 0.0),
        ],
    },
];

const DIRECTIONAL: [Param; 2] = [slider("Left to Right", -1.0, 1.0, 0.0), slider("Top to Bottom", -1.0, 1.0, 0.0)];

pub fn spec(id: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.id == id)
}

/// Reads an edit's parameters, falling back to defaults for missing ones.
pub fn read_params(spec: &Spec, filter: &Filter) -> Vec<ParamValue> {
    let values = filter.values();
    let mut i = 0;
    let mut next = || {
        let v = values.get(i).cloned();
        i += 1;
        v
    };
    spec.params
        .iter()
        .map(|param| match *param {
            Param::Point => {
                let x = next().and_then(|v| v.parse().ok()).unwrap_or(0.5);
                let y = next().and_then(|v| v.parse().ok()).unwrap_or(0.5);
                ParamValue::Point(x, y)
            }
            Param::Slider { default, .. } => ParamValue::Number(next().and_then(|v| v.parse().ok()).unwrap_or(default)),
            Param::Check { default, .. } => {
                ParamValue::Number(next().and_then(|v| v.parse().ok()).unwrap_or(if default { 1.0 } else { 0.0 }))
            }
            Param::Color { default, .. } => {
                ParamValue::Color(next().and_then(|v| u32::from_str_radix(&v, 16).ok()).unwrap_or(default))
            }
            Param::Rotation => ParamValue::Rotation(next().and_then(|v| v.parse().ok()).unwrap_or(0)),
        })
        .collect()
}

pub fn defaults(spec: &Spec) -> Vec<ParamValue> {
    read_params(spec, &Filter::effect(spec.id, []))
}

/// The edit for `spec` with these parameter values.
pub fn to_filter(spec: &Spec, params: &[ParamValue]) -> Filter {
    let values = params.iter().flat_map(|p| match *p {
        ParamValue::Point(x, y) => vec![Value::Float(x), Value::Float(y)],
        ParamValue::Number(v) => vec![Value::Float(v)],
        ParamValue::Color(c) => vec![Value::Color(c)],
        ParamValue::Rotation(r) => vec![Value::Int(r)],
    });
    Filter::effect(spec.id, values.collect::<Vec<_>>())
}

/// Renders the effect. `scale` is the size of the image being rendered
/// relative to the original photo, so pixel-sized parameters keep their look
/// on previews and thumbnails.
pub fn render(spec: &Spec, img: Img, params: &[ParamValue], scale: f32) -> Img {
    let n = |i: usize| params.get(i).map_or(0.0, |p| p.number());
    let c = |i: usize| params.get(i).map_or(0, |p| p.color());
    let point = |i: usize| params.get(i).map_or((0.5, 0.5), |p| p.point());
    let px = |v: f32| v * scale;
    let fade = |v: f32| 1.0 - v / 100.0;
    let mut img = img;
    match spec.id {
        // ---- Picasa
        // sepia, bw, warm, sat and the sharpens are rendered by `adjust`.
        "grain" | "grain2" => {
            let grain = ops::noise(img.width, img.height, 7, 90.0, 165.0, true);
            ops::blend(&mut img, &grain, Blend::Overlay, 0.6, None);
        }
        "tint" => {
            let original = img.clone();
            ops::tint(&mut img, c(1));
            // Color preservation keeps some of the original color.
            ops::blend(&mut img, &original, Blend::Normal, (n(0) / 255.0).clamp(0.0, 1.0), None);
        }
        "radblur" => {
            let (x, y) = point(0);
            let blurred = ops::blur(&img, px(18.0 + 14.0 * n(2)), px(18.0 + 14.0 * n(2)), 3);
            let mask = focal_mask(&img, (x, y), (n(1) + 1.0) / 2.0, 0.5);
            ops::blend(&mut img, &blurred, Blend::Normal, 1.0, Some(&mask));
        }
        "radsat" => {
            let (x, y) = point(0);
            let mut gray = img.clone();
            ops::black_and_white(&mut gray, 0xffffff);
            let mask = focal_mask(&img, (x, y), (n(1) + 1.0) / 2.0, 1.0 - n(2) * 0.9);
            ops::blend(&mut img, &gray, Blend::Normal, 1.0, Some(&mask));
        }
        "glow" | "glow2" => {
            let mut bright = ops::blur(&img, px(4.0 + 6.0 * n(1)), px(4.0 + 6.0 * n(1)), 3);
            ops::curves(&mut bright, &[(0.0, 0.0), (128.0, 90.0), (255.0, 255.0)], [&[], &[], &[]]);
            ops::blend(&mut img, &bright, Blend::Screen, n(0), None);
        }
        "ansel" => ops::black_and_white(&mut img, c(0)),
        "dir_tint" | "radtint" => {
            let (x, y) = point(0);
            let color = c(if spec.id == "dir_tint" { 3 } else { 2 });
            let shade = if spec.id == "dir_tint" { n(2) } else { 0.5 };
            let feather = n(1).clamp(0.02, 1.0);
            let rotation = if spec.id == "dir_tint" { n(4) as i32 } else { 0 };
            let tint_color = ops::rgb(color);
            let (w, h) = (img.width as f32, img.height as f32);
            img.map_xy(|px_x, px_y, p| {
                let (u, v) = (px_x as f32 / w, px_y as f32 / h);
                let t = if spec.id == "radtint" {
                    (((u - x).powi(2) + (v - y).powi(2)).sqrt() - 0.2) / feather
                } else {
                    // Tint comes from the top (rotated by quarter turns) and fades out around the point.
                    let (along, at) = match rotation.rem_euclid(4) {
                        1 => (1.0 - u, 1.0 - x),
                        2 => (1.0 - v, 1.0 - y),
                        3 => (u, x),
                        _ => (v, y),
                    };
                    (at - along) / feather + 0.5
                };
                let amount = t.clamp(0.0, 1.0) * (0.4 + shade);
                let tinted = [p[0] * tint_color[0], p[1] * tint_color[1], p[2] * tint_color[2], p[3]];
                ops::mix(p, tinted, amount.min(1.0))
            });
        }
        "linblur" => {
            let blurred = ops::blur(&img, px(n(1) * 6.0), 1.0, 3);
            ops::blend(&mut img, &blurred, Blend::Normal, 1.0, None);
        }
        "colortemp" => {
            let t = n(0);
            img.map(|p| [p[0] * (1.0 + 0.4 * t), p[1], p[2] * (1.0 - 0.4 * t), p[3]]);
        }
        "shadow" => {
            let (shadows, highlights) = (n(1), n(2));
            let blurred = ops::blur(&img, px(10.0 + 60.0 * n(0)), px(10.0 + 60.0 * n(0)), 3);
            for (p, b) in img.px.iter_mut().zip(&blurred.px) {
                let y = ops::luma(*b);
                let lift = shadows * (1.0 - y).powi(2) * 0.6 - highlights * y.powi(2) * 0.6;
                for v in p.iter_mut().take(3) {
                    *v = (*v + lift).clamp(0.0, 1.0);
                }
            }
        }
        "blur" => {
            let s = px((n(0) + 0.5) * 12.0);
            img = ops::blur(&img, s, s, 3);
        }
        "contrast" => ops::adjust(&mut img, Adjust { contrast: n(0) * 100.0, ..Default::default() }),
        "gamma" => {
            let gamma = 1.0 / (1.0 + n(0)).max(0.05);
            img.map(|p| [p[0].powf(gamma), p[1].powf(gamma), p[2].powf(gamma), p[3]]);
        }
        "backlight" | "autobacklight" => {
            let fill = if spec.id == "autobacklight" { 0.35 } else { n(0) };
            img.map(|p| {
                let f = |v: f32| 1.0 - (1.0 - v).powf(1.0 + 1.5 * fill);
                [f(p[0]), f(p[1]), f(p[2]), p[3]]
            });
        }
        "whitept" | "colorfix" => {
            let white = c(if spec.id == "colorfix" { 1 } else { 0 });
            let t = if spec.id == "colorfix" { n(0) } else { 0.0 };
            let gains = if white & 0xffffff == 0 {
                [1.0; 3]
            } else {
                let w = ops::rgb(white);
                let gray = (w[0] + w[1] + w[2]) / 3.0;
                [gray / w[0].max(0.01), gray / w[1].max(0.01), gray / w[2].max(0.01)]
            };
            img.map(|p| [p[0] * gains[0] * (1.0 + 0.4 * t), p[1] * gains[1], p[2] * gains[2] * (1.0 - 0.4 * t), p[3]]);
        }
        "triple" | "triple2" | "triple3" => {
            let (fill, low, high, brightness, contrast) = match spec.id {
                "triple" => (n(2), 0.0, 1.0, n(0) * 0.25, n(1)),
                "triple2" => (n(0), n(1) * 0.5, 0.5 + n(2) * 0.5, 0.0, 0.0),
                _ => (n(0), n(2), 1.0 - n(1), 0.0, 0.0),
            };
            img.map(|p| {
                let f = |v: f32| {
                    let v = 1.0 - (1.0 - v).powf(1.0 + 1.5 * fill);
                    let v = (v - low) / (high - low).max(0.01);
                    ((v - 0.5) * (1.0 + 2.0 * contrast) + 0.5 + brightness).clamp(0.0, 1.0)
                };
                [f(p[0]), f(p[1]), f(p[2]), p[3]]
            });
        }
        "dir_sat" | "dir_brite" | "dir_sharp" => {
            let (lr, tb) = (n(0), n(1));
            let sharp = (spec.id == "dir_sharp").then(|| {
                let mut s = img.clone();
                ops::local_contrast(&mut s, px(4.0).max(1.0), 1.0);
                s
            });
            let (w, h) = (img.width as f32, img.height as f32);
            img.map_xy(|x, y, p| {
                let k = lr * (x as f32 / w - 0.5) * 2.0 + tb * (y as f32 / h - 0.5) * 2.0;
                match spec.id {
                    "dir_sat" => {
                        let l = ops::luma(p);
                        let s = 1.0 + k * 0.5;
                        [l + (p[0] - l) * s, l + (p[1] - l) * s, l + (p[2] - l) * s, p[3]]
                    }
                    "dir_brite" => [p[0] + k * 0.15, p[1] + k * 0.15, p[2] + k * 0.15, p[3]],
                    _ => sharp.as_ref().map_or(p, |s| ops::mix(p, s.at(x, y), k.clamp(0.0, 1.0))),
                }
            });
        }
        "focalpixelate" => {
            let (x, y) = point(0);
            let mut blocky = img.clone();
            ops::pixelate(&mut blocky, px(n(1)).max(2.0) as usize);
            let mask = focal_mask(&img, (x, y), n(2) / 2.0, 1.0 - n(3));
            ops::blend(&mut img, &blocky, Blend::Normal, 1.0 - n(4), Some(&mask));
        }

        // ---- Picnik Creative Kit
        "Boost" => ops::adjust(
            &mut img,
            Adjust {
                brightness: n(0) * -20.0 / 50.0,
                saturation: n(0) * 20.0 / 50.0,
                contrast: n(0) * 40.0 / 50.0,
                hue: 0.0,
            },
        ),
        "Border" => {
            img = border(&img, c(4), px(n(0)), c(5), px(n(1)), px(n(2)), px(n(3)));
        }
        "Cinemascope" => {
            let letterbox = n(0) != 0.0;
            let (w, h) = (img.width, img.height);
            let crop_h = if letterbox { ((w as f32 / 1.7).round() as usize).min(h) } else { h };
            img = crop(&img, 0, (h - crop_h) / 2, w, crop_h);
            let original = img.clone();
            ops::auto_fix(&mut img);
            ops::adjust(&mut img, Adjust { saturation: -25.0, ..Default::default() });
            ops::curves(
                &mut img,
                &[(0.0, 0.0), (29.0, 19.0), (110.0, 150.0), (233.0, 245.0), (255.0, 255.0)],
                [&[(0.0, 0.0), (111.0, 141.0), (255.0, 255.0)], &[], &[(0.0, 0.0), (136.0, 121.0), (255.0, 255.0)]],
            );
            let grain = ops::noise(img.width, img.height, 0, 225.0, 245.0, true);
            ops::blend(&mut img, &grain, Blend::Multiply, 1.0, None);
            let _ = original;
            if letterbox {
                let bar = (crop_h as f32 * 0.15).round() as usize;
                img = ops::expand(&img, 0, bar, 0, bar, 0x000000);
            }
        }
        "Comicize" => {
            let original = img.clone();
            let dot = (img.width as f32 / 70.0).round() as usize + 1;
            let blurred = ops::blur(&img, 1.0 + 20.0 * n(0) / 100.0, 1.0 + 20.0 * n(0) / 100.0, 3);
            ops::blend(&mut img, &blurred, Blend::Darken, 1.0, None);
            let mut dots = original;
            ops::curves(
                &mut dots,
                &[(0.0, 0.0), (48.0, 48.0), (90.0 + n(1) * 1.5, 254.0), (255.0, 255.0)],
                [&[], &[], &[]],
            );
            ops::pixelate(&mut dots, dot);
            halftone(&mut dots, dot);
            ops::blend(&mut img, &dots, Blend::Multiply, 0.5 - n(2) / 200.0, None);
        }
        "CrossProcess" => {
            let original = img.clone();
            ops::curves(
                &mut img,
                &[],
                [
                    &[(0.0, 0.0), (60.0, 30.0), (210.0, 255.0), (255.0, 255.0)],
                    &[(0.0, 0.0), (47.0, 38.0), (101.0, 111.0), (187.0, 206.0), (255.0, 255.0)],
                    &[(0.0, 32.0), (255.0, 216.0)],
                ],
            );
            ops::adjust(&mut img, Adjust { contrast: 10.0, brightness: 10.0, ..Default::default() });
            let mut yellow = img.clone();
            ops::tint(&mut yellow, 0xfcff00);
            ops::blend(&mut img, &yellow, Blend::Multiply, 0.2, None);
            fade_from(&mut img, &original, fade(n(0)));
        }
        "DropShadow" => {
            img = drop_shadow(&img, c(4), c(5), 1.0 - n(3) / 100.0, px(n(0)), n(1), px(n(2)));
        }
        "PicnikFocalPixelate" | "FocalZoom" => {
            let (x, y) = point(0);
            let radius = px(n(2));
            let hardness = n(3) / 101.0;
            let reverse = spec.id == "PicnikFocalPixelate" && n(5) != 0.0;
            let (cx, cy) = (x * img.width as f32, y * img.height as f32);
            let mask = ops::circular_mask(
                img.width,
                img.height,
                Ring {
                    cx,
                    cy,
                    inner_radius: radius * hardness,
                    outer_radius: radius * (2.0 - hardness),
                    inner: if reverse { 1.0 } else { 0.0 },
                    outer: if reverse { 0.0 } else { 1.0 },
                },
            );
            let mut effect = img.clone();
            if spec.id == "FocalZoom" {
                effect = radial_blur(&img, cx, cy, n(1) / 100.0);
            } else {
                ops::pixelate(&mut effect, px(n(1)).max(2.0) as usize);
            }
            ops::blend(&mut img, &effect, Blend::Normal, fade(n(4)), Some(&mask));
        }
        "PicnikGrain" => {
            let lighten = n(1) != 0.0;
            let g = 2.55 * n(0);
            let grain = if lighten {
                ops::noise(img.width, img.height, 1, 0.0, g, true)
            } else {
                ops::noise(img.width, img.height, 1, 255.0 - g, 255.0, true)
            };
            ops::blend(&mut img, &grain, if lighten { Blend::Lighten } else { Blend::Darken }, 1.0, None);
        }
        "HDR" | "LocalContrast" => {
            let original = img.clone();
            ops::local_contrast(&mut img, px(n(0)), n(1));
            if spec.id == "HDR" {
                fade_from(&mut img, &original, fade(n(2)));
            }
        }
        "HeatMap" => {
            let original = img.clone();
            let hue = n(0);
            let stops: Vec<(f32, [f32; 3])> =
                [(240.0, 0.5, 0.0), (240.0, 1.0, 31.875), (120.0, 1.0, 127.5), (0.0, 1.0, 223.125), (0.0, 0.5, 255.0)]
                    .iter()
                    .map(|&(h, v, pos)| (pos / 255.0, ops::hsv_to_rgb(h + hue, 1.0, v)))
                    .collect();
            ops::gradient_map(&mut img, &stops);
            fade_from(&mut img, &original, fade(n(1)));
        }
        "Holga" => {
            let original = img.clone();
            let m = img.max_side() / 2.0;
            let mask = ops::circular_mask(
                img.width,
                img.height,
                Ring {
                    cx: img.width as f32 / 2.0,
                    cy: img.height as f32 / 2.0,
                    inner_radius: m * 0.9,
                    outer_radius: m * (2.0 - n(0) / 100.0),
                    inner: 0.0,
                    outer: 1.0,
                },
            );
            ops::auto_fix(&mut img);
            ops::inner_glow(&mut img, 0x000000, 0.5 * m, 0.4 * m, 1.4, 1.0);
            let blurred = ops::blur(&img, px(18.0), px(20.0), 3);
            ops::blend(&mut img, &blurred, Blend::Normal, 1.0, Some(&mask));
            ops::black_and_white(&mut img, 0xff6666);
            ops::adjust(&mut img, Adjust { contrast: 25.0, ..Default::default() });
            let grain = ops::noise(img.width, img.height, 5, 255.0 - (n(1) / 200.0 * 255.0).round(), 255.0, true);
            ops::blend(&mut img, &grain, Blend::Multiply, 0.6, None);
            fade_from(&mut img, &original, fade(n(2)));
        }
        "Invert" => img.map(|p| [1.0 - p[0], 1.0 - p[1], 1.0 - p[2], p[3]]),
        "IR" => {
            let original = img.clone();
            let mut green = img.clone();
            green.map(|p| [p[1], p[1], p[1], p[3]]);
            let green = ops::blur(&green, px(5.0), px(5.0), 3);
            ops::blend(&mut img, &green, Blend::Screen, 0.25, None);
            // Foliage (green) turns bright, sky (blue) dark.
            img.map(|p| {
                let v = (2.0 * p[1] - 0.5 * p[0] - 0.5 * p[2]).clamp(0.0, 1.0);
                [v, v, v, p[3]]
            });
            fade_from(&mut img, &original, fade(n(0)));
        }
        "Lomo" => {
            let original = img.clone();
            let m = img.max_side() / 2.0;
            let mask = ops::circular_mask(
                img.width,
                img.height,
                Ring {
                    cx: img.width as f32 / 2.0,
                    cy: img.height as f32 / 2.0,
                    inner_radius: m * 0.5,
                    outer_radius: m * (2.0 - n(0) / 100.0),
                    inner: 0.0,
                    outer: 1.0,
                },
            );
            ops::inner_glow(&mut img, 0x000000, 35.0 * 0.02 * m, 35.0 * 0.02 * m, 1.1, 1.0);
            let blurred = ops::blur(&img, px(20.0), px(20.0), 3);
            ops::blend(&mut img, &blurred, Blend::Normal, 1.0, Some(&mask));
            ops::adjust(&mut img, Adjust { saturation: 20.0, contrast: 35.0, brightness: 5.0, hue: 0.0 });
            fade_from(&mut img, &original, fade(n(1)));
        }
        "Matte" | "Vignette" => {
            let b = n(0) * 0.02 * img.max_side() / 4.0;
            ops::inner_glow(&mut img, c(3), b, b, n(1), fade(n(2)));
        }
        "MuseumMatte" => {
            let b = 2.0 * 0.02 * img.max_side() / 4.0;
            ops::inner_glow(&mut img, 0x000000, b, b, 1.3, 0.7);
            img = border(&img, 0, 0.0, c(3), px(n(1)), 0.0, 0.0);
            ops::inner_glow(&mut img, 0x000000, b, b, 1.3, 0.6);
            img = border(&img, c(2), px(n(0)), 0, 0.0, 0.0, 0.0);
        }
        "Neon" => {
            let original = img.clone();
            img = edges(&img);
            ops::tint(&mut img, c(1));
            fade_from(&mut img, &original, fade(n(0)));
        }
        "NightVision" => {
            let original = img.clone();
            ops::auto_fix(&mut img);
            ops::gradient_map(
                &mut img,
                &[(0.0, [0.0, 0.0, 0.0]), (1.0, [0x57 as f32 / 255.0, 0xcc as f32 / 255.0, 0x29 as f32 / 255.0])],
            );
            let b = 35.0 * 0.02 * img.max_side() / 3.0;
            ops::inner_glow(&mut img, 0x000000, b, b, 1.5, 1.0);
            let speckle = ops::noise(img.width, img.height, 30, 0.0, 180.0, false);
            ops::blend(&mut img, &speckle, Blend::Lighten, 0.2, None);
            ops::adjust(&mut img, Adjust { contrast: n(1), brightness: n(0), ..Default::default() });
            fade_from(&mut img, &original, fade(n(2)));
        }
        "Orton" => {
            let original = img.clone();
            let bloom = ops::blur(&img, px(n(0)), px(n(0)), 3);
            ops::blend(&mut img, &bloom, Blend::Overlay, 1.0, None);
            let mid = 128.0 + (n(1) - 50.0) * 75.0 / 50.0;
            ops::curves(&mut img, &[(0.0, 0.0), (128.0, mid), (255.0, 255.0)], [&[], &[], &[]]);
            fade_from(&mut img, &original, fade(n(2)));
        }
        "PencilSketch" => {
            let original = img.clone();
            ops::black_and_white(&mut img, 0xffffff);
            ops::auto_fix(&mut img);
            let base = img.clone();
            let mut inverted = img.clone();
            inverted.map(|p| [1.0 - p[0], 1.0 - p[1], 1.0 - p[2], p[3]]);
            let inverted = ops::blur(&inverted, px(n(0)), px(n(0)), 3);
            ops::blend(&mut img, &inverted, Blend::Add, 1.0, None);
            ops::blend(&mut img, &base, Blend::Overlay, 1.0, None);
            ops::auto_fix(&mut img);
            let mid = (127.0 - n(1) * 2.0 + 100.0).clamp(1.0, 254.0);
            ops::curves(&mut img, &[(0.0, 0.0), (127.0, mid), (255.0, 255.0)], [&[], &[], &[]]);
            fade_from(&mut img, &original, fade(n(2)));
        }
        "Pixelate" => {
            let mut blocky = img.clone();
            ops::pixelate(&mut blocky, px(n(0)).max(2.0) as usize);
            ops::blend(&mut img, &blocky, Blend::from_flash(n(1)), fade(n(2)), None);
            if Blend::from_flash(n(1)) == Blend::Subtract {
                // Flash mode 9 is "layer", Picnik's default: plain replacement.
                ops::blend(&mut img, &blocky, Blend::Normal, fade(n(2)), None);
            }
        }
        "Polaroid" => img = polaroid(&img, n(0), c(1)),
        "QuantizePalette" => {
            let original = img.clone();
            let s = px((100.0 - n(1)) / 10.0 + 0.1);
            img = ops::blur(&img, s, s, 3);
            ops::posterize(&mut img, n(0));
            fade_from(&mut img, &original, fade(n(2)));
        }
        "RoundedEdges" => ops::frame(&mut img, c(1), 0.0, px(n(0))),
        "Sixties" => {
            let original = img.clone();
            ops::auto_fix(&mut img);
            ops::curves(
                &mut img,
                &[(0.0, 0.0), (150.0, 104.0), (243.0, 255.0)],
                [
                    &[(0.0, 59.0), (96.0, 156.0), (210.0, 255.0)],
                    &[(0.0, 22.0), (150.0, 166.0), (255.0, 216.0)],
                    &[(0.0, 9.0), (126.0, 98.0), (255.0, 231.0)],
                ],
            );
            let grain = ops::noise(img.width, img.height, 5, 235.0, 255.0, true);
            ops::blend(&mut img, &grain, Blend::Multiply, 0.6, None);
            fade_from(&mut img, &original, fade(n(1)));
            if n(0) != 0.0 {
                let r = img.width.min(img.height) as f32 / 14.0;
                ops::frame(&mut img, c(2), 0.0, r);
            }
        }
        "Soften" => {
            let s = px(n(0) * 20.0 / 50.0);
            let blurred = ops::blur(&img, s, s, 3);
            ops::blend(&mut img, &blurred, Blend::Normal, (100.0 - n(1)) * 0.8 / 100.0, None);
        }
        "PicnikTint" => {
            let original = img.clone();
            ops::tint(&mut img, c(1));
            fade_from(&mut img, &original, fade(n(0)));
        }
        "TwoTone" => {
            let original = img.clone();
            ops::adjust(&mut img, Adjust { saturation: -100.0, brightness: n(0), contrast: n(1), hue: 0.0 });
            let (dark, light) = (ops::rgb(c(3)), ops::rgb(c(4)));
            ops::gradient_map(&mut img, &[(0.0, [dark[0], dark[1], dark[2]]), (1.0, [light[0], light[1], light[2]])]);
            fade_from(&mut img, &original, fade(n(2)));
        }
        _ => {}
    }
    img
}

/// Blends `original` back over the effect so that `keep` of the effect remains.
fn fade_from(img: &mut Img, original: &Img, keep: f32) {
    if keep < 1.0 && original.width == img.width && original.height == img.height {
        ops::blend(img, original, Blend::Normal, 1.0 - keep.clamp(0.0, 1.0), None);
    }
}

/// 0 inside a circle around `center` whose radius is `size` of the half
/// diagonal, ramping to 1 outside; `hardness` near 1 gives a sharp edge.
fn focal_mask(img: &Img, center: (f32, f32), size: f32, hardness: f32) -> Vec<f32> {
    let half_diagonal = ((img.width.pow(2) + img.height.pow(2)) as f32).sqrt() / 2.0;
    let radius = size.clamp(0.0, 1.0) * half_diagonal;
    let soft = (1.0 - hardness.clamp(0.0, 1.0)) * half_diagonal * 0.6 + 2.0;
    ops::circular_mask(
        img.width,
        img.height,
        Ring {
            cx: center.0 * img.width as f32,
            cy: center.1 * img.height as f32,
            inner_radius: radius,
            outer_radius: radius + soft,
            inner: 0.0,
            outer: 1.0,
        },
    )
}

fn crop(img: &Img, x: usize, y: usize, w: usize, h: usize) -> Img {
    let mut out = Img::filled(w, h, [0.0; 4]);
    for row in 0..h {
        out.px[row * w..(row + 1) * w]
            .copy_from_slice(&img.px[(row + y) * img.width + x..(row + y) * img.width + x + w]);
    }
    out
}

/// Picnik's border: an inner and an outer band added around the photo, rounded
/// corners and an optional caption strip below.
fn border(img: &Img, outer: u32, outer_px: f32, inner: u32, inner_px: f32, corner: f32, caption: f32) -> Img {
    let (i, o) = (inner_px.round() as usize, outer_px.round() as usize);
    let mut out = ops::expand(img, i, i, i, i, inner);
    if corner > 0.0 {
        ops::frame(&mut out, outer, 0.0, corner);
    }
    ops::expand(&out, o, o, o, o + caption.round() as usize, outer)
}

fn halftone(img: &mut Img, cell: usize) {
    let cell = cell.max(2) as f32;
    img.map_xy(|x, y, p| {
        let (fx, fy) = ((x as f32 % cell) / cell - 0.5, (y as f32 % cell) / cell - 0.5);
        let r = (1.0 - ops::luma(p)).sqrt() * 0.7;
        if (fx * fx + fy * fy).sqrt() < r { [0.1, 0.1, 0.1, p[3]] } else { [1.0, 1.0, 1.0, p[3]] }
    });
}

/// Simple edge detection for Neon: bright lines on black.
fn edges(img: &Img) -> Img {
    let soft = ops::blur(img, 2.0, 2.0, 1);
    let mut out = img.clone();
    let (w, h) = (img.width, img.height);
    for y in 0..h {
        for x in 0..w {
            let l = |dx: isize, dy: isize| {
                let (xx, yy) = ((x as isize + dx).clamp(0, w as isize - 1), (y as isize + dy).clamp(0, h as isize - 1));
                ops::luma(soft.at(xx as usize, yy as usize))
            };
            let gx = l(1, 0) - l(-1, 0);
            let gy = l(0, 1) - l(0, -1);
            let e = ((gx * gx + gy * gy).sqrt() * 4.0).clamp(0.0, 1.0);
            out.px[y * w + x] = [e, e, e, img.at(x, y)[3]];
        }
    }
    out
}

fn radial_blur(img: &Img, cx: f32, cy: f32, amount: f32) -> Img {
    let steps = 12;
    let mut out = img.clone();
    let (w, h) = (img.width, img.height);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f32; 4];
            for s in 0..steps {
                let k = 1.0 - amount * 0.15 * s as f32 / steps as f32;
                let sx = (cx + (x as f32 - cx) * k).clamp(0.0, w as f32 - 1.0) as usize;
                let sy = (cy + (y as f32 - cy) * k).clamp(0.0, h as f32 - 1.0) as usize;
                let p = img.at(sx, sy);
                for c in 0..4 {
                    sum[c] += p[c];
                }
            }
            out.px[y * w + x] = sum.map(|v| v / steps as f32);
        }
    }
    out
}

fn drop_shadow(img: &Img, shadow: u32, background: u32, alpha: f32, distance: f32, angle: f32, blur_px: f32) -> Img {
    let margin = (distance + blur_px * 1.5).ceil() as usize + 2;
    let mut out = ops::expand(img, margin, margin, margin, margin, background);
    let mut shape = Img::filled(out.width, out.height, [0.0; 4]);
    let (dx, dy) = (angle.to_radians().cos() * distance, angle.to_radians().sin() * distance);
    for y in 0..img.height {
        for x in 0..img.width {
            let (sx, sy) = ((x + margin) as f32 + dx, (y + margin) as f32 + dy);
            if sx >= 0.0 && sy >= 0.0 && (sx as usize) < out.width && (sy as usize) < out.height {
                shape.px[sy as usize * out.width + sx as usize] = [1.0; 4];
            }
        }
    }
    let shape = ops::blur(&shape, blur_px.max(1.0), blur_px.max(1.0), 3);
    let s = ops::rgb(shadow);
    out.map_xy(|x, y, p| {
        let inside = x >= margin && y >= margin && x < margin + img.width && y < margin + img.height;
        if inside { p } else { ops::mix(p, s, shape.at(x, y)[0] * alpha) }
    });
    for y in 0..img.height {
        let start = (y + margin) * out.width + margin;
        out.px[start..start + img.width].copy_from_slice(&img.px[y * img.width..(y + 1) * img.width]);
    }
    out
}

/// A square crop on white instant-film paper, slightly turned.
fn polaroid(img: &Img, degrees: f32, background: u32) -> Img {
    let size = img.width.min(img.height);
    let square = crop(img, (img.width - size) / 2, (img.height - size) / 2, size, size);
    let s = size as f32;
    let (lr, top, bottom) =
        ((s * 0.0645).round() as usize, (s * 0.0968).round() as usize, (s * 0.258).round() as usize);
    let card = ops::expand(&square, lr, top, lr, bottom, 0xffffff);
    let shadowed = drop_shadow(&card, 0x000000, background, 0.35, s * 0.01, 90.0, s * 0.02);
    rotate_on(&shadowed, degrees, background)
}

fn rotate_on(img: &Img, degrees: f32, background: u32) -> Img {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (w, h) = (img.width as f32, img.height as f32);
    let (nw, nh) = ((w * cos.abs() + h * sin.abs()).ceil(), (w * sin.abs() + h * cos.abs()).ceil());
    let mut out = Img::filled(nw as usize, nh as usize, ops::rgb(background));
    let (cx, cy, ncx, ncy) = (w / 2.0, h / 2.0, nw / 2.0, nh / 2.0);
    out.map_xy(|x, y, p| {
        let (dx, dy) = (x as f32 + 0.5 - ncx, y as f32 + 0.5 - ncy);
        let (sx, sy) = (cos * dx + sin * dy + cx, -sin * dx + cos * dy + cy);
        if sx >= 0.0 && sy >= 0.0 && sx < w && sy < h { img.at(sx as usize, sy as usize) } else { p }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterStack;

    #[test]
    fn every_spec_id_is_unique_and_renders() {
        let mut ids: Vec<_> = SPECS.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), SPECS.len());
        let img = Img::filled(48, 32, [0.5, 0.4, 0.3, 1.0]);
        for spec in SPECS {
            let out = render(spec, img.clone(), &defaults(spec), 1.0);
            assert!(out.width > 0 && out.height > 0, "{}", spec.id);
            assert!(out.px.iter().all(|p| p.iter().all(|c| c.is_finite())), "{} produced NaN", spec.id);
        }
    }

    #[test]
    fn reads_real_picnik_parameters() {
        // From a real .picasa.ini: blur, strength, fade, then the color.
        let filter = FilterStack::parse("Vignette=1,50.000000,1.400000,84.210526,00000000;").0.remove(0);
        let params = read_params(spec("Vignette").unwrap(), &filter);
        assert_eq!(
            params,
            [ParamValue::Number(50.0), ParamValue::Number(1.4), ParamValue::Number(84.210526), ParamValue::Color(0)]
        );
        assert_eq!(
            to_filter(spec("Vignette").unwrap(), &params).to_string(),
            "Vignette=1,50.000000,1.400000,84.210526,00000000;"
        );

        // Picasa effects with a center point store it first.
        let filter = FilterStack::parse("radblur=1,0.500000,0.500000,1.000000,-0.497076;").0.remove(0);
        let params = read_params(spec("radblur").unwrap(), &filter);
        assert_eq!(params[0], ParamValue::Point(0.5, 0.5));
        assert_eq!(params[2], ParamValue::Number(-0.497076));
    }

    #[test]
    fn missing_parameters_use_defaults() {
        let filter = FilterStack::parse("HDR=1;").0.remove(0);
        let params = read_params(spec("HDR").unwrap(), &filter);
        assert_eq!(params, [ParamValue::Number(20.0), ParamValue::Number(3.0), ParamValue::Number(0.0)]);
    }

    #[test]
    fn frames_grow_the_canvas() {
        let img = Img::filled(100, 80, [0.5; 4]);
        let out = render(spec("Border").unwrap(), img.clone(), &defaults(spec("Border").unwrap()), 1.0);
        assert_eq!((out.width, out.height), (150, 130), "20 + 5 pixels on each side");
        let shadow = render(spec("DropShadow").unwrap(), img, &defaults(spec("DropShadow").unwrap()), 1.0);
        assert!(shadow.width > 100 && shadow.height > 80);
    }

    #[test]
    fn picasa_effects_change_the_image() {
        // Each of Picasa's own effects must visibly change a mid-tone, colorful
        // image with its default settings; this caught a lost match arm once.
        let mut img = Img::filled(64, 48, [0.6, 0.45, 0.3, 1.0]);
        img.map_xy(|x, y, p| if (x / 8 + y / 8) % 2 == 0 { p } else { [0.2, 0.5, 0.7, 1.0] });
        for spec in SPECS.iter().filter(|s| s.group == Group::Picasa) {
            if ["sepia", "bw", "warm", "sat", "unsharp2"].contains(&spec.id) {
                continue; // rendered by `adjust`, tested there
            }
            let mut params = defaults(spec);
            // Make center-based effects reach the sample pixel in the corner.
            for p in &mut params {
                if let ParamValue::Point(..) = p {
                    *p = ParamValue::Point(0.9, 0.9);
                }
            }
            let out = render(spec, img.clone(), &params, 1.0);
            let diff: f32 = out.px.iter().zip(&img.px).map(|(a, b)| (a[0] - b[0]).abs() + (a[2] - b[2]).abs()).sum();
            assert!(diff > 1.0, "{} did nothing", spec.id);
        }
    }

    #[test]
    fn vignette_darkens_corners() {
        let img = Img::filled(200, 120, [0.8, 0.8, 0.8, 1.0]);
        let out = render(spec("Vignette").unwrap(), img, &defaults(spec("Vignette").unwrap()), 1.0);
        assert!(out.at(0, 0)[0] < out.at(100, 60)[0] - 0.2);
    }
}
