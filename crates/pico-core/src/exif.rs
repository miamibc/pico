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
    /// Human-readable `(label, value)` pairs in display order: the summary.
    pub fields: Vec<(&'static str, String)>,
    /// Every tag of the main, EXIF and GPS directories, named and readable,
    /// in file order, then the text in the maker's notes; what cannot be read
    /// (the rest of the maker's notes, pointers) in hex.
    pub all: Vec<(String, String)>,
}

/// Reads EXIF from a JPEG's first segments. Returns `None` when there is none.
/// Besides JPEG: PNG's `eXIf` chunk, WebP's `EXIF` chunk, TIFF and the RAW
/// files built like it, and wherever else an `Exif` block shows near the
/// start (HEIC, AVIF).
pub fn read(path: &std::path::Path) -> Option<Exif> {
    use std::io::Read;
    let mut head = Vec::new();
    // EXIF sits at the start; 256 KiB covers it even with large previews.
    std::fs::File::open(path).ok()?.take(256 * 1024).read_to_end(&mut head).ok()?;
    // HEIC, HEIF and AVIF keep EXIF as an item of its own, found by the
    // item list and located by the item locations.
    if head.get(4..8) == Some(b"ftyp")
        && let Some((offset, length)) = heif_exif(&head)
    {
        use std::io::{Seek, SeekFrom};
        let mut file = std::fs::File::open(path).ok()?;
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut item = Vec::new();
        file.take(length.min(16 * 1024 * 1024)).read_to_end(&mut item).ok()?;
        // The item starts with where the TIFF header is, after "Exif\0\0".
        let skip = u32::from_be_bytes(item.get(..4)?.try_into().ok()?) as usize;
        let tiff = item.get(4 + skip..)?;
        return parse(tiff.strip_prefix(EXIF_HEADER).unwrap_or(tiff));
    }
    if head.starts_with(b"II*\0") || head.starts_with(b"MM\0*") {
        // TIFF and RAW: the whole file is the block, with values further in.
        head.clear();
        std::fs::File::open(path).ok()?.take(16 * 1024 * 1024).read_to_end(&mut head).ok()?;
        return parse(&head);
    }
    parse(&tiff_block(&head)?)
}

/// The TIFF-structured EXIF block of a file's first bytes.
/// Where the EXIF item of a HEIF file (HEIC, AVIF) is: its offset and
/// length, from the `meta` box in `head` (ISO/IEC 14496-12 and 23008-12).
pub(crate) fn heif_exif(head: &[u8]) -> Option<(u64, u64)> {
    // The boxes in `data`: (type, contents).
    fn boxes(mut data: &[u8]) -> Vec<(&[u8], &[u8])> {
        let mut out = Vec::new();
        while data.len() >= 8 {
            let size = u32::from_be_bytes(data[..4].try_into().unwrap()) as u64;
            let (header, size) = match size {
                1 if data.len() >= 16 => (16, u64::from_be_bytes(data[8..16].try_into().unwrap())),
                0 => (8, data.len() as u64),
                size => (8, size),
            };
            let Some(size) = usize::try_from(size).ok().filter(|&s| s >= header) else { break };
            let end = size.min(data.len());
            out.push((&data[4..8], &data[header.min(end)..end]));
            data = &data[end..];
        }
        out
    }
    // Big-endian numbers of 0, 4 or 8 bytes, moving on.
    fn take(data: &mut &[u8], bytes: usize) -> Option<u64> {
        let (field, rest) = (data.get(..bytes)?, data.get(bytes..)?);
        *data = rest;
        Some(field.iter().fold(0u64, |n, &b| n << 8 | u64::from(b)))
    }
    let meta = boxes(head).into_iter().find(|(kind, _)| *kind == b"meta")?.1;
    // A full box: version and flags first.
    let children = boxes(meta.get(4..)?);
    let iinf = children.iter().find(|(kind, _)| *kind == b"iinf")?.1;
    let version = *iinf.first()?;
    let entries = iinf.get(4 + if version == 0 { 2 } else { 4 }..)?;
    let item = boxes(entries).into_iter().filter(|(kind, _)| *kind == b"infe").find_map(|(_, infe)| {
        let mut data = infe;
        let version = *data.first()?;
        take(&mut data, 4)?;
        let id = take(&mut data, if version >= 3 { 4 } else { 2 })?;
        take(&mut data, 2)?;
        (version >= 2 && data.get(..4)? == b"Exif").then_some(id)
    })?;
    let mut iloc = children.iter().find(|(kind, _)| *kind == b"iloc")?.1;
    let version = *iloc.first()?;
    take(&mut iloc, 4)?;
    let sizes = take(&mut iloc, 2)?;
    let (offset_size, length_size) = ((sizes >> 12) as usize & 15, (sizes >> 8) as usize & 15);
    let (base_size, index_size) = ((sizes >> 4) as usize & 15, if version >= 1 { sizes as usize & 15 } else { 0 });
    let count = take(&mut iloc, if version < 2 { 2 } else { 4 })?;
    for _ in 0..count {
        let id = take(&mut iloc, if version < 2 { 2 } else { 4 })?;
        // Only offsets into the file itself (construction method 0).
        let method = if version >= 1 { take(&mut iloc, 2)? & 15 } else { 0 };
        take(&mut iloc, 2)?;
        let base = take(&mut iloc, base_size)?;
        let extents = take(&mut iloc, 2)?;
        let mut first = None;
        for _ in 0..extents {
            take(&mut iloc, index_size)?;
            let (offset, length) = (take(&mut iloc, offset_size)?, take(&mut iloc, length_size)?);
            first = first.or(Some((base + offset, length)));
        }
        if id == item && method == 0 {
            return first;
        }
    }
    None
}

