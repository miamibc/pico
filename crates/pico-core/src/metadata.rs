//! Metadata besides EXIF and XMP, for the Properties panel: IPTC-IIM, the
//! ICC colour profile, JPEG's JFIF header and comment, PNG's header and text
//! chunks, WebP's header, and the tags ffprobe finds in videos. Each kind is
//! a titled section of `(label, value)` rows.
//!
//! Where the formats come from: IPTC-IIM 4.2 (iptc.org), ICC.1:2022
//! (color.org), JFIF 1.02, the PNG specification (w3.org/TR/png-3), the WebP
//! container (developers.google.com/speed/webp/docs/riff_container), and
//! ExifTool's tag tables for names.

use crate::exif::{png_chunks, riff_chunk};
use crate::media::MediaKind;
use std::path::Path;

pub type Rows = Vec<(String, String)>;

/// The sections found in the file, in a fixed order; empty ones left out.
pub fn read(path: &Path, kind: MediaKind) -> Vec<(&'static str, Rows)> {
    if kind == MediaKind::Video {
        return vec![("Video", crate::video::describe(path))]
            .into_iter()
            .filter(|(_, rows)| !rows.is_empty())
            .collect();
    }
    use std::io::Read;
    let mut head = Vec::new();
    let Ok(file) = std::fs::File::open(path) else { return Vec::new() };
    // Metadata comes before the image data; 1 MiB holds even large profiles.
    if file.take(1024 * 1024).read_to_end(&mut head).is_err() {
        return Vec::new();
    }
    let mut sections = Vec::new();
    let mut icc = None;
    if head.starts_with(&[0xff, 0xd8]) {
        let (jpeg, iptc, profile) = jpeg(&head);
        sections.push(("JPEG", jpeg));
        sections.push(("IPTC", iptc));
        icc = profile;
    } else if head.starts_with(b"\x89PNG") {
        let (png, iptc, profile) = png(&head);
        sections.push(("PNG", png));
        sections.push(("IPTC", iptc));
        icc = profile;
    } else if let Some(header) = riff_chunk(&head, b"VP8X").or_else(|| riff_chunk(&head, b"VP8L")) {
        sections.push(("WebP", webp(&head, header)));
        icc = riff_chunk(&head, b"ICCP").map(<[u8]>::to_vec);
    }
    sections.push(("ICC Profile", icc.map(|p| icc_profile(&p)).unwrap_or_default()));
    sections.retain(|(_, rows)| !rows.is_empty());
    sections
}

/// A position written as ISO 6709, as phones tag videos: "+59.4370+024.7536+012.0/".
pub fn iso6709(text: &str) -> Option<(f64, f64)> {
    let text = text.trim().trim_end_matches('/');
    let signs: Vec<usize> = text.char_indices().filter(|(_, c)| *c == '+' || *c == '-').map(|(i, _)| i).collect();
    if signs.first() != Some(&0) || signs.len() < 2 {
        return None;
    }
    let end = signs.get(2).copied().unwrap_or(text.len());
    let (lat, lon) = (text[..signs[1]].parse::<f64>().ok()?, text[signs[1]..end].parse::<f64>().ok()?);
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some((lat, lon))
}

fn push(rows: &mut Rows, label: &str, value: impl Into<String>) {
    let value = value.into();
    let value = value.trim();
    if !value.is_empty() {
        rows.push((label.to_owned(), value.to_owned()));
    }
}

