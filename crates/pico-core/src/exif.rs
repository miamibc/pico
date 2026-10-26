//! Carrying EXIF metadata over when Pico writes a JPEG.
//!
//! The `image` crate writes JPEGs without EXIF, so the original APP1 segment is
//! copied into the new file. Because the edits (including rotation) are baked
//! into the pixels, the Orientation tag is reset to "normal", and the embedded
//! preview thumbnail (IFD1), which would show the unedited photo, is dropped.

const SOI: [u8; 2] = [0xff, 0xd8];
const APP1: u8 = 0xe1;
const EXIF_HEADER: &[u8] = b"Exif\0\0";
const ORIENTATION: u16 = 0x0112;

/// The complete APP1 EXIF segment (marker and length included), if any.
pub fn exif_segment(jpeg: &[u8]) -> Option<Vec<u8>> {
    if !jpeg.starts_with(&SOI) {
        return None;
    }
    let mut pos = 2;
    while pos + 4 <= jpeg.len() && jpeg[pos] == 0xff {
        let marker = jpeg[pos + 1];
        // Metadata segments come before the image data; stop at SOS.
        if marker == 0xda {
            return None;
        }
        let len = u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]) as usize;
        let end = pos + 2 + len;
        if len < 2 || end > jpeg.len() {
            return None;
        }
        if marker == APP1 && jpeg[pos + 4..end].starts_with(EXIF_HEADER) {
            return Some(jpeg[pos..end].to_vec());
        }
        pos = end;
    }
    None
}

/// Resets Orientation to 1 and unlinks the thumbnail IFD, in place.
pub fn normalize(segment: &mut [u8]) {
    let tiff_start = 4 + EXIF_HEADER.len();
    if segment.len() < tiff_start + 8 {
        return;
    }
    let tiff = &mut segment[tiff_start..];
    let little = match &tiff[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return,
    };
    let read16 = |b: &[u8], at: usize| {
        let v = [b[at], b[at + 1]];
        if little { u16::from_le_bytes(v) } else { u16::from_be_bytes(v) }
    };
    let read32 = |b: &[u8], at: usize| {
        let v = [b[at], b[at + 1], b[at + 2], b[at + 3]];
        if little { u32::from_le_bytes(v) } else { u32::from_be_bytes(v) }
    };
    let ifd0 = read32(tiff, 4) as usize;
    if ifd0 + 2 > tiff.len() {
        return;
    }
    let count = read16(tiff, ifd0) as usize;
    let entries_end = ifd0 + 2 + count * 12;
    if entries_end + 4 > tiff.len() {
        return;
    }
    for i in 0..count {
        let entry = ifd0 + 2 + i * 12;
        if read16(tiff, entry) == ORIENTATION {
            // SHORT value stored in the first two bytes of the value field.
            let one = if little { 1u16.to_le_bytes() } else { 1u16.to_be_bytes() };
            tiff[entry + 8..entry + 10].copy_from_slice(&one);
        }
    }
    // Next-IFD offset 0: no IFD1, so no stale thumbnail.
    tiff[entries_end..entries_end + 4].copy_from_slice(&[0; 4]);
}

