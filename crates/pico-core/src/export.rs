//! Writing edited photos: export to a folder, save a copy, save in place with
//! a backup in `.picasaoriginals`, and revert to that backup — Picasa's model.

use crate::exif;
use crate::ini::{self, PicasaIni};
use crate::library::{Folder, MediaFile};
use crate::media::MediaKind;
use crate::meta::PhotoMeta;
use crate::render;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat};
use std::io;
use std::path::{Path, PathBuf};

/// Where Picasa keeps originals of photos saved with edits.
pub const ORIGINALS_DIR: &str = ".picasaoriginals";

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportOptions {
    /// Longest side in pixels; `None` keeps the original size.
    pub max_side: Option<u32>,
    /// JPEG quality, 1..=100.
    pub quality: u8,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self { max_side: None, quality: 85 }
    }
}

/// Whether a file has edits that saving would bake into its pixels.
pub fn has_edits(meta: &PhotoMeta) -> bool {
    !meta.rotate.is_multiple_of(4)
        || meta.filters.0.iter().any(|f| f.enabled() && !matches!(f.id.as_str(), "moviestart" | "movieend"))
}

/// Renders the photo with its edits at full size.
pub fn render_full(file: &MediaFile) -> io::Result<DynamicImage> {
    let img = render::decode(&file.path, file.kind, &file.meta)?;
    Ok(render::apply_edits(img, &file.meta))
}

/// Exports a photo with its edits as a JPEG into `dir`, never overwriting an
/// existing file. Videos are copied as they are. Returns the written path.
pub fn export(file: &MediaFile, dir: &Path, options: ExportOptions) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    if file.kind == MediaKind::Video {
        let target = free_name(dir, &file.name());
        std::fs::copy(&file.path, &target)?;
        return Ok(target);
    }
    let stem = file.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
    let target = free_name(dir, &format!("{stem}.jpg"));
    let mut img = render_full(file)?;
    if let Some(side) = options.max_side
        && img.width().max(img.height()) > side
    {
        img = img.resize(side, side, image::imageops::FilterType::Lanczos3);
    }
    write_jpeg(&img, &file.path, &target, options.quality)?;
    Ok(target)
}

/// Writes the edited photo next to the original as `name-1.jpg` (or the next
/// free number), like Picasa's "Save a Copy".
pub fn save_copy(file: &MediaFile) -> io::Result<PathBuf> {
    let dir = file.path.parent().unwrap_or(Path::new("."));
    let stem = file.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let target = (1..).map(|n| dir.join(format!("{stem}-{n}.jpg"))).find(|p| !p.exists()).expect("a free name");
    write_jpeg(&render_full(file)?, &file.path, &target, 92)?;
    Ok(target)
}

/// Bakes the edits into the file itself. The untouched original moves to
/// `.picasaoriginals/` first (unless a backup already exists), and its edit
/// stack is remembered in that folder's `.picasa.ini` so [`revert`] can bring
/// both back. The photo's own edits are then cleared.
pub fn save_in_place(folder: &mut Folder, index: usize) -> io::Result<()> {
    let file = folder.files[index].clone();
    if file.kind == MediaKind::Video || !has_edits(&file.meta) {
        return Ok(());
    }
    let rendered = render_full(&file)?;
    let originals = folder.path.join(ORIGINALS_DIR);
    std::fs::create_dir_all(&originals)?;
    let backup = originals.join(file.name());
    let backup_ini_path = originals.join(ini::FILE_NAME);
    let mut backup_ini = PicasaIni::load(&backup_ini_path).unwrap_or_default();
    if !backup.exists() {
        std::fs::copy(&file.path, &backup)?;
    }
    // Remember the edits that are now baked in, on top of any saved earlier.
    let section = backup_ini.section_mut(&file.name());
    let mut previous = PhotoMeta::from_section(section);
    previous.filters.0.extend(file.meta.filters.0.iter().cloned());
    previous.rotate = (previous.rotate + file.meta.rotate) % 4;
    let edits = PhotoMeta { filters: previous.filters, rotate: previous.rotate, ..Default::default() };
    edits.write_to(section);
    backup_ini.save(&backup_ini_path)?;

    let format = ImageFormat::from_path(&file.path).unwrap_or(ImageFormat::Jpeg);
    let temp = file.path.with_extension("pico-saving");
    let written = if format == ImageFormat::Jpeg {
        write_jpeg(&rendered, &backup, &temp, 92)
    } else {
        rendered.save_with_format(&temp, format).map_err(io::Error::other)
    };
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    std::fs::rename(&temp, &file.path)?;
    let meta = &mut folder.files[index].meta;
    meta.filters.0.clear();
    meta.rotate = 0;
    folder.save_meta(index)
}

/// Whether a saved original exists for this file.
pub fn has_backup(folder: &Folder, index: usize) -> bool {
    folder.path.join(ORIGINALS_DIR).join(folder.files[index].name()).is_file()
}

