//! Camera RAW photos through the JPEG preview the camera stores inside.
//!
//! Nearly every RAW format is a TIFF (Sony ARW, Nikon NEF and NRW, Canon CR2,
//! DNG, Pentax PEF, Olympus ORF, Panasonic RW2, Samsung SRW…) whose
//! directories point at one or more JPEG previews next to the sensor data:
//! Nikon and newer Sonys (A7C II) keep a full-size one. Fujifilm's RAF has a
//! header of its own with the offset of its JPEG. Canon's CR3 is an ISO media
//! file, like MP4: its first track is a full-size JPEG, and the photo's EXIF
//! directories sit in a box of Canon's own. The preview is what the
//! camera made of the shot, with its colours and white balance, so it shows
//! the photo as its author saw it on the camera. How the sensor data is
//! compressed (Sony's lossless, Nikon's High Efficiency) does not matter.

use image::DynamicImage;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Where a JPEG lies in the file: (offset, length).
type Span = (u64, u64);

/// How much of the file's start is read to find the directories.
const HEAD: u64 = 1 << 20;

/// Decodes the largest JPEG preview inside a RAW file, upright.
pub fn preview(path: &Path) -> io::Result<DynamicImage> {
    let mut file = File::open(path)?;
    let mut head = Vec::new();
    (&mut file).take(HEAD).read_to_end(&mut head)?;
    let (mut candidates, orientation) = if head.starts_with(b"FUJIFILMCCD-RAW") {
        (raf_preview(&head).into_iter().collect(), None)
    } else if head.get(4..8) == Some(b"ftyp") {
        cr3_previews(&head).ok_or_else(|| io::Error::other("not a RAW file Picco can read"))?
    } else {
        tiff_previews(&head).ok_or_else(|| io::Error::other("not a RAW file Picco can read"))?
    };
    // The largest first; sensor data can look like a JPEG too (lossless
    // JPEG), so the first that decodes wins.
    candidates.sort_by_key(|&(_, len)| std::cmp::Reverse(len));
    candidates.dedup();
    let size = file.metadata()?.len();
    for (offset, len) in candidates {
        if len < 1024 || offset.checked_add(len).is_none_or(|end| end > size) {
            continue;
        }
        let mut jpeg = vec![0; len as usize];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut jpeg)?;
        if !is_plain_jpeg(&jpeg) {
            continue;
        }
        let Ok(mut img) = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg) else { continue };
        if let Some(orientation) = orientation.and_then(|o| image::metadata::Orientation::from_exif(o as u8)) {
            img.apply_orientation(orientation);
        }
        return Ok(img);
    }
    Err(io::Error::other("no preview inside the RAW file"))
}

/// Whether `data` is a JPEG an ordinary decoder reads: baseline or
/// progressive, not the lossless kind RAW files keep sensor data in.
fn is_plain_jpeg(data: &[u8]) -> bool {
    if !data.starts_with(&[0xff, 0xd8]) {
        return false;
    }
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xff {
            return false;
        }
        let marker = data[at + 1];
        match marker {
            0xc0..=0xc2 => return true,
            0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf => return false,
            0xd8 | 0x01 | 0xd0..=0xd7 => at += 2,
            0xff => at += 1,
            _ => at += 2 + u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize,
        }
    }
    false
}

/// Fujifilm RAF: the JPEG's offset and length follow the header, big-endian.
fn raf_preview(head: &[u8]) -> Option<Span> {
    let word = |at: usize| Some(u32::from_be_bytes(head.get(at..at + 4)?.try_into().ok()?) as u64);
    Some((word(84)?, word(88)?))
}

/// Canon's box with the photo's metadata (`CMT1`: the EXIF IFD0 as a TIFF).
const CANON_BOX: [u8; 16] = *b"\x85\xc0\xb6\x87\x82\x0f\x11\xe0\x81\x11\xf4\xce\x46\x2b\x6a\x48";

/// Canon CR3: the first sample of every track, as (offset, length) — the
/// full-size JPEG, a smaller one and the sensor data — and the orientation
/// from the EXIF directory in Canon's box.
fn cr3_previews(head: &[u8]) -> Option<(Vec<Span>, Option<u32>)> {
    if head.get(4..12) != Some(b"ftypcrx ") {
        return None;
    }
    let moov = boxes(head, 0, head.len()).find(|b| &b.kind == b"moov")?;
    let mut found = Vec::new();
    let mut orientation = None;
    for child in boxes(head, moov.body, moov.end) {
        match &child.kind {
            b"uuid" if head.get(child.body..child.body + 16) == Some(&CANON_BOX) => {
                let cmt1 = boxes(head, child.body + 16, child.end).find(|b| &b.kind == b"CMT1");
                orientation = cmt1.and_then(|b| tiff_previews(&head[b.body..b.end])).and_then(|(_, o)| o);
            }
            b"trak" => {
                let stbl = ["mdia", "minf", "stbl"]
                    .iter()
                    .try_fold(child, |b, name| boxes(head, b.body, b.end).find(|c| c.kind == name.as_bytes()));
                if let Some(span) = stbl.and_then(|stbl| first_sample(head, stbl)) {
                    found.push(span);
                }
            }
            _ => {}
        }
    }
    Some((found, orientation))
}