/// Inserts an APP1 segment right after the SOI (and a JFIF APP0, if present).
pub fn insert(jpeg: &[u8], segment: &[u8]) -> Vec<u8> {
    if !jpeg.starts_with(&SOI) {
        return jpeg.to_vec();
    }
    let mut at = 2;
    if jpeg.len() > 4 && jpeg[2] == 0xff && jpeg[3] == 0xe0 {
        at = 4 + u16::from_be_bytes([jpeg[4], jpeg[5]]) as usize;
    }
    let mut out = Vec::with_capacity(jpeg.len() + segment.len());
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(segment);
    out.extend_from_slice(&jpeg[at..]);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal little-endian EXIF APP1 with Orientation and a Make tag, plus
    /// a link to an IFD1.
    pub(crate) fn sample_segment(orientation: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes()); // two entries
        // Make (0x010f), ASCII, 5 bytes at offset 38.
        tiff.extend_from_slice(&0x010fu16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&5u32.to_le_bytes());
        tiff.extend_from_slice(&38u32.to_le_bytes());
        // Orientation, SHORT, 1 value.
        tiff.extend_from_slice(&ORIENTATION.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&orientation.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&38u32.to_le_bytes()); // "next IFD" (pretend IFD1)
        tiff.extend_from_slice(b"Pico\0");
        let mut segment = vec![0xff, APP1];
        let len = (2 + EXIF_HEADER.len() + tiff.len()) as u16;
        segment.extend_from_slice(&len.to_be_bytes());
        segment.extend_from_slice(EXIF_HEADER);
        segment.extend_from_slice(&tiff);
        segment
    }

    #[test]
    fn finds_normalizes_and_inserts() {
        let jpeg_without = {
            let mut buf = Vec::new();
            image::RgbImage::new(8, 8).write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Jpeg).unwrap();
            buf
        };
        assert!(exif_segment(&jpeg_without).is_none());
        let with = insert(&jpeg_without, &sample_segment(6));
        let mut segment = exif_segment(&with).expect("inserted");
        assert_eq!(segment, sample_segment(6));
        normalize(&mut segment);
        let orientation_at = 4 + 6 + 8 + 2 + 12 + 8;
        assert_eq!(&segment[orientation_at..orientation_at + 2], &[1, 0]);
        assert!(segment.windows(5).any(|w| w == b"Pico\0"), "other tags survive");
        // The result still decodes as an image.
        assert!(image::load_from_memory(&insert(&jpeg_without, &segment)).is_ok());
    }
}

/// Camera details read from a photo's EXIF block, for the Properties panel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exif {
    /// When the photo was taken, as written by the camera: `YYYY:MM:DD HH:MM:SS`
    /// (which also sorts correctly as text).
    pub taken: Option<String>,
    /// Latitude and longitude in degrees (south and west negative).
    pub gps: Option<(f64, f64)>,
    /// Human-readable `(label, value)` pairs in display order.
    pub fields: Vec<(&'static str, String)>,
}

/// Reads EXIF from a JPEG's first segments. Returns `None` when there is none.
pub fn read(path: &std::path::Path) -> Option<Exif> {
    use std::io::Read;
    let mut head = Vec::new();
    // EXIF sits at the start; 256 KiB covers it even with large previews.
    std::fs::File::open(path).ok()?.take(256 * 1024).read_to_end(&mut head).ok()?;
    let segment = exif_segment(&head)?;
    parse(&segment[4 + EXIF_HEADER.len()..])
}

/// Parses a TIFF-structured EXIF block.
pub fn parse(tiff: &[u8]) -> Option<Exif> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let reader = Tiff { data: tiff, little };
    let mut tags = std::collections::BTreeMap::new();
    let ifd0 = reader.u32(4)? as usize;
    let mut subs = Vec::new();
    reader.read_ifd(ifd0, &mut tags, &mut subs);
    for (tag, offset) in subs {
        let mut nested = Vec::new();
        let mut found = std::collections::BTreeMap::new();
        reader.read_ifd(offset, &mut found, &mut nested);
        for (t, v) in found {
            // GPS tags reuse small numbers; keep them apart.
            tags.insert(if tag == 0x8825 { 0x1_0000 + t } else { t }, v);
        }
    }
    let text = |t: u32| match tags.get(&t) {
        Some(Value::Text(s)) => Some(s.clone()),
        _ => None,
    };
    let rational = |t: u32| match tags.get(&t) {
        Some(Value::Rationals(r)) => r.first().copied(),
        Some(Value::Numbers(n)) => n.first().map(|&v| (v as f64, 1.0)),
        _ => None,
    };
    let number = |t: u32| match tags.get(&t) {
        Some(Value::Numbers(n)) => n.first().copied(),
        _ => None,
    };
    let mut exif = Exif { taken: text(0x9003).or_else(|| text(0x0132)), gps: None, fields: Vec::new() };
    let mut push = |label: &'static str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            exif.fields.push((label, v.trim().to_owned()));
        }
    };
    let camera = match (text(0x010f), text(0x0110)) {
        // Many models already start with the make ("Canon Canon EOS…").
        (Some(make), Some(model)) if model.starts_with(make.split_whitespace().next().unwrap_or("")) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (make, model) => make.or(model),
    };
    push("Camera", camera);
    push("Lens", text(0xa434));
    push("Date taken", exif.taken.clone());
    push(
        "Exposure",
        rational(0x829a)
            .map(|(n, d)| if n > 0.0 && n < d { format!("1/{:.0} s", d / n) } else { format!("{:.1} s", n / d) }),
    );
    push("Aperture", rational(0x829d).map(|(n, d)| format!("f/{:.1}", n / d)));
    push("ISO", number(0x8827).map(|v| v.to_string()));
    push("Focal length", rational(0x920a).map(|(n, d)| format!("{:.0} mm", n / d)));
    push("Flash", number(0x9209).map(|v| if v & 1 == 1 { "Fired".to_owned() } else { "Did not fire".to_owned() }));
    push("Size (camera)", number(0xa002).zip(number(0xa003)).map(|(w, h)| format!("{w} × {h}")));
    push("Software", text(0x0131));
    let coordinate = |value: u32, reference: u32| {
        let Some(Value::Rationals(r)) = tags.get(&(0x1_0000 + value)) else { return None };
        let [deg, min, sec] = [r.first()?, r.get(1)?, r.get(2)?].map(|(n, d)| if *d == 0.0 { 0.0 } else { n / d });
        let sign = if matches!(text(0x1_0000 + reference).as_deref(), Some("S" | "W")) { -1.0 } else { 1.0 };
        Some(sign * (deg + min / 60.0 + sec / 3600.0))
    };
    let gps = coordinate(2, 1).zip(coordinate(4, 3)).filter(|(lat, lon)| lat.abs() <= 90.0 && lon.abs() <= 180.0);
    push("GPS", gps.map(|(lat, lon)| format!("{lat:.6}, {lon:.6}")));
    exif.gps = gps;
    Some(exif)
}