fn tiff_block(head: &[u8]) -> Option<Vec<u8>> {
    if let Some(segment) = exif_segment(head) {
        return Some(segment[4 + EXIF_HEADER.len()..].to_vec());
    }
    if let Some(data) = png_chunk(head, b"eXIf") {
        return Some(data.to_vec());
    }
    // ImageMagick keeps EXIF in PNG as hex text.
    if let Some(bytes) = png_chunks(head)
        .filter(|(kind, _)| matches!(kind, b"tEXt" | b"zTXt" | b"iTXt"))
        .filter_map(|(kind, data)| crate::metadata::png_text(&kind, data))
        .find(|(key, _)| key == "Raw profile type exif")
        .and_then(|(_, text)| crate::metadata::raw_profile(&text))
    {
        return Some(bytes.strip_prefix(EXIF_HEADER).unwrap_or(&bytes).to_vec());
    }
    if let Some(data) = riff_chunk(head, b"EXIF") {
        return Some(data.strip_prefix(EXIF_HEADER).unwrap_or(data).to_vec());
    }
    let at = head.windows(8).position(|w| w[..6] == *EXIF_HEADER && (w[6..] == *b"II" || w[6..] == *b"MM"))?;
    Some(head[at + EXIF_HEADER.len()..].to_vec())
}

/// The data of the first PNG chunk of `kind`.
pub(crate) fn png_chunk<'a>(file: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    png_chunks(file).find(|(k, _)| k == kind).map(|(_, data)| data)
}