/// Restores the original from `.picasaoriginals/`, together with the edits
/// that were baked in, so they show again as editable.
pub fn revert(folder: &mut Folder, index: usize) -> io::Result<()> {
    let name = folder.files[index].name();
    let originals = folder.path.join(ORIGINALS_DIR);
    let backup = originals.join(&name);
    if !backup.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "no saved original"));
    }
    let backup_ini_path = originals.join(ini::FILE_NAME);
    let mut backup_ini = PicasaIni::load(&backup_ini_path).unwrap_or_default();
    let saved = backup_ini.section(&name).map(PhotoMeta::from_section).unwrap_or_default();
    std::fs::rename(&backup, &folder.files[index].path)?;
    backup_ini.sections.retain(|s| s.name != name);
    if backup_ini.sections.is_empty() {
        let _ = std::fs::remove_file(&backup_ini_path);
        let _ = std::fs::remove_dir(&originals);
    } else {
        backup_ini.save(&backup_ini_path)?;
    }
    let meta = &mut folder.files[index].meta;
    let mut restored = saved.filters;
    restored.0.append(&mut meta.filters.0);
    meta.filters = restored;
    meta.rotate = (saved.rotate + meta.rotate) % 4;
    folder.save_meta(index)
}

fn write_jpeg(img: &DynamicImage, exif_from: &Path, target: &Path, quality: u8) -> io::Result<()> {
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(1, 100))
        .encode_image(&DynamicImage::ImageRgb8(img.to_rgb8()))
        .map_err(io::Error::other)?;
    let source = std::fs::read(exif_from).unwrap_or_default();
    if let Some(mut segment) = exif::exif_segment(&source) {
        exif::normalize(&mut segment);
        jpeg = exif::insert(&jpeg, &segment);
    }
    std::fs::write(target, jpeg)
}

fn free_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_owned(), format!(".{e}")),
        None => (name.to_owned(), String::new()),
    };
    (1..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|p| !p.exists()).expect("a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterStack;
    use crate::library;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pico-test-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A 40x20 JPEG whose EXIF says "rotate 90° clockwise to view".
    fn camera_jpeg(path: &Path) {
        let mut jpeg = Vec::new();
        image::RgbImage::from_pixel(40, 20, image::Rgb([200, 30, 30]))
            .write_to(&mut std::io::Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .unwrap();
        std::fs::write(path, exif::insert(&jpeg, &exif::tests::sample_segment(6))).unwrap();
    }

    #[test]
    fn export_bakes_edits_keeps_exif_and_resizes() {
        let dir = temp_dir("export");
        camera_jpeg(&dir.join("cam.jpg"));
        let mut folders = library::scan(&dir);
        let file = &mut folders[0].files[0];
        file.meta.filters = FilterStack::parse("bw=1;");
        let out = export(file, &dir.join("out"), ExportOptions { max_side: Some(10), quality: 90 }).unwrap();
        let img = image::open(&out).unwrap();
        // EXIF 6 made it 20x40 upright; then it was scaled to fit 10.
        assert_eq!((img.width(), img.height()), (5, 10));
        let p = img.to_rgb8().get_pixel(2, 5).0;
        assert!(p[0].abs_diff(p[1]) < 12, "B&W baked in: {p:?}");
        let bytes = std::fs::read(&out).unwrap();
        let segment = exif::exif_segment(&bytes).expect("EXIF kept");
        assert!(segment.windows(5).any(|w| w == b"Pico\0"));
        // Viewers must not rotate the already upright pixels again.
        let reopened = render::decode(&out, MediaKind::Photo, &PhotoMeta::default()).unwrap();
        assert_eq!((reopened.width(), reopened.height()), (5, 10));
        // A second export does not overwrite the first.
        let again = export(file, &dir.join("out"), ExportOptions::default()).unwrap();
        assert_eq!(again.file_name().unwrap(), "cam (1).jpg");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_backs_up_and_revert_restores() {
        let dir = temp_dir("save");
        camera_jpeg(&dir.join("cam.jpg"));
        let original = std::fs::read(dir.join("cam.jpg")).unwrap();
        let mut folders = library::scan(&dir);
        let folder = &mut folders[0];
        folder.files[0].meta.filters = FilterStack::parse("bw=1;");
        folder.files[0].meta.rotate = 1;
        folder.save_meta(0).unwrap();

        save_in_place(folder, 0).unwrap();
        assert_eq!(std::fs::read(dir.join(".picasaoriginals/cam.jpg")).unwrap(), original);
        let backup_ini = std::fs::read_to_string(dir.join(".picasaoriginals/.picasa.ini")).unwrap();
        assert!(backup_ini.contains("filters=bw=1;") && backup_ini.contains("rotate=rotate(1)"), "{backup_ini}");
        let saved = image::open(dir.join("cam.jpg")).unwrap();
        assert_eq!((saved.width(), saved.height()), (40, 20), "EXIF 6 plus a quarter turn = upside-down landscape");
        assert!(folder.files[0].meta.filters.is_empty() && folder.files[0].meta.rotate == 0);
        assert!(has_backup(folder, 0));

        revert(folder, 0).unwrap();
        assert_eq!(std::fs::read(dir.join("cam.jpg")).unwrap(), original);
        assert_eq!(folder.files[0].meta.filters.to_string(), "bw=1;");
        assert_eq!(folder.files[0].meta.rotate, 1);
        assert!(!dir.join(".picasaoriginals").exists(), "an empty backup folder is removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