#[derive(Debug, Clone)]
enum Value {
    Text(String),
    Numbers(Vec<u32>),
    Rationals(Vec<(f64, f64)>),
}

struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = [*self.data.get(at)?, *self.data.get(at + 1)?];
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = [*self.data.get(at)?, *self.data.get(at + 1)?, *self.data.get(at + 2)?, *self.data.get(at + 3)?];
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// Reads one IFD's entries; pointers to the EXIF and GPS IFDs go to `subs`.
    fn read_ifd(&self, at: usize, tags: &mut std::collections::BTreeMap<u32, Value>, subs: &mut Vec<(u16, usize)>) {
        let Some(count) = self.u16(at) else { return };
        for i in 0..count as usize {
            let entry = at + 2 + i * 12;
            let (Some(tag), Some(kind), Some(n)) = (self.u16(entry), self.u16(entry + 2), self.u32(entry + 4)) else {
                return;
            };
            let n = n as usize;
            let size: usize = match kind {
                1 | 2 | 7 => 1,
                3 => 2,
                4 | 9 => 4,
                5 | 10 => 8,
                _ => continue,
            };
            let Some(total) = size.checked_mul(n) else { continue };
            let start = if total <= 4 { entry + 8 } else { self.u32(entry + 8).unwrap_or(0) as usize };
            if start + total > self.data.len() {
                continue;
            }
            if tag == 0x8769 || tag == 0x8825 {
                if let Some(offset) = self.u32(entry + 8) {
                    subs.push((tag, offset as usize));
                }
                continue;
            }
            let value = match kind {
                2 => {
                    let bytes = &self.data[start..start + total];
                    let text = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
                    Value::Text(String::from_utf8_lossy(text).into_owned())
                }
                3 => Value::Numbers((0..n).filter_map(|k| self.u16(start + 2 * k).map(u32::from)).collect()),
                4 | 9 => Value::Numbers((0..n).filter_map(|k| self.u32(start + 4 * k)).collect()),
                5 | 10 => Value::Rationals(
                    (0..n)
                        .filter_map(|k| {
                            let (num, den) = (self.u32(start + 8 * k)?, self.u32(start + 8 * k + 4)?);
                            Some(if kind == 10 {
                                (num as i32 as f64, den as i32 as f64)
                            } else {
                                (num as f64, den as f64)
                            })
                        })
                        .collect(),
                ),
                _ => Value::Numbers(self.data[start..start + total].iter().map(|&b| b as u32).collect()),
            };
            tags.insert(tag as u32, value);
        }
    }
}

#[cfg(test)]
pub(crate) mod read_tests {
    use super::*;