/// A PNG file's chunks: kind and data.
pub(crate) fn png_chunks(file: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut at = if file.starts_with(b"\x89PNG\r\n\x1a\n") { 8 } else { file.len() };
    std::iter::from_fn(move || {
        let len = u32::from_be_bytes(file.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = file.get(at + 4..at + 8)?.try_into().ok()?;
        let data = file.get(at + 8..at + 8 + len)?;
        at += 12 + len;
        Some((kind, data))
    })
}

/// The data of the first chunk of `kind` in a RIFF file (WebP).
pub(crate) fn riff_chunk<'a>(file: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    if !file.starts_with(b"RIFF") || file.get(8..12)? != b"WEBP" {
        return None;
    }
    let mut at = 12;
    while let Some(header) = file.get(at..at + 8) {
        let len = u32::from_le_bytes(header[4..].try_into().ok()?) as usize;
        if &header[..4] == kind {
            return file.get(at + 8..at + 8 + len);
        }
        at += 8 + len + len % 2;
    }
    None
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
    let mut all: Vec<(String, String)> = tags.iter().filter_map(|(&t, v)| describe(t, v)).collect();
    // The EXIF and GPS directories, and the Interoperability one inside EXIF.
    let mut pending = std::collections::VecDeque::from(subs);
    let mut seen = vec![ifd0];
    let mut maker = Vec::new();
    while let Some((tag, offset)) = pending.pop_front() {
        if tag == 0x927c {
            let text = |tag: u32| match tags.get(&tag) {
                Some(Value::Text(text)) => text.clone(),
                _ => String::new(),
            };
            let (make, model) = (text(0x010f), text(0x0110));
            let length = match tags.get(&0x927c) {
                Some(Value::Bytes(b)) => b.len(),
                _ => 0,
            };
            if let Some((summary, rows)) = crate::maker_notes::read(tiff, little, offset, length, &make, &model) {
                // In place of the hex.
                if let Some(row) = all.iter_mut().find(|(name, _)| name == "Maker Note") {
                    row.1 = summary;
                }
                maker = rows;
            }
            continue;
        }
        if seen.contains(&offset) || seen.len() > 8 {
            continue;
        }
        seen.push(offset);
        let mut nested = Vec::new();
        let mut found = std::collections::BTreeMap::new();
        reader.read_ifd(offset, &mut found, &mut nested);
        // GPS and Interoperability tags reuse small numbers; keep them apart.
        let base = match tag {
            0x8825 => 0x1_0000,
            0xa005 => 0x2_0000,
            _ => 0,
        };
        for (t, v) in found {
            all.extend(describe(base + t, &v));
            tags.insert(base + t, v);
        }
        pending.extend(nested);
    }
    all.extend(maker);
    // IFD1 describes the embedded thumbnail.
    if let Some(ifd1) = reader.next_ifd(ifd0).filter(|&at| at != 0 && !seen.contains(&at)) {
        let mut found = std::collections::BTreeMap::new();
        reader.read_ifd(ifd1, &mut found, &mut Vec::new());
        all.extend(found.iter().filter_map(|(&t, v)| describe(t, v)).map(|(name, v)| (format!("Thumbnail {name}"), v)));
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
    let mut exif = Exif { taken: text(0x9003).or_else(|| text(0x0132)), gps: None, fields: Vec::new(), all };
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
pub(crate) enum Value {
    Text(String),
    Numbers(Vec<u32>),
    Rationals(Vec<(f64, f64)>),
    /// BYTE and UNDEFINED values, as they are.
    Bytes(Vec<u8>),
    /// SBYTE, SSHORT and SLONG.
    Signed(Vec<i32>),
    /// FLOAT and DOUBLE.
    Floats(Vec<f64>),
}

impl Value {
    /// The value at `at` as a number, whatever its type.
    pub(crate) fn number(&self, at: usize) -> Option<f64> {
        match self {
            Value::Numbers(n) => n.get(at).map(|&v| v as f64),
            Value::Signed(n) => n.get(at).map(|&v| v as f64),
            Value::Floats(n) => n.get(at).copied(),
            Value::Bytes(b) => b.get(at).map(|&v| v as f64),
            Value::Rationals(r) => r.get(at).map(|(n, d)| if *d == 0.0 { 0.0 } else { n / d }),
            Value::Text(t) => (at == 0).then(|| t.trim().parse().ok()).flatten(),
        }
    }

    pub(crate) fn count(&self) -> usize {
        match self {
            Value::Numbers(n) => n.len(),
            Value::Signed(n) => n.len(),
            Value::Floats(n) => n.len(),
            Value::Bytes(b) => b.len(),
            Value::Rationals(r) => r.len(),
            Value::Text(_) => 1,
        }
    }
}

/// A value from XMP (where EXIF's `exif:` and `tiff:` properties are text)
/// in the words the EXIF tag `name` gets; `None` when it has no special form.
pub(crate) fn readable(name: &str, text: &str) -> Option<String> {
    let tag = crate::exif_tags::tag_named(name)?;
    let value = match text.split_once('/') {
        Some((n, d)) => Value::Rationals(vec![(n.trim().parse().ok()?, d.trim().parse().ok()?)]),
        None => Value::Numbers(vec![text.trim().parse::<u32>().ok()?]),
    };
    let (_, shown) = describe(tag, &value)?;
    (shown != text.trim()).then_some(shown)
}

/// A tag's name and readable value, for the list of all tags; pointers and
/// binary blocks in hex.
fn describe(tag: u32, value: &Value) -> Option<(String, String)> {
    if matches!(tag, 0x8769 | 0x8825 | 0xa005 | 0x0201 | 0xea1d)
        && let Value::Numbers(n) = value
    {
        let name = tag_name(tag).or_else(|| crate::exif_tags::standard_name(tag)).unwrap_or("Pointer");
        return n.first().map(|offset| (name.to_owned(), format!("0x{offset:08X}")));
    }
    let name = match tag_name(tag).or_else(|| crate::exif_tags::standard_name(tag)) {
        Some(name) => name.to_owned(),
        None if tag >= 0x2_0000 => format!("Interoperability Tag 0x{:04X}", tag & 0xffff),
        None if tag >= 0x1_0000 => format!("GPS Tag 0x{:04X}", tag & 0xffff),
        None => format!("Tag 0x{tag:04X}"),
    };
    let first = || value.number(0).filter(|n| *n >= 0.0).map(|n| n as u32);
    let ratio = |at: usize| value.number(at);
    // Degrees and percent stick to the number; other units keep a space.
    let unit = |suffix: &str| {
        let gap = if suffix == "°" || suffix == "%" { "" } else { " " };
        ratio(0).map(|v| format!("{}{gap}{suffix}", trim_number(v)))
    };
    let text = match tag {
        0x829a => ratio(0).map(exposure),
        // APEX: the shutter is open 2^-v seconds, apertures are f/2^(v/2).
        0x9201 => ratio(0).map(|v| exposure(2f64.powf(-v))),
        0x829d => ratio(0).map(|f| format!("f/{f:.1}")),
        0x9202 | 0x9205 => ratio(0).map(|v| format!("f/{:.1}", 2f64.powf(v / 2.0))),
        0x9203 => ratio(0).map(|ev| format!("{ev:.2} EV")),
        0x9204 => ratio(0).map(|ev| format!("{ev:+.1} EV")),
        0x9206 => match value {
            Value::Rationals(r) if r.first().is_some_and(|(n, _)| *n == u32::MAX as f64) => Some("Infinity".into()),
            _ => unit("m"),
        },
        0x920a => ratio(0).map(|f| format!("{f:.1} mm")),
        0xa405 => first().map(|f| if f == 0 { "Unknown".into() } else { format!("{f} mm") }),
        0xa404 => ratio(0).map(|z| if z == 0.0 { "Not used".into() } else { format!("{}×", trim_number(z)) }),
        0xa432 | 0xc630 => lens(value),
        0x9209 => first().map(flash),
        0x4746 => first().map(|stars| {
            let stars = stars.min(5) as usize;
            format!("{}{} ({stars})", "★".repeat(stars), "☆".repeat(5 - stars))
        }),
        0x4749 | 0x9401 => unit("%"),
        0x882a => ratio(0).map(|h| format!("{h:+} h")),
        0x882b => unit("s"),
        0x9400 => unit("°C"),
        0x9402 => unit("hPa"),
        0x9403 | 0x1_0006 | 0x1_001f => unit("m"),
        0x9404 => unit("mGal"),
        0x9405 | 0x1_000f | 0x1_0011 | 0x1_0018 => unit("°"),
        // GPS references are letters, spelt out.
        0x1_0001 | 0x1_0003 | 0x1_0009 | 0x1_000a | 0x1_000c | 0x1_000e | 0x1_0010 | 0x1_0013 | 0x1_0015 | 0x1_0017
        | 0x1_0019 => match value {
            Value::Text(t) => gps_letter(tag, t.trim()).map(str::to_owned),
            _ => None,
        },
        0x1_0000 => match value {
            Value::Bytes(b) => Some(b.iter().map(u8::to_string).collect::<Vec<_>>().join(".")),
            _ => None,
        },
        0x1_0002 | 0x1_0004 | 0x1_0014 | 0x1_0016 => {
            let [d, m, s] = [ratio(0)?, ratio(1)?, ratio(2)?];
            Some(format!("{d:.0}° {m:.0}' {s:.2}\""))
        }
        0x1_0007 => {
            let [h, m, s] = [ratio(0)?, ratio(1)?, ratio(2)?];
            Some(format!("{h:02.0}:{m:02.0}:{s:02.0}"))
        }
        0x9101 => match value {
            Value::Bytes(b) => {
                Some(b.iter().filter_map(|c| ["", "Y", "Cb", "Cr", "R", "G", "B"].get(*c as usize).copied()).collect())
            }
            _ => None,
        },
        0x9000 | 0xa000 | 0x2_0002 => match value {
            Value::Bytes(b) if b.len() == 4 && b.iter().all(u8::is_ascii_digit) => {
                let v = String::from_utf8_lossy(b).into_owned();
                Some(format!("{}.{}", v[..2].trim_start_matches('0'), &v[2..]))
            }
            _ => None,
        },
        0x2_0001 => match value {
            Value::Text(t) => Some(match t.trim() {
                "R98" => "R98 (sRGB)".to_owned(),
                "R03" => "R03 (Adobe RGB)".to_owned(),
                "THM" => "THM (thumbnail)".to_owned(),
                other => other.to_owned(),
            }),
            _ => None,
        },
        // Text after an 8-byte character code.
        0x9286 | 0x1_001b | 0x1_001c => match value {
            Value::Bytes(b) => user_comment(b),
            _ => None,
        },
        0x9c9b..=0x9c9f => match value {
            Value::Bytes(b) => Some(utf16(b)),
            _ => None,
        },
        _ if value.count() == 1 => first().and_then(|n| choice(tag, n)).map(str::to_owned),
        _ => None,
    };
    let text = text.or_else(|| plain(value))?;
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some((name, text))
}

/// An exposure time: "1/250 s", or "2.5 s".
pub(crate) fn exposure(seconds: f64) -> String {
    if seconds > 0.0 && seconds < 1.0 { format!("1/{:.0} s", 1.0 / seconds) } else { format!("{seconds:.1} s") }
}

/// A number without needless decimals: 56, 56.5, 0.0123.
pub(crate) fn trim_number(v: f64) -> String {
    format!("{}", (v * 10_000.0).round() / 10_000.0)
}

/// LensSpecification: the shortest and longest focal length and the widest
/// aperture at each, 0 where unknown: "70–200 mm f/2.8".
pub(crate) fn lens(value: &Value) -> Option<String> {
    let [short, long, at_short, at_long] = [value.number(0)?, value.number(1)?, value.number(2)?, value.number(3)?];
    let focal = if short == long || long == 0.0 {
        format!("{} mm", trim_number(short))
    } else {
        format!("{}–{} mm", trim_number(short), trim_number(long))
    };
    let aperture = match (at_short, at_long) {
        (a, b) if a > 0.0 && (b == 0.0 || a == b) => format!(" f/{}", trim_number(a)),
        (a, b) if a > 0.0 => format!(" f/{}–{}", trim_number(a), trim_number(b)),
        _ => String::new(),
    };
    Some(focal + &aperture)
}

/// Any value as text: numbers and ratios as they are.
pub(crate) fn plain(value: &Value) -> Option<String> {
    let ratio = |(n, d): &(f64, f64)| {
        if *d == 1.0 || *d == 0.0 { format!("{n}") } else { format!("{}", ((n / d) * 10_000.0).round() / 10_000.0) }
    };
    Some(match value {
        Value::Text(s) => s.clone(),
        Value::Numbers(n) if n.len() > 16 => format!("{} values", n.len()),
        Value::Numbers(n) => n.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
        Value::Rationals(r) => r.iter().map(ratio).collect::<Vec<_>>().join(", "),
        Value::Signed(n) if n.len() > 16 => format!("{} values", n.len()),
        Value::Signed(n) => n.iter().map(i32::to_string).collect::<Vec<_>>().join(", "),
        Value::Floats(n) => n.iter().map(|&v| trim_number(v)).collect::<Vec<_>>().join(", "),
        Value::Bytes(b) if !b.is_empty() && b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') => {
            String::from_utf8_lossy(b).into_owned()
        }
        Value::Bytes(b) if b.len() <= 8 => b.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", "),
        Value::Bytes(b) => hex(b),
    })
}

