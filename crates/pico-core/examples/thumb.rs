//! Renders a thumbnail with the photo's Picasa edits applied, for checking
//! rendering by eye: `cargo run -p pico-core --example thumb -- IN OUT [SIZE]`.

use pico_core::ini::PicasaIni;
use pico_core::media::MediaKind;
use pico_core::meta::PhotoMeta;
use pico_core::thumbnail;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(input), Some(output)) = (args.next().map(PathBuf::from), args.next().map(PathBuf::from)) else {
        eprintln!("usage: thumb IN OUT [SIZE]");
        std::process::exit(2);
    };
    let size = args.next().and_then(|s| s.to_str()?.parse().ok()).unwrap_or(thumbnail::DEFAULT_SIZE);
    let kind = MediaKind::of(&input).expect("unsupported file type");
    let name = input.file_name().unwrap().to_string_lossy().into_owned();
    let meta = PicasaIni::load_for_folder(input.parent().unwrap())
        .ok()
        .flatten()
        .and_then(|(_, ini)| ini.section(&name).map(PhotoMeta::from_section))
        .unwrap_or_default();
    let thumb = thumbnail::generate(&input, kind, &meta, size).expect("render");
    thumb.save(&output).expect("save");
    println!("{}x{} rotate={} filters={}", thumb.width(), thumb.height(), meta.rotate, meta.filters);
}
