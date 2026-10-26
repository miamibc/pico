//! Renders one photo through each supported filter into a labelled contact
//! sheet, for checking filters by eye:
//! `cargo run -p pico-core --example filter_sheet -- IN OUT`.

use image::{DynamicImage, GenericImage, Rgba, RgbaImage};
use pico_core::effects;
use pico_core::filters::FilterStack;
use pico_core::media::MediaKind;
use pico_core::meta::PhotoMeta;
use pico_core::render;
use std::path::PathBuf;

const CELL: u32 = 320;

const STACKS: &[(&str, &str)] = &[
    ("original", ""),
    ("I'm Feeling Lucky", "enhance=1;"),
    ("Auto Contrast", "autolight=1;"),
    ("Auto Color", "autocolor=1;"),
    ("Fill Light 0.5", "fill=1,0.5;"),
    ("Highlights+Shadows", "finetune2=1,0,0.2,0.15,00000000,0;"),
    ("Warmer -> Cooler", "finetune2=1,0,0,0,00000000,-0.6;"),
    ("B&W", "bw=1;"),
    ("Sepia", "sepia=1;"),
    ("Warmify", "warm=1;"),
    ("Saturation +0.6", "sat=1,0.6;"),
    ("Sharpen 2.0", "unsharp2=1,2.0;"),
];

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: filter_sheet IN OUT");
        std::process::exit(2);
    };
    let kind = MediaKind::of(&input).expect("unsupported file type");
    let base = render::decode(&input, kind, &PhotoMeta::default()).expect("decode").thumbnail(CELL, CELL);
    let columns = 6;
    // Fixed stacks first, then every registered effect with its defaults.
    let all: Vec<(String, String)> = STACKS
        .iter()
        .map(|(l, s)| (l.to_string(), s.to_string()))
        .chain(
            effects::SPECS
                .iter()
                .filter(|s| s.group != effects::Group::Legacy)
                .map(|spec| (spec.label.to_string(), effects::to_filter(spec, &effects::defaults(spec)).to_string())),
        )
        .collect();
    let rows = all.len().div_ceil(columns) as u32;
    let mut sheet = RgbaImage::from_pixel(CELL * columns as u32, CELL * rows, Rgba([24, 24, 24, 255]));
    for (i, (label, stack)) in all.iter().enumerate() {
        let meta = PhotoMeta { filters: FilterStack::parse(stack), ..Default::default() };
        // Frames grow the picture; fit it back into the cell.
        let cell: DynamicImage = render::apply_edits(base.clone(), &meta).thumbnail(CELL, CELL);
        let (x, y) = ((i % columns) as u32 * CELL, (i / columns) as u32 * CELL);
        sheet.copy_from(&cell.to_rgba8(), x, y).expect("fits");
        println!("cell {i}: {label} ({stack})");
    }
    sheet.save(&output).expect("save");
}