/// The first bytes of a binary block in hex, and its size.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const SHOWN: usize = 32;
    let head: Vec<String> = bytes.iter().take(SHOWN).map(|b| format!("{b:02X}")).collect();
    if bytes.len() > SHOWN { format!("{} … ({} bytes)", head.join(" "), bytes.len()) } else { head.join(" ") }
}

/// UserComment: an 8-byte character code, then the text.
fn user_comment(bytes: &[u8]) -> Option<String> {
    let (code, text) = (bytes.get(..8)?, bytes.get(8..)?);
    let text = if code.starts_with(b"UNICODE") {
        utf16(text)
    } else {
        String::from_utf8_lossy(text).trim_end_matches('\0').to_owned()
    };
    Some(text)
}

/// Windows' XP tags: UTF-16LE, zero-terminated.
fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> =
        bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

fn flash(value: u32) -> String {
    let mut parts = vec![if value & 1 == 1 { "Fired" } else { "Did not fire" }];
    match (value >> 3) & 3 {
        1 => parts.push("compulsory"),
        2 => parts.push("suppressed"),
        3 => parts.push("auto"),
        _ => {}
    }
    if value & 0x40 != 0 {
        parts.push("red-eye reduction");
    }
    if value & 0x20 != 0 {
        return "No flash function".to_owned();
    }
    parts.join(", ")
}

