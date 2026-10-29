//! Prints what the Properties panel shows for a photo:
//! `cargo run -p pico-core --example exif -- PHOTO`.

fn main() {
    for path in std::env::args_os().skip(1) {
        let path = std::path::PathBuf::from(path);
        println!("{}", path.display());
        match pico_core::exif::read(&path) {
            Some(exif) => {
                for (label, value) in exif.fields {
                    println!("  {label:>14}: {value}");
                }
                println!("  All tags:");
                for (label, value) in &exif.all {
                    println!("    {label}: {value}");
                }
                let xmp = pico_core::xmp::read(&path);
                for (_, label, note) in pico_core::agree::disagreements(&[("EXIF", &exif.all), ("XMP", &xmp)]) {
                    println!("  ! {label}: {}", note.replace('\n', "; "));
                }
            }
            None => println!("  (no EXIF)"),
        }
    }
}