/// JFIF and comments, the IPTC block, and the ICC profile (which may span
/// several APP2 segments).
fn jpeg(file: &[u8]) -> (Rows, Rows, Option<Vec<u8>>) {
    let (mut info, mut iptc, mut icc_parts) = (Rows::new(), Rows::new(), Vec::new());
    let mut at = 2;
    while at + 4 <= file.len() && file[at] == 0xff {
        let marker = file[at + 1];
        if marker == 0xda {
            break;
        }
        let len = u16::from_be_bytes([file[at + 2], file[at + 3]]) as usize;
        let Some(data) = file.get(at + 4..at + 2 + len) else { break };
        match marker {
            0xe0 if data.starts_with(b"JFIF\0") && data.len() >= 12 => {
                push(&mut info, "JFIF Version", format!("{}.{:02}", data[5], data[6]));
                let unit = ["", " dots per inch", " dots per cm"].get(data[7] as usize).copied().unwrap_or("");
                let (x, y) = (u16::from_be_bytes([data[8], data[9]]), u16::from_be_bytes([data[10], data[11]]));
                let density =
                    if unit.is_empty() { format!("{x}:{y} (aspect ratio)") } else { format!("{x} × {y}{unit}") };
                push(&mut info, "Density", density);
            }
            0xe2 if data.starts_with(b"ICC_PROFILE\0") && data.len() > 14 => {
                icc_parts.push((data[12], data[14..].to_vec()));
            }
            0xed if data.starts_with(b"Photoshop 3.0\0") => {
                let (found, resources) = photoshop(&data[14..]);
                iptc.extend(found);
                for (id, size) in resources {
                    let name = photoshop_name(id).map(|n| format!(" ({n})")).unwrap_or_default();
                    push(&mut info, &format!("Photoshop Resource 0x{id:04X}{name}"), format!("{size} bytes"));
                }
            }
            0xee if data.starts_with(b"Adobe") && data.len() >= 12 => {
                push(&mut info, "Adobe DCT Version", u16::from_be_bytes([data[5], data[6]]).to_string());
                let transform =
                    ["Unknown (RGB or CMYK)", "YCbCr", "YCCK"].get(data[11] as usize).copied().unwrap_or("");
                push(&mut info, "Adobe Color Transform", transform);
            }
            // Photoshop's Save for Web: the quality, 0-100.
            0xec if data.starts_with(b"Ducky") && data.len() >= 9 => {
                let mut rest = &data[5..];
                while rest.len() >= 4 {
                    let (tag, len) =
                        (u16::from_be_bytes([rest[0], rest[1]]), u16::from_be_bytes([rest[2], rest[3]]) as usize);
                    if tag == 1 && len == 4 && rest.len() >= 8 {
                        push(
                            &mut info,
                            "Save for Web Quality",
                            u32::from_be_bytes(rest[4..8].try_into().unwrap_or_default()).to_string(),
                        );
                    }
                    if tag == 0 {
                        break;
                    }
                    rest = rest.get(4 + len..).unwrap_or_default();
                }
            }
            0xfe => push(&mut info, "Comment", String::from_utf8_lossy(data).trim_end_matches('\0')),
            // EXIF and XMP (APP1) have their own sections; others by name.
            0xe1 if data.starts_with(b"Exif\0") || data.starts_with(b"http://ns.adobe.com/xap/1.0/\0") => {}
            0xe0..=0xef => {
                let id: String = data
                    .iter()
                    .take_while(|b| b.is_ascii_graphic() || **b == b' ')
                    .take(32)
                    .map(|&b| b as char)
                    .collect();
                let id = if id.is_empty() { String::new() } else { format!(" {id}") };
                push(&mut info, &format!("APP{}{id}", marker - 0xe0), format!("{} bytes", data.len()));
            }
            _ => {}
        }
        at += 2 + len;
    }
    icc_parts.sort_by_key(|(n, _)| *n);
    let icc = (!icc_parts.is_empty()).then(|| icc_parts.into_iter().flat_map(|(_, part)| part).collect());
    (info, iptc, icc)
}

/// The IPTC-IIM block among Photoshop's image resources (resource 0x0404).
fn photoshop_iptc(data: &[u8]) -> Rows {
    photoshop(data).0
}

/// Photoshop's image resources: the IPTC block read, and every resource's
/// number and size.
fn photoshop(mut data: &[u8]) -> (Rows, Vec<(u16, usize)>) {
    let mut iptc_rows = Rows::new();
    let mut resources = Vec::new();
    while data.len() >= 12 && data.starts_with(b"8BIM") {
        let id = u16::from_be_bytes([data[4], data[5]]);
        // A Pascal string, padded to an even length.
        let name_len = data[6] as usize;
        let after_name = 7 + name_len + (name_len + 1) % 2;
        let Some(size) = data.get(after_name..after_name + 4) else { break };
        let size = u32::from_be_bytes(size.try_into().unwrap_or_default()) as usize;
        let start = after_name + 4;
        let Some(body) = data.get(start..start + size) else { break };
        if id == 0x0404 {
            iptc_rows = iptc(body);
        }
        resources.push((id, size));
        data = data.get(start + size + size % 2..).unwrap_or_default();
    }
    (iptc_rows, resources)
}

