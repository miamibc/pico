//! Renders every collage layout from the given photos into one overview image:
//! `cargo run --release -p pico-core --example collage -- OUT.png PHOTO...`.

use image::{GenericImage, Rgba, RgbaImage};
use pico_core::collage::{self, Border, CollageOptions, Item, Layout};
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let out = args.next().expect("usage: collage OUT.png PHOTO...");
    let items: Vec<Item> = args
        .filter_map(|path| {
            let kind = MediaKind::of(&path)?;
            let file = MediaFile::new(path, kind);
            let image = pico_core::export::render_full(&file).ok()?.thumbnail(1200, 1200);
            Some(Item { image, name: file.name(), caption: Some(file.name()) })
        })
        .collect();
    let (cw, ch) = (900, 600);
    let mut sheet = RgbaImage::from_pixel(cw * 3 + 40, ch * 2 + 30, Rgba([40, 40, 40, 255]));
    for (i, (layout, name)) in Layout::ALL.iter().enumerate() {
        let border = if *layout == Layout::Mosaic { Border::InstantCamera } else { Border::White };
        let options = CollageOptions {
            layout: *layout,
            width: cw,
            aspect: 1.5,
            border,
            captions: true,
            background: collage::Background::Color(0xeeeae4),
            title: "Test library".into(),
            ..Default::default()
        };
        let refs: Vec<&Item> = items.iter().collect();
        let page = collage::render(&refs, &options);
        sheet.copy_from(&page, 10 + (i as u32 % 3) * (cw + 10), 10 + (i as u32 / 3) * (ch + 10)).unwrap();
        println!("{name}");
    }
    sheet.save(out).unwrap();
}