/// The EXIF of a CR3, from `head`: IFD0 (`CMT1`), and the EXIF (`CMT2`) and
/// GPS (`CMT4`) directories, each a TIFF block of its own.
pub(crate) fn cr3_exif(head: &[u8]) -> Option<Cr3Exif<'_>> {
    if head.get(4..12) != Some(b"ftypcrx ") {
        return None;
    }
    let moov = boxes(head, 0, head.len()).find(|b| &b.kind == b"moov")?;
    let canon = boxes(head, moov.body, moov.end)
        .find(|b| &b.kind == b"uuid" && head.get(b.body..b.body + 16) == Some(&CANON_BOX))?;
    let part = |name: &[u8; 4]| {
        boxes(head, canon.body + 16, canon.end).find(|b| &b.kind == name).map(|b| &head[b.body..b.end])
    };
    Some(Cr3Exif { ifd0: part(b"CMT1")?, exif: part(b"CMT2"), gps: part(b"CMT4") })
}

/// The EXIF blocks of a CR3, each a TIFF of its own.
pub(crate) struct Cr3Exif<'a> {
    pub ifd0: &'a [u8],
    pub exif: Option<&'a [u8]>,
    pub gps: Option<&'a [u8]>,
}

/// Where the first sample of a track lies, from its `stbl` box: the offset
/// of its first chunk (`stco` or `co64`) and its size (`stsz`).
fn first_sample(head: &[u8], stbl: IsoBox) -> Option<Span> {
    let be32 = |at: usize| Some(u32::from_be_bytes(head.get(at..at + 4)?.try_into().ok()?) as u64);
    let be64 = |at: usize| Some(u64::from_be_bytes(head.get(at..at + 8)?.try_into().ok()?));
    let mut offset = None;
    let mut len = None;
    for b in boxes(head, stbl.body, stbl.end) {
        // Each starts with a version and flags, then an entry count.
        match &b.kind {
            b"stco" => offset = be32(b.body + 8),
            b"co64" => offset = be64(b.body + 8),
            // One size for every sample, or 0 and a size per sample.
            b"stsz" => len = be32(b.body + 4).filter(|&n| n > 0).or_else(|| be32(b.body + 12)),
            _ => {}
        }
    }
    Some((offset?, len?))
}

/// A box of an ISO media file: its kind, where its contents start and where
/// it ends.
#[derive(Clone, Copy)]
struct IsoBox {
    kind: [u8; 4],
    body: usize,
    end: usize,
}

/// The boxes one after another in `data[start..end]`.
fn boxes(data: &[u8], start: usize, end: usize) -> impl Iterator<Item = IsoBox> + '_ {
    let end = end.min(data.len());
    let mut at = start;
    std::iter::from_fn(move || {
        let size = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as u64;
        let kind: [u8; 4] = data.get(at + 4..at + 8)?.try_into().ok()?;
        let (size, header) = match size {
            1 => (u64::from_be_bytes(data.get(at + 8..at + 16)?.try_into().ok()?), 16),
            0 => ((end - at) as u64, 8),
            size => (size, 8),
        };
        let box_end = at.checked_add(usize::try_from(size).ok()?)?;
        if size < header as u64 || box_end > end && kind != *b"mdat" {
            return None;
        }
        let found = IsoBox { kind, body: at + header, end: box_end.min(end) };
        at = box_end;
        Some(found)
    })
}