/// Names of the common Photoshop image resources.
fn photoshop_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x03e9 => "Print Info",
        0x03ed => "Resolution Info",
        0x03f3 => "Print Flags",
        0x03f5 => "Color Halftoning Info",
        0x03f8 => "Color Transfer Functions",
        0x0404 => "IPTC-NAA",
        0x0406 => "JPEG Quality",
        0x0408 => "Grid and Guides",
        0x0409 | 0x040c => "Thumbnail",
        0x040a => "Copyright Flag",
        0x040b => "URL",
        0x040d => "Global Angle",
        0x040f => "ICC Profile",
        0x0410 => "Watermark",
        0x0411 => "ICC Untagged",
        0x0412 => "Effects Visible",
        0x0414 => "Document ID Seed",
        0x0419 => "Global Altitude",
        0x041a => "Slices",
        0x041e => "URL List",
        0x0421 => "Version Info",
        0x0422 => "EXIF Data 1",
        0x0423 => "EXIF Data 3",
        0x0424 => "XMP",
        0x0425 => "IPTC Digest",
        0x0426 => "Print Scale",
        0x0428 => "Pixel Aspect Ratio",
        0x043a => "Print Information",
        0x043b => "Print Style",
        0x2710 => "Print Flags Info",
        _ => return None,
    })
}

/// IPTC-IIM datasets; repeated ones (keywords) joined.
fn iptc(mut data: &[u8]) -> Rows {
    let mut rows = Rows::new();
    let mut utf8 = false;
    while data.len() >= 5 && data[0] == 0x1c {
        let (record, dataset) = (data[1], data[2]);
        let size = u16::from_be_bytes([data[3], data[4]]) as usize;
        // Sizes over 32 KiB use an extended length; nothing worth showing does.
        if size & 0x8000 != 0 {
            break;
        }
        let Some(value) = data.get(5..5 + size) else { break };
        data = &data[5 + size..];
        if (record, dataset) == (1, 90) {
            // ESC % G: the text is UTF-8.
            utf8 = value == b"\x1b%G";
            push(&mut rows, "Coded Character Set", if utf8 { "UTF-8".to_owned() } else { format!("{value:02X?}") });
            continue;
        }
        // Numbers: the records' version datasets.
        if dataset == 0 && value.len() == 2 {
            let label = if record == 1 { "Envelope Record Version" } else { "Record Version" };
            push(&mut rows, label, u16::from_be_bytes([value[0], value[1]]).to_string());
            continue;
        }
        let text = if utf8 || std::str::from_utf8(value).is_ok() {
            String::from_utf8_lossy(value).into_owned()
        } else {
            value.iter().map(|&b| b as char).collect()
        };
        let label = match record {
            2 => iptc_name(dataset).map_or_else(|| format!("IPTC 2:{dataset}"), str::to_owned),
            _ => format!("IPTC {record}:{dataset}"),
        };
        match rows.iter_mut().find(|(l, _)| *l == label) {
            Some((_, v)) => {
                v.push_str(", ");
                v.push_str(text.trim());
            }
            None => push(&mut rows, &label, text),
        }
    }
    rows
}

/// Names of IPTC-IIM record 2, the application record.
fn iptc_name(dataset: u8) -> Option<&'static str> {
    Some(match dataset {
        3 => "Object Type",
        4 => "Object Attribute",
        5 => "Title",
        7 => "Edit Status",
        8 => "Editorial Update",
        10 => "Urgency",
        12 => "Subject Reference",
        15 => "Category",
        20 => "Supplemental Categories",
        22 => "Fixture Identifier",
        25 => "Keywords",
        26 => "Content Location Code",
        27 => "Content Location Name",
        30 => "Release Date",
        35 => "Release Time",
        37 => "Expiration Date",
        38 => "Expiration Time",
        40 => "Special Instructions",
        42 => "Action Advised",
        45 => "Reference Service",
        47 => "Reference Date",
        50 => "Reference Number",
        55 => "Date Created",
        60 => "Time Created",
        62 => "Digital Creation Date",
        63 => "Digital Creation Time",
        65 => "Originating Program",
        70 => "Program Version",
        75 => "Object Cycle",
        80 => "By-line",
        85 => "By-line Title",
        90 => "City",
        92 => "Sub-location",
        95 => "Province/State",
        100 => "Country Code",
        101 => "Country",
        103 => "Original Transmission Reference",
        105 => "Headline",
        110 => "Credit",
        115 => "Source",
        116 => "Copyright Notice",
        118 => "Contact",
        120 => "Caption/Abstract",
        121 => "Local Caption",
        122 => "Writer/Editor",
        130 => "Image Type",
        131 => "Image Orientation",
        135 => "Language",
        _ => return None,
    })
}

