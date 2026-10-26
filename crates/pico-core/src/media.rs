//! Which files Pico shows, by extension.
//!
//! Picasa 3 handled a fixed list; Pico accepts a superset. Photos are decoded
//! with the `image` crate, everything else goes through ffmpeg.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// Decoded natively.
    Photo,
    /// Camera RAW and photo formats the `image` crate cannot read (AVIF, HEIC,
    /// JPEG XL, PSD); decoded through ffmpeg for now.
    RawPhoto,
    Video,
}

const PHOTO: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "dib", "tif", "tiff", "webp", "tga", "ico", "pnm", "pbm", "pgm",
    "ppm", "qoi", "hdr", "exr", "dds", "ff",
];

const RAW_PHOTO: &[&str] = &[
    "3fr", "ari", "arw", "bay", "cr2", "cr3", "crw", "dcr", "dng", "erf", "fff", "iiq", "k25", "kdc", "mef", "mos",
    "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw", "x3f", "avif", "heic", "heif",
    "jxl", "psd",
];

const VIDEO: &[&str] = &[
    "3g2", "3gp", "asf", "avi", "divx", "dv", "f4v", "flv", "m2t", "m2ts", "m4v", "mkv", "mmv", "mod", "mov", "mp4",
    "mpe", "mpeg", "mpg", "mts", "mxf", "ogv", "qt", "rm", "rmvb", "tod", "ts", "vob", "webm", "wmv",
];

impl MediaKind {
    pub fn of(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        let ext = ext.as_str();
        if PHOTO.contains(&ext) {
            Some(Self::Photo)
        } else if RAW_PHOTO.contains(&ext) {
            Some(Self::RawPhoto)
        } else if VIDEO.contains(&ext) {
            Some(Self::Video)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_extension_case_insensitively() {
        assert_eq!(MediaKind::of(Path::new("a/_Z7A0047.JPG")), Some(MediaKind::Photo));
        assert_eq!(MediaKind::of(Path::new("clip.MTS")), Some(MediaKind::Video));
        assert_eq!(MediaKind::of(Path::new("raw.cr2")), Some(MediaKind::RawPhoto));
        assert_eq!(MediaKind::of(Path::new(".picasa.ini")), None);
    }
}
