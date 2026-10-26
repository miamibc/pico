//! Thumbnails with a persistent disk cache.
//!
//! Cached files live in the platform cache directory, named by a hash of
//! everything that affects the picture: path, size and modification time of
//! the file, the requested size, and the edits from `.picasa.ini`. Changing any
//! of them simply produces a new entry.

use crate::config;
use crate::media::MediaKind;
use crate::meta::PhotoMeta;
use crate::render;
use image::{DynamicImage, ImageFormat, RgbaImage};
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Picasa's "normal" thumbnails are about this size; the grid scales them down.
pub const DEFAULT_SIZE: u32 = 256;

/// Bump when thumbnail rendering changes, to ignore stale cache entries.
const CACHE_VERSION: u32 = 3;

/// Renders a thumbnail without touching the cache.
pub fn generate(path: &Path, kind: MediaKind, meta: &PhotoMeta, max_side: u32) -> io::Result<RgbaImage> {
    let img = render::decode(path, kind, meta)?;
    let img = render::apply_edits(img, meta);
    Ok(img.thumbnail(max_side, max_side).to_rgba8())
}

/// Returns the cached thumbnail or renders and caches a new one.
/// Cache failures are not errors: the thumbnail is still returned.
pub fn cached(path: &Path, kind: MediaKind, meta: &PhotoMeta, max_side: u32) -> io::Result<RgbaImage> {
    match config::cache_dir() {
        Some(root) => cached_in(&root.join("thumbs"), path, kind, meta, max_side),
        None => generate(path, kind, meta, max_side),
    }
}

fn cached_in(root: &Path, path: &Path, kind: MediaKind, meta: &PhotoMeta, max_side: u32) -> io::Result<RgbaImage> {
    let Some(base) = cache_path(root, path, meta, max_side) else {
        return generate(path, kind, meta, max_side);
    };
    for format in [ImageFormat::Jpeg, ImageFormat::Png] {
        let file = base.with_extension(format.extensions_str()[0]);
        if let Ok(img) = image::open(&file) {
            return Ok(img.to_rgba8());
        }
    }
    let thumb = generate(path, kind, meta, max_side)?;
    let _ = store(&base, &thumb);
    Ok(thumb)
}

fn store(base: &Path, thumb: &RgbaImage) -> io::Result<()> {
    std::fs::create_dir_all(base.parent().unwrap_or(Path::new(".")))?;
    // JPEG is several times smaller; keep PNG only where transparency matters.
    let opaque = thumb.pixels().all(|p| p.0[3] == u8::MAX);
    let (file, result) = if opaque {
        let file = base.with_extension("jpg");
        let rgb = DynamicImage::ImageRgba8(thumb.clone()).to_rgb8();
        let result = rgb.save_with_format(&file, ImageFormat::Jpeg);
        (file, result)
    } else {
        let file = base.with_extension("png");
        let result = thumb.save_with_format(&file, ImageFormat::Png);
        (file, result)
    };
    result.map_err(|e| {
        let _ = std::fs::remove_file(&file);
        io::Error::other(e)
    })
}

fn cache_path(root: &Path, path: &Path, meta: &PhotoMeta, max_side: u32) -> Option<PathBuf> {
    let stat = std::fs::metadata(path).ok()?;
    let modified = stat.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    let mut hash = Fnv64::default();
    hash.write(path.as_os_str().as_encoded_bytes());
    hash.write(&stat.len().to_le_bytes());
    hash.write(&modified.to_le_bytes());
    hash.write(&max_side.to_le_bytes());
    hash.write(&CACHE_VERSION.to_le_bytes());
    hash.write(&[meta.rotate]);
    hash.write(meta.filters.to_string().as_bytes());
    let key = format!("{:016x}", hash.0);
    Some(root.join(&key[..2]).join(key))
}

/// FNV-1a: tiny and, unlike `DefaultHasher`, stable across Rust releases.
struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv64 {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pico-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn caches_by_content_and_edits() {
        let dir = temp_dir("thumbs");
        let photo = dir.join("photo.png");
        RgbaImage::from_pixel(400, 200, image::Rgba([10, 20, 30, 255])).save(&photo).unwrap();
        let cache = dir.join("cache");

        let plain = cached_in(&cache, &photo, MediaKind::Photo, &PhotoMeta::default(), 100).unwrap();
        assert_eq!(plain.dimensions(), (100, 50));
        let entries = || walk(&cache);
        assert_eq!(entries().len(), 1);
        assert!(entries()[0].extension().is_some_and(|e| e == "jpg"), "opaque thumbnails are JPEG");

        // Served from the cache: same size, no new entry.
        assert_eq!(
            cached_in(&cache, &photo, MediaKind::Photo, &PhotoMeta::default(), 100).unwrap().dimensions(),
            (100, 50)
        );
        assert_eq!(entries().len(), 1);

        // Different edits are a different entry.
        let rotated = PhotoMeta { rotate: 1, ..Default::default() };
        assert_eq!(cached_in(&cache, &photo, MediaKind::Photo, &rotated, 100).unwrap().dimensions(), (50, 100));
        assert_eq!(entries().len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn walk(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
        entries.flatten().flat_map(|e| if e.path().is_dir() { walk(&e.path()) } else { vec![e.path()] }).collect()
    }
}