/// The ICC profile's header and its description and copyright.
fn icc_profile(p: &[u8]) -> Rows {
    let mut rows = Rows::new();
    if p.len() < 132 || &p[36..40] != b"acsp" {
        return rows;
    }
    let text4 = |at: usize| String::from_utf8_lossy(&p[at..at + 4]).trim_matches(['\0', ' ']).to_owned();
    let u32_at = |at: usize| p.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap_or_default()));
    // Tags: description, copyright, device maker and model.
    let count = u32_at(128).unwrap_or(0).min(256) as usize;
    let tag = |wanted: &[u8; 4]| {
        (0..count).find_map(|i| {
            let at = 132 + i * 12;
            (p.get(at..at + 4)? == wanted).then(|| {
                let (offset, size) = (u32_at(at + 4)? as usize, u32_at(at + 8)? as usize);
                icc_text(p.get(offset..offset + size)?)
            })?
        })
    };
    push(&mut rows, "Description", tag(b"desc").unwrap_or_default());
    let class = match &p[12..16] {
        b"scnr" => "Input device",
        b"mntr" => "Display",
        b"prtr" => "Output device",
        b"link" => "Device link",
        b"spac" => "Color space",
        b"abst" => "Abstract",
        b"nmcl" => "Named color",
        _ => "",
    };
    push(&mut rows, "Class", class);
    push(&mut rows, "Color Space", text4(16));
    push(&mut rows, "Connection Space", text4(20));
    push(&mut rows, "Version", format!("{}.{}", p[8], p[9] >> 4));
    let date = |at: usize| u16::from_be_bytes([p[at], p[at + 1]]);
    push(
        &mut rows,
        "Date",
        format!("{}-{:02}-{:02} {:02}:{:02}:{:02}", date(24), date(26), date(28), date(30), date(32), date(34)),
    );
    push(&mut rows, "CMM", text4(4));
    push(&mut rows, "Platform", text4(40));
    push(&mut rows, "Device Maker", text4(48));
    push(&mut rows, "Device Model", tag(b"dmdd").unwrap_or_default());
    push(&mut rows, "Copyright", tag(b"cprt").unwrap_or_default());
    push(&mut rows, "Size", format!("{} bytes", u32_at(0).unwrap_or(0)));
    let tags: Vec<String> = (0..count)
        .filter_map(|i| p.get(132 + i * 12..136 + i * 12))
        .map(|sig| String::from_utf8_lossy(sig).trim().to_owned())
        .collect();
    push(&mut rows, "Tags", tags.join(", "));
    rows
}

/// ICC text types: `desc` (ASCII), `text`, and `mluc` (UTF-16, first record).
fn icc_text(data: &[u8]) -> Option<String> {
    let ascii = |b: &[u8]| String::from_utf8_lossy(b).trim_end_matches('\0').trim().to_owned();
    match data.get(..4)? {
        b"desc" => {
            let len = u32::from_be_bytes(data.get(8..12)?.try_into().ok()?) as usize;
            Some(ascii(data.get(12..12 + len)?))
        }
        b"text" => Some(ascii(data.get(8..)?)),
        b"mluc" => {
            let len = u32::from_be_bytes(data.get(20..24)?.try_into().ok()?) as usize;
            let offset = u32::from_be_bytes(data.get(24..28)?.try_into().ok()?) as usize;
            let units: Vec<u16> =
                data.get(offset..offset + len)?.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_owned())
        }
        _ => None,
    }
}