/// Every JPEG a TIFF-based RAW points at, as (offset, length), and the
/// photo's EXIF orientation.
fn tiff_previews(head: &[u8]) -> Option<(Vec<Span>, Option<u32>)> {
    let little = match head.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let tiff = Tiff { data: head, little };
    // 42 for TIFF; Olympus and Panasonic use their own magic numbers.
    if !matches!(tiff.u16(2)?, 42 | 0x4f52 | 0x5352 | 0x55) {
        return None;
    }
    let mut found = Vec::new();
    let mut orientation = None;
    let mut queue = vec![tiff.u32(4)? as usize];
    let mut seen = std::collections::HashSet::new();
    while let Some(at) = queue.pop() {
        if at == 0 || !seen.insert(at) || seen.len() > 64 {
            continue;
        }
        let Some(count) = tiff.u16(at) else { continue };
        let mut fields = std::collections::HashMap::new();
        for i in 0..count as usize {
            let entry = at + 2 + i * 12;
            let (Some(tag), Some(kind), Some(n)) = (tiff.u16(entry), tiff.u16(entry + 2), tiff.u32(entry + 4)) else {
                break;
            };
            let values = tiff.values(entry, kind, n);
            if tag == 0x014a {
                // SubIFDs: Nikon's full-size preview, DNG's previews.
                queue.extend(values.iter().map(|&v| v as usize));
            }
            fields.insert(tag, values);
        }
        let first = |tag: u16| fields.get(&tag).and_then(|v| v.first().copied()).map(u64::from);
        if orientation.is_none() {
            orientation = first(0x0112).map(|v| v as u32);
        }
        // A JPEG pointed at directly (JPEGInterchangeFormat), …
        if let (Some(offset), Some(len)) = (first(0x0201), first(0x0202)) {
            found.push((offset, len));
        }
        // … one kept as the directory's single strip (old-style JPEG, or
        // JPEG in DNG), …
        if matches!(first(0x0103), Some(6 | 7))
            && let (Some(offset), Some(len)) = (first(0x0111), first(0x0117))
            && fields.get(&0x0111).is_some_and(|v| v.len() == 1)
        {
            found.push((offset, len));
        }
        // … or Panasonic's JpgFromRaw, a JPEG inside the tag itself.
        if let Some((offset, len)) = tiff.blob(at, count, 0x002e) {
            found.push((offset, len));
        }
        queue.push(tiff.u32(at + 2 + count as usize * 12).unwrap_or(0) as usize);
    }
    Some((found, orientation))
}

struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.data.get(at..at + 4)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// The numbers of a SHORT, LONG or IFD entry; nothing for other types.
    fn values(&self, entry: usize, kind: u16, n: u32) -> Vec<u32> {
        let size = match kind {
            3 => 2,
            4 | 13 => 4,
            _ => return Vec::new(),
        };
        let n = (n as usize).min(64);
        let start = if size * n <= 4 { entry + 8 } else { self.u32(entry + 8).unwrap_or(0) as usize };
        (0..n)
            .filter_map(|k| if size == 2 { self.u16(start + 2 * k).map(u32::from) } else { self.u32(start + 4 * k) })
            .collect()
    }

    /// Where the bytes of an UNDEFINED entry `tag` of the directory at `at` lie.
    fn blob(&self, at: usize, count: u16, tag: u16) -> Option<Span> {
        (0..count as usize).map(|i| at + 2 + i * 12).find(|&e| self.u16(e) == Some(tag)).and_then(|entry| {
            let len = self.u32(entry + 4)? as u64;
            (len > 4).then_some(())?;
            Some((self.u32(entry + 8)? as u64, len))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A little-endian TIFF like a Sony ARW: IFD0 with an orientation and a
    /// small JPEG, a SubIFD with a bigger one (as Nikon keeps), and a
    /// lossless-JPEG strip like sensor data, which must be passed over.
    fn fake_raw(small: &[u8], big: &[u8]) -> Vec<u8> {
        let mut lossless = vec![0xff, 0xd8, 0xff, 0xc3, 0, 4, 0, 0];
        lossless.resize(big.len() + 10_000, 0);
        let ifd0 = 8usize;
        let sub = ifd0 + 2 + 4 * 12 + 4;
        let data = sub + 2 + 3 * 12 + 4;
        let (small_at, big_at) = (data, data + small.len());
        let lossless_at = big_at + big.len();
        let mut t = b"II*\0".to_vec();
        t.extend_from_slice(&(ifd0 as u32).to_le_bytes());
        let entry = |t: &mut Vec<u8>, tag: u16, kind: u16, value: u32| {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&kind.to_le_bytes());
            t.extend_from_slice(&1u32.to_le_bytes());
            t.extend_from_slice(&value.to_le_bytes());
        };
        t.extend_from_slice(&4u16.to_le_bytes());
        entry(&mut t, 0x0112, 3, 6); // rotate 90° clockwise
        entry(&mut t, 0x014a, 4, sub as u32);
        entry(&mut t, 0x0201, 4, small_at as u32);
        entry(&mut t, 0x0202, 4, small.len() as u32);
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend_from_slice(&3u16.to_le_bytes());
        entry(&mut t, 0x0103, 3, 7);
        entry(&mut t, 0x0111, 4, lossless_at as u32);
        entry(&mut t, 0x0117, 4, lossless.len() as u32);
        // A second SubIFD is chained as the next directory.
        let next = data + small.len() + big.len() + lossless.len();
        t.extend_from_slice(&(next as u32).to_le_bytes());
        t.extend_from_slice(small);
        t.extend_from_slice(big);
        t.extend_from_slice(&lossless);
        t.extend_from_slice(&2u16.to_le_bytes());
        entry(&mut t, 0x0201, 4, big_at as u32);
        entry(&mut t, 0x0202, 4, big.len() as u32);
        t.extend_from_slice(&0u32.to_le_bytes());
        t
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 3) as u8, (y * 5) as u8, 90]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn takes_the_largest_preview_upright() {
        let dir = std::env::temp_dir().join(format!("picco-raw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("DSC00001.ARW");
        std::fs::write(&path, fake_raw(&jpeg(160, 120), &jpeg(640, 480))).unwrap();
        let img = preview(&path).unwrap();
        // 640 × 480 turned a quarter: the big one, upright.
        assert_eq!((img.width(), img.height()), (480, 640));
        std::fs::write(&path, b"II*\0\x08\0\0\0\0\0\0\0\0\0").unwrap();
        assert!(preview(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lossless_jpeg_is_not_a_preview() {
        assert!(is_plain_jpeg(&jpeg(16, 16)));
        assert!(!is_plain_jpeg(&[0xff, 0xd8, 0xff, 0xc3, 0, 4, 0, 0]));
        assert!(!is_plain_jpeg(b"not a jpeg"));
    }

    /// Real camera files, when there are some: `PICCO_RAW_SAMPLES` lists them,
    /// else those in `testdata/raw-samples` (not in the repository).
    #[test]
    fn cr3_reads_the_first_track_turned_and_its_exif() {
        fn iso(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
            [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat()
        }
        // A TIFF block with one directory of one entry.
        fn tiff(tag: u16, kind: u16, count: u32, value: [u8; 4], tail: &[u8]) -> Vec<u8> {
            let mut t = b"II*\0\x08\0\0\0\x01\0".to_vec();
            t.extend(tag.to_le_bytes());
            t.extend(kind.to_le_bytes());
            t.extend(count.to_le_bytes());
            t.extend(value);
            t.extend([0; 4]);
            t.extend(tail);
            t
        }
        // Busy enough to be over the 1 KiB a preview must have.
        let photo =
            image::RgbImage::from_fn(64, 32, |x, y| image::Rgb([((x * 37) ^ (y * 91)) as u8, (x * y) as u8, 0]));
        let mut jpeg = Vec::new();
        photo.write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        assert!(jpeg.len() > 1024);
        // Orientation 6: turned a quarter clockwise. The date sits after
        // the directory, at 8 + 2 + 12 + 4 = 26.
        let cmt1 = tiff(0x0112, 3, 1, [6, 0, 0, 0], &[]);
        let cmt2 = tiff(0x9003, 2, 20, [26, 0, 0, 0], b"2024:05:01 10:00:00\0");
        let canon = [&CANON_BOX[..], &iso(b"CMT1", &cmt1), &iso(b"CMT2", &cmt2)].concat();
        let build = |offset: u32| {
            let stco = iso(b"stco", &[&[0; 4][..], &1u32.to_be_bytes(), &offset.to_be_bytes()].concat());
            let stsz = iso(b"stsz", &[&[0; 8][..], &1u32.to_be_bytes(), &(jpeg.len() as u32).to_be_bytes()].concat());
            let stbl = iso(b"stbl", &[stco, stsz].concat());
            let trak = iso(b"trak", &iso(b"mdia", &iso(b"minf", &stbl)));
            let moov = iso(b"moov", &[iso(b"uuid", &canon), trak].concat());
            [iso(b"ftyp", b"crx \0\0\0\x01crx isom"), moov].concat()
        };
        // The JPEG goes right after the boxes, whose length does not depend
        // on where it is.
        let head = build(0);
        let file = [build(head.len() as u32 + 8), iso(b"mdat", &jpeg)].concat();
        let path = std::env::temp_dir().join(format!("picco-test-{}.cr3", std::process::id()));
        std::fs::write(&path, &file).unwrap();
        let img = preview(&path).unwrap();
        assert_eq!((img.width(), img.height()), (32, 64), "turned upright");
        let exif = crate::exif::read(&path).unwrap();
        assert_eq!(exif.taken.as_deref(), Some("2024:05:01 10:00:00"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn real_raw_files() {
        let paths: Vec<std::path::PathBuf> = match std::env::var_os("PICCO_RAW_SAMPLES") {
            Some(list) => std::env::split_paths(&list).collect(),
            None => std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/raw-samples"))
                .map(|dir| dir.flatten().map(|e| e.path()).collect())
                .unwrap_or_default(),
        };
        for path in paths {
            let img = preview(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let exif = crate::exif::read_quick(&path);
            let taken = exif.as_ref().and_then(|e| e.taken.clone());
            let fields = exif.map_or(0, |e| e.fields.len());
            println!("{}: {} × {}, taken {taken:?}, {fields} EXIF fields", path.display(), img.width(), img.height());
        }
    }
}