/// What the GPS reference letters mean.
fn gps_letter(tag: u32, letter: &str) -> Option<&'static str> {
    Some(match (tag, letter) {
        (0x1_0001 | 0x1_0013, "N") => "North",
        (0x1_0001 | 0x1_0013, "S") => "South",
        (0x1_0003 | 0x1_0015, "E") => "East",
        (0x1_0003 | 0x1_0015, "W") => "West",
        (0x1_0009, "A") => "Measurement active",
        (0x1_0009, "V") => "Measurement void",
        (0x1_000a, "2") => "2-dimensional",
        (0x1_000a, "3") => "3-dimensional",
        (0x1_000c, "K") => "km/h",
        (0x1_000c, "M") => "mph",
        (0x1_000c, "N") => "knots",
        (0x1_0019, "K") => "Kilometers",
        (0x1_0019, "M") => "Miles",
        (0x1_0019, "N") => "Nautical miles",
        (0x1_000e | 0x1_0010 | 0x1_0017, "T") => "True north",
        (0x1_000e | 0x1_0010 | 0x1_0017, "M") => "Magnetic north",
        _ => return None,
    })
}

/// Words for the tags whose values are codes.
fn choice(tag: u32, value: u32) -> Option<&'static str> {
    let pick = |options: &[&'static str]| options.get(value as usize).copied().filter(|o| !o.is_empty());
    match tag {
        0x0112 => pick(&[
            "",
            "Normal",
            "Mirrored",
            "Rotated 180°",
            "Flipped",
            "Mirrored, rotated 90° counterclockwise",
            "Rotated 90° clockwise",
            "Mirrored, rotated 90° clockwise",
            "Rotated 90° counterclockwise",
        ]),
        0x0128 | 0xa210 | 0x9210 => pick(&["", "None", "Inches", "Centimeters"]),
        0x0213 => pick(&["", "Centered", "Co-sited"]),
        0x00fe => match value {
            0 => Some("Full-resolution image"),
            1 => Some("Reduced-resolution image"),
            2 => Some("Single page of multi-page image"),
            4 => Some("Transparency mask"),
            _ => None,
        },
        0x00ff => pick(&["", "Full-resolution image", "Reduced-resolution image", "Single page of multi-page image"]),
        0x0103 => match value {
            1 => Some("Uncompressed"),
            2 => Some("CCITT 1D"),
            3 => Some("T4/Group 3 Fax"),
            4 => Some("T6/Group 4 Fax"),
            5 => Some("LZW"),
            6 => Some("JPEG (old-style)"),
            7 => Some("JPEG"),
            8 | 32946 => Some("Deflate"),
            99 => Some("JPEG"),
            32773 => Some("PackBits"),
            34712 => Some("JPEG 2000"),
            34892 => Some("Lossy JPEG"),
            52546 => Some("JPEG XL"),
            _ => None,
        },
        0x0106 => match value {
            0 => Some("WhiteIsZero"),
            1 => Some("BlackIsZero"),
            2 => Some("RGB"),
            3 => Some("RGB Palette"),
            4 => Some("Transparency Mask"),
            5 => Some("CMYK"),
            6 => Some("YCbCr"),
            8 => Some("CIELab"),
            9 => Some("ICCLab"),
            10 => Some("ITULab"),
            32803 => Some("Color Filter Array"),
            34892 => Some("Linear Raw"),
            _ => None,
        },
        0x010a => pick(&["", "Normal", "Reversed"]),
        0x011c => pick(&["", "Chunky", "Planar"]),
        0x013d => match value {
            1 => Some("None"),
            2 => Some("Horizontal differencing"),
            3 => Some("Floating point"),
            _ => None,
        },
        0xa460 => pick(&[
            "Unknown",
            "Not a composite image",
            "General composite image",
            "Composite image captured while shooting",
        ]),
        0x8822 => pick(&[
            "Not defined",
            "Manual",
            "Normal program",
            "Aperture priority",
            "Shutter priority",
            "Creative program",
            "Action program",
            "Portrait mode",
            "Landscape mode",
        ]),
        0x8830 => pick(&[
            "Unknown",
            "Standard output sensitivity",
            "Recommended exposure index",
            "ISO speed",
            "Standard output sensitivity and recommended exposure index",
            "Standard output sensitivity and ISO speed",
            "Recommended exposure index and ISO speed",
            "All three",
        ]),
        0x9207 => match value {
            255 => Some("Other"),
            _ => pick(&["Unknown", "Average", "Center-weighted average", "Spot", "Multi-spot", "Pattern", "Partial"]),
        },
        0x9208 | 0xc65a | 0xc65b | 0xcd31 => match value {
            0 => Some("Unknown"),
            1 => Some("Daylight"),
            2 => Some("Fluorescent"),
            3 => Some("Tungsten"),
            4 => Some("Flash"),
            9 => Some("Fine weather"),
            10 => Some("Cloudy"),
            11 => Some("Shade"),
            12..=16 => Some("Fluorescent"),
            17 => Some("Standard light A"),
            18 => Some("Standard light B"),
            19 => Some("Standard light C"),
            20..=23 => Some("Daylight (D-series)"),
            24 => Some("ISO studio tungsten"),
            255 => Some("Other"),
            _ => None,
        },
        0xa001 => match value {
            1 => Some("sRGB"),
            2 => Some("Adobe RGB"),
            0xffff => Some("Uncalibrated"),
            _ => None,
        },
        0xa217 | 0x9217 => pick(&[
            "",
            "Not defined",
            "One-chip color area",
            "Two-chip color area",
            "Three-chip color area",
            "Color sequential area",
            "",
            "Trilinear",
            "Color sequential linear",
        ]),
        0xa300 => pick(&["Others", "Film scanner", "Reflection print scanner", "Digital camera"]),
        0xa301 => pick(&["", "Directly photographed"]),
        0xa401 => pick(&["Normal", "Custom"]),
        0xa402 => pick(&["Auto", "Manual", "Auto bracket"]),
        0xa403 => pick(&["Auto", "Manual"]),
        0xa406 => pick(&["Standard", "Landscape", "Portrait", "Night scene"]),
        0xa407 => pick(&["None", "Low gain up", "High gain up", "Low gain down", "High gain down"]),
        0xa408 | 0xa40a => pick(&["Normal", "Soft", "Hard"]),
        0xa409 => pick(&["Normal", "Low", "High"]),
        0xa40c => pick(&["Unknown", "Macro", "Close", "Distant"]),
        0x1_0005 => pick(&[
            "Above sea level",
            "Below sea level",
            "Positive sea-level reference",
            "Negative sea-level reference",
        ]),
        0x1_001e => pick(&["No correction", "Differential corrected"]),
        _ => None,
    }
}

