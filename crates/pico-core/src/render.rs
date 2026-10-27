//! Turning a media file plus its Picasa edits into pixels.

use crate::adjust;
use crate::media::MediaKind;
use crate::meta::PhotoMeta;
use crate::video;
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::io;
use std::path::Path;
use std::time::Duration;

/// Decodes a file at full size, upright according to its EXIF orientation.
/// For videos this is a frame from the trim start, or a second in.
pub fn decode(path: &Path, kind: MediaKind, meta: &PhotoMeta) -> io::Result<DynamicImage> {
    match kind {
        MediaKind::Photo => {
            let mut decoder =
                ImageReader::open(path)?.with_guessed_format()?.into_decoder().map_err(io::Error::other)?;
            // A broken orientation tag must not hide the photo.
            let orientation = decoder.orientation().ok();
            let mut img = DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
            if let Some(orientation) = orientation {
                img.apply_orientation(orientation);
            }
            Ok(img)
        }
        // A second in skips black intro frames; short clips fall back to the start.
        MediaKind::Video => video::extract_frame(path, meta.video_start().unwrap_or(Duration::from_secs(1)), None)
            .or_else(|_| video::extract_frame(path, Duration::ZERO, None))
            .map(DynamicImage::ImageRgba8),
        MediaKind::RawPhoto => video::extract_frame(path, Duration::ZERO, None).map(DynamicImage::ImageRgba8),
    }
}

/// Pixel counts per level (0–255) for red, green and blue.
pub type Histogram = [[u32; 256]; 3];

pub fn histogram(img: &image::RgbaImage) -> Box<Histogram> {
    let mut counts = Box::new([[0u32; 256]; 3]);
    for pixel in img.pixels() {
        for (channel, &value) in pixel.0[..3].iter().enumerate() {
            counts[channel][value as usize] += 1;
        }
    }
    counts
}

/// Applies the edit stack in order, then the quarter-turn rotation, which
/// Picasa keeps outside the stack. Filters Pico cannot render are skipped.
pub fn apply_edits(img: DynamicImage, meta: &PhotoMeta) -> DynamicImage {
    apply_edits_scaled(img, meta, 1.0)
}

/// Like [`apply_edits`] on a copy that is `scale` times the original size.
pub fn apply_edits_scaled(img: DynamicImage, meta: &PhotoMeta, scale: f32) -> DynamicImage {
    let img = meta.filters.0.iter().fold(img, |img, filter| adjust::apply(img, filter, scale));
    match meta.rotate % 4 {
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filters::FilterStack;
    use crate::rect64::Rect64;

    #[test]
    fn crops_then_rotates() {
        let img = DynamicImage::new_rgb8(200, 100);
        let crop = Rect64::from_unit([0.0, 0.0, 0.5, 1.0]);
        let meta =
            PhotoMeta { rotate: 1, filters: FilterStack::parse(&format!("crop64=1,{crop};")), ..Default::default() };
        let out = apply_edits(img, &meta);
        assert_eq!((out.width(), out.height()), (100, 100));

        let meta = PhotoMeta { rotate: 1, ..Default::default() };
        let out = apply_edits(DynamicImage::new_rgb8(200, 100), &meta);
        assert_eq!((out.width(), out.height()), (100, 200));
    }
}

#[cfg(test)]
mod histogram_tests {
    use super::*;

    #[test]
    fn counts_each_channel() {
        let mut img = image::RgbaImage::from_pixel(4, 1, image::Rgba([255, 0, 128, 255]));
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let h = histogram(&img);
        assert_eq!((h[0][255], h[0][0]), (3, 1));
        assert_eq!(h[1][0], 4);
        assert_eq!((h[2][128], h[2][0]), (3, 1));
    }
}