/// PNG's header, time, density, gamma and text chunks; IPTC and the ICC
/// profile, also where ImageMagick keeps them as "Raw profile type" text.
fn png(file: &[u8]) -> (Rows, Rows, Option<Vec<u8>>) {
    let mut rows = Rows::new();
    let mut iptc_rows = Rows::new();
    let mut icc = None;
    let mut others: Vec<String> = Vec::new();
    for (kind, data) in png_chunks(file) {
        match &kind {
            b"IHDR" if data.len() >= 13 => {
                let size = |at: usize| u32::from_be_bytes(data[at..at + 4].try_into().unwrap_or_default());
                push(&mut rows, "Size", format!("{} × {}", size(0), size(4)));
                push(&mut rows, "Bit Depth", data[8].to_string());
                let color = match data[9] {
                    0 => "Grayscale",
                    2 => "RGB",
                    3 => "Palette",
                    4 => "Grayscale with alpha",
                    6 => "RGB with alpha",
                    _ => "",
                };
                push(&mut rows, "Color Type", color);
                push(&mut rows, "Interlace", if data[12] == 1 { "Adam7" } else { "None" });
            }
            b"tIME" if data.len() >= 7 => {
                let year = u16::from_be_bytes([data[0], data[1]]);
                let time = format!("{year}-{:02}-{:02} {:02}:{:02}:{:02}", data[2], data[3], data[4], data[5], data[6]);
                push(&mut rows, "Modified", time);
            }
            b"pHYs" if data.len() >= 9 => {
                let per_unit = |at: usize| u32::from_be_bytes(data[at..at + 4].try_into().unwrap_or_default());
                let (x, y) = (per_unit(0), per_unit(4));
                let density = if data[8] == 1 {
                    format!("{:.0} × {:.0} dots per inch", x as f64 * 0.0254, y as f64 * 0.0254)
                } else {
                    format!("{x}:{y} (aspect ratio)")
                };
                push(&mut rows, "Density", density);
            }
            b"gAMA" if data.len() >= 4 => {
                let gamma = u32::from_be_bytes(data[..4].try_into().unwrap_or_default()) as f64 / 100_000.0;
                push(&mut rows, "Gamma", format!("{gamma}"));
            }
            b"sRGB" if !data.is_empty() => {
                let intent = ["Perceptual", "Relative colorimetric", "Saturation", "Absolute colorimetric"];
                push(&mut rows, "sRGB Intent", intent.get(data[0] as usize).copied().unwrap_or(""));
            }
            b"iCCP" => {
                let Some(name_end) = data.iter().position(|&b| b == 0) else { continue };
                icc = data.get(name_end + 2..).and_then(inflate);
            }
            b"tEXt" | b"zTXt" | b"iTXt" => {
                let Some((key, text)) = png_text(&kind, data) else { continue };
                match key.strip_prefix("Raw profile type ") {
                    Some("iptc" | "8bim") => {
                        let bytes = raw_profile(&text).unwrap_or_default();
                        let found = photoshop_iptc(&bytes);
                        iptc_rows.extend(if found.is_empty() { iptc(&bytes) } else { found });
                    }
                    Some("icc" | "icm") => icc = raw_profile(&text),
                    // EXIF and XMP have their own sections.
                    Some(_) => {}
                    None if key == "XML:com.adobe.xmp" => {}
                    None => push(&mut rows, &key, text),
                }
            }
            // The image data and the end say nothing.
            b"IDAT" | b"IEND" | b"eXIf" => {}
            _ => others.push(format!("{} ({} bytes)", String::from_utf8_lossy(&kind), data.len())),
        }
    }
    push(&mut rows, "Other Chunks", others.join(", "));
    (rows, iptc_rows, icc)
}

/// ImageMagick's "Raw profile type" text: a line with the kind, one with the
/// length, then the bytes in hex.
pub(crate) fn raw_profile(text: &str) -> Option<Vec<u8>> {
    let mut lines = text.trim_start().lines();
    let _kind = lines.next()?;
    let len: usize = lines.next()?.trim().parse().ok()?;
    let hex: String = lines.flat_map(|l| l.chars()).filter(char::is_ascii_hexdigit).collect();
    let bytes: Vec<u8> =
        (0..hex.len() / 2).filter_map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()).take(len).collect();
    (bytes.len() == len).then_some(bytes)
}