/// Names of the standard tags; GPS ones are 0x1_0000 + their number.
fn tag_name(tag: u32) -> Option<&'static str> {
    Some(match tag {
        0x0100 => "Image Width",
        0x0101 => "Image Height",
        0x0102 => "Bits per Sample",
        0x0103 => "Compression",
        0x0106 => "Photometric Interpretation",
        0x010e => "Description",
        0x010f => "Make",
        0x0110 => "Model",
        0x0112 => "Orientation",
        0x011a => "X Resolution",
        0x011b => "Y Resolution",
        0x0128 => "Resolution Unit",
        0x0131 => "Software",
        0x0132 => "Date Modified",
        0x013b => "Artist",
        0x013e => "White Point",
        0x013f => "Primary Chromaticities",
        0x0211 => "YCbCr Coefficients",
        0x0201 => "Thumbnail Offset",
        0x0202 => "Thumbnail Length",
        0x0213 => "YCbCr Positioning",
        0x02bc => "XMP Packet",
        0x83bb => "IPTC Data",
        0x8769 => "Exif IFD Pointer",
        0x8773 => "ICC Profile",
        0x8825 => "GPS IFD Pointer",
        0x927c => "Maker Note",
        0xa005 => "Interoperability IFD Pointer",
        0xc4a5 => "Print Image Matching",
        0xea1c => "Padding",
        0xea1d => "Offset Schema",
        0x4746 => "Rating",
        0x4749 => "Rating Percent",
        0x8298 => "Copyright",
        0x829a => "Exposure Time",
        0x829d => "F Number",
        0x8822 => "Exposure Program",
        0x8824 => "Spectral Sensitivity",
        0x8827 => "ISO",
        0x8830 => "Sensitivity Type",
        0x8831 => "Standard Output Sensitivity",
        0x8832 => "Recommended Exposure Index",
        0x9000 => "Exif Version",
        0x9003 => "Date Taken",
        0x9004 => "Date Digitized",
        0x9010 => "Time Zone",
        0x9011 => "Time Zone (Taken)",
        0x9012 => "Time Zone (Digitized)",
        0x9101 => "Components Configuration",
        0x9102 => "Compressed Bits per Pixel",
        0x9201 => "Shutter Speed Value",
        0x9202 => "Aperture Value",
        0x9203 => "Brightness Value",
        0x9204 => "Exposure Compensation",
        0x9205 => "Max Aperture Value",
        0x9206 => "Subject Distance",
        0x9207 => "Metering Mode",
        0x9208 => "Light Source",
        0x9209 => "Flash",
        0x920a => "Focal Length",
        0x9214 => "Subject Area",
        0x9286 => "User Comment",
        0x9290 => "Subseconds",
        0x9291 => "Subseconds (Taken)",
        0x9292 => "Subseconds (Digitized)",
        0x9c9b => "Title",
        0x9c9c => "Comment",
        0x9c9d => "Author",
        0x9c9e => "Keywords",
        0x9c9f => "Subject",
        0xa000 => "FlashPix Version",
        0xa001 => "Color Space",
        0xa002 => "Width (camera)",
        0xa003 => "Height (camera)",
        0xa004 => "Related Sound File",
        0xa20e => "Focal Plane X Resolution",
        0xa20f => "Focal Plane Y Resolution",
        0xa210 => "Focal Plane Resolution Unit",
        0xa215 => "Exposure Index",
        0xa217 => "Sensing Method",
        0xa300 => "File Source",
        0xa301 => "Scene Type",
        0xa302 => "CFA Pattern",
        0xa401 => "Custom Rendered",
        0xa402 => "Exposure Mode",
        0xa403 => "White Balance",
        0xa404 => "Digital Zoom Ratio",
        0xa405 => "Focal Length (35 mm)",
        0xa406 => "Scene Capture Type",
        0xa407 => "Gain Control",
        0xa408 => "Contrast",
        0xa409 => "Saturation",
        0xa40a => "Sharpness",
        0xa40c => "Subject Distance Range",
        0xa420 => "Image Unique ID",
        0xa500 => "Gamma",
        0xa430 => "Camera Owner",
        0xa431 => "Body Serial Number",
        0xa432 => "Lens Specification",
        0xa433 => "Lens Make",
        0xa434 => "Lens Model",
        0xa435 => "Lens Serial Number",
        0x1_0000 => "GPS Version",
        0x1_0001 => "GPS Latitude Ref",
        0x1_0002 => "GPS Latitude",
        0x1_0003 => "GPS Longitude Ref",
        0x1_0004 => "GPS Longitude",
        0x1_0005 => "GPS Altitude Ref",
        0x1_0006 => "GPS Altitude",
        0x1_0007 => "GPS Time (UTC)",
        0x1_0008 => "GPS Satellites",
        0x1_0009 => "GPS Status",
        0x1_000a => "GPS Measure Mode",
        0x1_000b => "GPS Precision (DOP)",
        0x1_000c => "GPS Speed Ref",
        0x1_000d => "GPS Speed",
        0x1_000e => "GPS Track Ref",
        0x1_000f => "GPS Track",
        0x1_0010 => "GPS Image Direction Ref",
        0x1_0011 => "GPS Image Direction",
        0x1_0012 => "GPS Map Datum",
        0x1_0013 => "GPS Destination Latitude Ref",
        0x1_0014 => "GPS Destination Latitude",
        0x1_0015 => "GPS Destination Longitude Ref",
        0x1_0016 => "GPS Destination Longitude",
        0x1_0017 => "GPS Destination Bearing Ref",
        0x1_0018 => "GPS Destination Bearing",
        0x1_0019 => "GPS Destination Distance Ref",
        0x1_001a => "GPS Destination Distance",
        0x1_001b => "GPS Processing Method",
        0x1_001c => "GPS Area Information",
        0x1_001d => "GPS Date",
        0x1_001e => "GPS Differential",
        0x1_001f => "GPS Horizontal Error",
        _ => return None,
    })
}