    /// A big-endian EXIF TIFF block: Canon EOS 5D, taken 2009-07-14 18:30:05,
    /// 1/250 s, f/2.8, ISO 400, at 59°26' N 24°45' E (Tallinn).
    pub(crate) fn sample_tiff() -> Vec<u8> {
        let mut t = Vec::new();
        let be16 = |v: u16| v.to_be_bytes();
        let be32 = |v: u32| v.to_be_bytes();
        t.extend_from_slice(b"MM");
        t.extend_from_slice(&be16(42));
        t.extend_from_slice(&be32(8));
        // IFD0 at 8: 4 entries.
        let ifd0 = |t: &mut Vec<u8>, tag: u16, kind: u16, n: u32, value: u32| {
            t.extend_from_slice(&be16(tag));
            t.extend_from_slice(&be16(kind));
            t.extend_from_slice(&be32(n));
            t.extend_from_slice(&be32(value));
        };
        t.extend_from_slice(&be16(4));
        let data = 8 + 2 + 4 * 12 + 4; // after IFD0 and its next pointer
        ifd0(&mut t, 0x010f, 2, 6, data as u32); // "Canon\0"
        ifd0(&mut t, 0x0110, 2, 13, data as u32 + 6); // "Canon EOS 5D\0"
        let exif_at = data + 6 + 13;
        ifd0(&mut t, 0x8769, 4, 1, exif_at as u32);
        let gps_at = exif_at + 2 + 4 * 12 + 4 + 20 + 16;
        ifd0(&mut t, 0x8825, 4, 1, gps_at as u32);
        t.extend_from_slice(&be32(0));
        t.extend_from_slice(b"Canon\0Canon EOS 5D\0");
        // EXIF IFD.
        let values = exif_at + 2 + 4 * 12 + 4;
        t.extend_from_slice(&be16(4));
        ifd0(&mut t, 0x9003, 2, 20, values as u32);
        ifd0(&mut t, 0x829a, 5, 1, values as u32 + 20);
        ifd0(&mut t, 0x829d, 5, 1, values as u32 + 28);
        t.extend_from_slice(&be16(0x8827));
        t.extend_from_slice(&be16(3));
        t.extend_from_slice(&be32(1));
        t.extend_from_slice(&be16(400));
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&be32(0));
        t.extend_from_slice(b"2009:07:14 18:30:05\0");
        t.extend_from_slice(&be32(1));
        t.extend_from_slice(&be32(250));
        t.extend_from_slice(&be32(28));
        t.extend_from_slice(&be32(10));
        // GPS IFD: N 59° 26' 0", E 24° 45' 0".
        assert_eq!(t.len(), gps_at);
        let rationals = gps_at + 2 + 4 * 12 + 4;
        t.extend_from_slice(&be16(4));
        ifd0(&mut t, 1, 2, 2, u32::from_be_bytes(*b"N\0\0\0"));
        ifd0(&mut t, 2, 5, 3, rationals as u32);
        ifd0(&mut t, 3, 2, 2, u32::from_be_bytes(*b"E\0\0\0"));
        ifd0(&mut t, 4, 5, 3, rationals as u32 + 24);
        t.extend_from_slice(&be32(0));
        for (n, d) in [(59, 1), (26, 1), (0, 1), (24, 1), (45, 1), (0, 1)] {
            t.extend_from_slice(&be32(n));
            t.extend_from_slice(&be32(d));
        }

        t
    }

    /// The same block wrapped as a JPEG APP1 segment.
    pub(crate) fn sample_app1() -> Vec<u8> {
        let tiff = sample_tiff();
        let mut segment = vec![0xff, APP1];
        segment.extend_from_slice(&((2 + EXIF_HEADER.len() + tiff.len()) as u16).to_be_bytes());
        segment.extend_from_slice(EXIF_HEADER);
        segment.extend_from_slice(&tiff);
        segment
    }

    #[test]
    fn reads_camera_fields() {
        let t = sample_tiff();
        let exif = parse(&t).unwrap();
        assert_eq!(exif.taken.as_deref(), Some("2009:07:14 18:30:05"));
        let get = |label: &str| exif.fields.iter().find(|(l, _)| *l == label).map(|(_, v)| v.as_str());
        assert_eq!(get("Camera"), Some("Canon EOS 5D"));
        assert_eq!(get("Exposure"), Some("1/250 s"));
        assert_eq!(get("Aperture"), Some("f/2.8"));
        assert_eq!(get("ISO"), Some("400"));
        assert_eq!(get("GPS"), Some("59.433333, 24.750000"));
    }
}