/// A PNG text chunk's keyword and text: `tEXt` is Latin-1, `zTXt` compressed
/// Latin-1, `iTXt` UTF-8 and maybe compressed.
pub(crate) fn png_text(kind: &[u8; 4], data: &[u8]) -> Option<(String, String)> {
    let key_end = data.iter().position(|&b| b == 0)?;
    let key: String = data[..key_end].iter().map(|&b| b as char).collect();
    let rest = &data[key_end + 1..];
    let latin1 = |b: &[u8]| b.iter().map(|&c| c as char).collect::<String>();
    let text = match kind {
        b"tEXt" => latin1(rest),
        b"zTXt" => latin1(&inflate(rest.get(1..)?)?),
        _ => {
            let (compressed, rest) = (*rest.first()? == 1, rest.get(2..)?);
            // Language tag and translated keyword, each zero-terminated.
            let lang_end = rest.iter().position(|&b| b == 0)?;
            let rest = &rest[lang_end + 1..];
            let translated_end = rest.iter().position(|&b| b == 0)?;
            let body = &rest[translated_end + 1..];
            let body = if compressed { inflate(body)? } else { body.to_vec() };
            String::from_utf8_lossy(&body).into_owned()
        }
    };
    Some((key, text))
}

fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, 16 * 1024 * 1024).ok()
}

/// WebP's kind and canvas size.
fn webp(file: &[u8], header: &[u8]) -> Rows {
    let mut rows = Rows::new();
    let extended = riff_chunk(file, b"VP8X").is_some();
    if extended && header.len() >= 10 {
        let side = |at: usize| u32::from_le_bytes([header[at], header[at + 1], header[at + 2], 0]) + 1;
        push(&mut rows, "Canvas", format!("{} × {}", side(4), side(7)));
        let flags = header[0];
        push(&mut rows, "Animated", if flags & 0x02 != 0 { "Yes" } else { "No" });
        push(&mut rows, "Alpha", if flags & 0x10 != 0 { "Yes" } else { "No" });
    }
    let kind = if riff_chunk(file, b"VP8L").is_some() { "Lossless (VP8L)" } else { "Lossy (VP8)" };
    push(&mut rows, "Compression", kind);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_iso6709_positions() {
        assert_eq!(iso6709("+59.4370+024.7536+012.000/"), Some((59.437, 24.7536)));
        assert_eq!(iso6709("-33.8700+151.2100/"), Some((-33.87, 151.21)));
        assert_eq!(iso6709("2022-04-21"), None);
        assert_eq!(iso6709("+1"), None);
    }

    #[test]
    fn reads_iptc_datasets() {
        let mut block = Vec::new();
        let mut dataset = |record: u8, number: u8, text: &[u8]| {
            block.extend_from_slice(&[0x1c, record, number]);
            block.extend_from_slice(&(text.len() as u16).to_be_bytes());
            block.extend_from_slice(text);
        };
        dataset(1, 90, b"\x1b%G");
        dataset(2, 5, "Закат".as_bytes());
        dataset(2, 25, b"sea");
        dataset(2, 25, b"sun");
        dataset(2, 90, b"Tallinn");
        let rows = iptc(&block);
        let get = |l: &str| rows.iter().find(|(k, _)| k == l).map(|(_, v)| v.as_str());
        assert_eq!(get("Title"), Some("Закат"));
        assert_eq!(get("Keywords"), Some("sea, sun"));
        assert_eq!(get("City"), Some("Tallinn"));
    }

    #[test]
    fn reads_imagemagick_raw_profiles() {
        let text = "\niptc\n       5\n1c02190002\n";
        assert_eq!(raw_profile(text), Some(vec![0x1c, 0x02, 0x19, 0x00, 0x02]));
        assert_eq!(raw_profile("\niptc\n       9\n1c02190002\n"), None, "shorter than it says");
    }

    #[test]
    fn reads_png_text_chunks() {
        assert_eq!(png_text(b"tEXt", b"Author\0Ann"), Some(("Author".into(), "Ann".into())));
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib("Привет".as_bytes(), 6);
        let mut itxt = b"Comment\0\x01\0ru\0\0".to_vec();
        itxt.extend_from_slice(&compressed);
        assert_eq!(png_text(b"iTXt", &itxt), Some(("Comment".into(), "Привет".into())));
    }
}