pub(crate) struct Tiff<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) little: bool,
}

impl Tiff<'_> {
    pub(crate) fn u16(&self, at: usize) -> Option<u16> {
        let b = [*self.data.get(at)?, *self.data.get(at + 1)?];
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    pub(crate) fn u32(&self, at: usize) -> Option<u32> {
        let b = [*self.data.get(at)?, *self.data.get(at + 1)?, *self.data.get(at + 2)?, *self.data.get(at + 3)?];
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// Where the directory after the one at `at` starts (0 for none).
    fn next_ifd(&self, at: usize) -> Option<usize> {
        let count = self.u16(at)? as usize;
        self.u32(at + 2 + count * 12).map(|v| v as usize)
    }

    /// Reads one IFD's entries; pointers to the EXIF, GPS and
    /// Interoperability IFDs go to `subs`.
    pub(crate) fn read_ifd(
        &self,
        at: usize,
        tags: &mut std::collections::BTreeMap<u32, Value>,
        subs: &mut Vec<(u16, usize)>,
    ) {
        let Some(count) = self.u16(at) else { return };
        for i in 0..count as usize {
            let entry = at + 2 + i * 12;
            let (Some(tag), Some(kind), Some(n)) = (self.u16(entry), self.u16(entry + 2), self.u32(entry + 4)) else {
                return;
            };
            let n = n as usize;
            let size: usize = match kind {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 | 13 => 4,
                5 | 10 | 12 => 8,
                _ => continue,
            };
            let Some(total) = size.checked_mul(n) else { continue };
            let start = if total <= 4 { entry + 8 } else { self.u32(entry + 8).unwrap_or(0) as usize };
            if start + total > self.data.len() {
                continue;
            }
            // The maker's notes are a directory of their own in most cameras.
            if tag == 0x927c {
                subs.push((tag, start));
            }
            if tag == 0x8769 || tag == 0x8825 || tag == 0xa005 {
                if let Some(offset) = self.u32(entry + 8) {
                    subs.push((tag, offset as usize));
                    tags.insert(tag as u32, Value::Numbers(vec![offset]));
                }
                continue;
            }
            let value = match kind {
                2 => {
                    let bytes = &self.data[start..start + total];
                    let text = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
                    match std::str::from_utf8(text) {
                        Ok(text) => Value::Text(text.to_owned()),
                        // Older cameras wrote Latin-1 ("©"); anything else
                        // under a text type is binary data, kept as it is.
                        Err(_) if text.iter().all(|&b| (0x20..0x7f).contains(&b) || b >= 0xa0) => {
                            Value::Text(text.iter().map(|&b| b as char).collect())
                        }
                        Err(_) => Value::Bytes(bytes.to_vec()),
                    }
                }
                3 => Value::Numbers((0..n).filter_map(|k| self.u16(start + 2 * k).map(u32::from)).collect()),
                // LONG, and IFD (an offset, as in DNG).
                4 | 13 => Value::Numbers((0..n).filter_map(|k| self.u32(start + 4 * k)).collect()),
                6 => Value::Signed(self.data[start..start + total].iter().map(|&b| b as i8 as i32).collect()),
                8 => Value::Signed((0..n).filter_map(|k| self.u16(start + 2 * k).map(|v| v as i16 as i32)).collect()),
                9 => Value::Signed((0..n).filter_map(|k| self.u32(start + 4 * k).map(|v| v as i32)).collect()),
                11 => Value::Floats(
                    (0..n).filter_map(|k| self.u32(start + 4 * k).map(|v| f32::from_bits(v) as f64)).collect(),
                ),
                12 => Value::Floats(
                    (0..n)
                        .filter_map(|k| {
                            let (a, b) = (self.u32(start + 8 * k)? as u64, self.u32(start + 8 * k + 4)? as u64);
                            Some(f64::from_bits(if self.little { b << 32 | a } else { a << 32 | b }))
                        })
                        .collect(),
                ),
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
                _ => Value::Bytes(self.data[start..start + total].to_vec()),
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
    fn names_and_spells_out_more_tags() {
        let text = |t: &str| Value::Text(t.into());
        let shown = |tag, value| describe(tag, &value).unwrap();
        let named = |name: &str, value: &str| (name.to_owned(), value.to_owned());
        assert_eq!(shown(0x1_0017, text("T")), named("GPS Destination Bearing Ref", "True north"));
        assert_eq!(
            shown(0x1_0018, Value::Rationals(vec![(2_448_307.0, 10_000.0)])),
            named("GPS Destination Bearing", "244.8307°")
        );
        assert_eq!(shown(0x1_001f, Value::Rationals(vec![(5.0, 1.0)])), named("GPS Horizontal Error", "5 m"));
        assert_eq!(shown(0xa43a, text("darktable")), named("RAW Developing Software", "darktable"));
        assert_eq!(shown(0x1_0030, text("x")).0, "GPS Tag 0x0030");
        assert_eq!(shown(0x7777, text("x")).0, "Tag 0x7777");
    }

    /// IFD0 with Make "Canon", an EXIF directory holding the maker's notes,
    /// and in those a bare directory with two text entries, one of them
    /// binary data under a text type.
    fn canon_tiff() -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend_from_slice(&8u32.to_le_bytes());
        let entry = |t: &mut Vec<u8>, tag: u16, kind: u16, n: u32, value: u32| {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&kind.to_le_bytes());
            t.extend_from_slice(&n.to_le_bytes());
            t.extend_from_slice(&value.to_le_bytes());
        };
        // IFD0 at 8: Make at 38, EXIF IFD at 44.
        t.extend_from_slice(&2u16.to_le_bytes());
        entry(&mut t, 0x010f, 2, 6, 38);
        entry(&mut t, 0x8769, 4, 1, 44);
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend_from_slice(b"Canon\0");
        // EXIF IFD at 44: MakerNote (UNDEFINED) at 62.
        t.extend_from_slice(&1u16.to_le_bytes());
        entry(&mut t, 0x927c, 7, 30, 62);
        t.extend_from_slice(&0u32.to_le_bytes());
        // The notes at 62, 30 bytes; both texts fit in their entries.
        t.extend_from_slice(&2u16.to_le_bytes());
        entry(&mut t, 0x0006, 2, 4, u32::from_le_bytes(*b"A40\0"));
        entry(&mut t, 0x0009, 2, 4, u32::from_le_bytes([0xff, 0x01, 0xfe, 0]));
        t.extend_from_slice(&0u32.to_le_bytes());
        t
    }

    #[test]
    fn finds_the_exif_item_of_heif_files() {
        let sized = |kind: &[u8], body: Vec<u8>| {
            [((body.len() + 8) as u32).to_be_bytes().to_vec(), kind.to_vec(), body].concat()
        };
        // An image item (1) and the EXIF item (2), version 2 entries.
        let infe = |id: u16, kind: &[u8]| {
            sized(b"infe", [vec![2, 0, 0, 0], id.to_be_bytes().to_vec(), vec![0, 0], kind.to_vec(), vec![0]].concat())
        };
        let iinf = sized(b"iinf", [vec![0, 0, 0, 0, 0, 2], infe(1, b"hvc1"), infe(2, b"Exif")].concat());
        // Version 0, 4-byte offsets and lengths, no base offset: two items, one extent each.
        let mut iloc = vec![0, 0, 0, 0, 0x44, 0x00, 0, 2];
        for (id, offset, length) in [(1u16, 1000u32, 500u32), (2, 4242, 77)] {
            iloc.extend(id.to_be_bytes());
            iloc.extend([0, 0, 0, 1]);
            iloc.extend(offset.to_be_bytes());
            iloc.extend(length.to_be_bytes());
        }
        let meta = sized(b"meta", [vec![0, 0, 0, 0], iinf, sized(b"iloc", iloc)].concat());
        let file = [sized(b"ftyp", b"heic\0\0\0\0mif1heic".to_vec()), meta].concat();
        assert_eq!(heif_exif(&file), Some((4242, 77)));
    }

    #[test]
    fn reads_the_text_in_the_makers_notes() {
        let exif = parse(&canon_tiff()).unwrap();
        let get = |label: &str| exif.all.iter().find(|(l, _)| l == label).map(|(_, v)| v.as_str());
        assert_eq!(get("Maker Note"), Some("Canon, 2 entries"));
        assert_eq!(get("Canon Image Type"), Some("A40"));
        // Binary data under a text type shows in hex.
        assert_eq!(get("Canon Owner Name"), Some("FF 01 FE 00"));
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
