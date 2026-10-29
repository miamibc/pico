//! The maker's notes (EXIF MakerNote): every camera maker's own directory of
//! settings. Their layouts follow ExifTool's tag tables; what is known is
//! spelt out, the rest shown as numbers, text or hex.

use crate::exif::{Tiff, Value, exposure, hex, lens, plain, trim_number};
use std::collections::BTreeMap;

/// Named fields read out of a list of numbers.
type Fields = Vec<(&'static str, String)>;

/// The `length` bytes of maker's notes at `at` in the TIFF block: what they
/// are ("Canon, 17 entries") and their entries, named and readable. Most
/// makers write a TIFF directory, after a header of their own; `None` for
/// the ones that do not.
pub(crate) fn read(
    tiff: &[u8],
    little: bool,
    at: usize,
    length: usize,
    make: &str,
    model: &str,
) -> Option<(String, Vec<(String, String)>)> {
    let note = tiff.get(at..at + length)?;
    let maker = make.split_whitespace().next().unwrap_or("Maker").to_owned();
    // Where the directory is, and what its offsets count from.
    let (data, little, ifd) = if note.starts_with(b"Nikon\0\x02") {
        // An embedded TIFF block of its own, after 10 bytes.
        let inner = note.get(10..)?;
        let little = inner.starts_with(b"II");
        (inner, little, Tiff { data: inner, little }.u32(4)? as usize)
    } else if note.starts_with(b"OLYMPUS\0") || note.starts_with(b"OM SYSTEM\0") {
        let header = if note.starts_with(b"OLYMPUS") { 12 } else { 16 };
        (note, note.get(header - 4..header - 2)? == b"II", header)
    } else if note.starts_with(b"Apple iOS\0") {
        // Big-endian, offsets from the notes' start, after 14 bytes.
        (note, false, 14)
    } else if note.starts_with(b"FUJIFILM") {
        (note, true, Tiff { data: note, little: true }.u32(8)? as usize)
    } else if note.starts_with(b"QVC\0") {
        // Casio's newer notes.
        (tiff, little, at + 6)
    } else if note.starts_with(b"Panasonic\0") || note.starts_with(b"SONY DSC ") || note.starts_with(b"SONY CAM ") {
        (tiff, little, at + 12)
    } else if note.starts_with(b"Nikon\0\x01") || note.starts_with(b"OLYMP\0") || note.starts_with(b"EPSON\0") {
        (tiff, little, at + 8)
    } else {
        // Canon and many others: a bare directory.
        (tiff, little, at)
    };
    // Canon ends its notes with where they first were. An editor that moved
    // them left the offsets inside pointing there: count from the old place.
    let moved;
    let (data, ifd) = match canon_footer(note, make) {
        Some(was) if was > at => {
            moved = [vec![0; was - at], tiff.to_vec()].concat();
            (moved.as_slice(), was)
        }
        Some(was) if was < at => (&tiff[at - was..], was),
        _ => (data, ifd),
    };
    // Editors that rewrite EXIF in the other byte order often leave the
    // notes as they were: the order that gives a sensible directory wins.
    let sensible = |little: bool| {
        let count = Tiff { data, little }.u16(ifd).map_or(0, usize::from);
        (1..=500).contains(&count) && ifd + 2 + count * 12 <= data.len()
    };
    let little = if sensible(little) { little } else { !little };
    let reader = Tiff { data, little };
    let count = reader.u16(ifd)? as usize;
    if !sensible(little) {
        return None;
    }
    let mut tags = BTreeMap::new();
    reader.read_ifd(ifd, &mut tags, &mut Vec::new());
    let mut rows: Vec<(String, String)> = tags
        .iter()
        .filter_map(|(&tag, value)| {
            let name = maker_tag_name(&maker, tag)
                .or_else(|| known(&maker, 0, tag).map(|t| t.name))
                .map_or_else(|| format!("Tag 0x{tag:04X}"), str::to_owned);
            let text = maker_value(&maker, tag, value, little)
                .or_else(|| meaning(&maker, 0, tag, value))
                .or_else(|| maker_plain(value))?;
            let text = text.trim().to_owned();
            (!text.is_empty()).then(|| (format!("{maker} {name}"), text))
        })
        .collect();
    // Olympus keeps its newer settings in directories of their own, at
    // offsets counted as the notes' are; each read out in place of its bytes.
    if maker.eq_ignore_ascii_case("olympus") || maker.eq_ignore_ascii_case("om") {
        for sub in [0x2010, 0x2020, 0x2030, 0x2040, 0x2050] {
            let Some(at) = entry_offset(&reader, ifd, sub) else { continue };
            let mut found = BTreeMap::new();
            reader.read_ifd(at, &mut found, &mut Vec::new());
            if found.is_empty() {
                continue;
            }
            let Some(name) = maker_tag_name(&maker, sub) else { continue };
            let label = format!("{maker} {name}");
            let Some(place) = rows.iter().position(|(l, _)| *l == label) else { continue };
            rows[place].1 = format!("{} entries", found.len());
            let inner = found.iter().filter_map(|(&tag, value)| {
                let name = olympus_name(sub, tag).map_or_else(|| format!("{name} 0x{tag:04X}"), str::to_owned);
                let text = olympus_value(sub, tag, value).or_else(|| maker_plain(value))?;
                let text = text.trim().to_owned();
                (!text.is_empty()).then(|| (format!("{maker} {name}"), text))
            });
            rows.splice(place + 1..place + 1, inner.collect::<Vec<_>>());
        }
    }
    // Lists of numbers (Canon's settings and shot info) spelt out, each under
    // its list.
    let (spelt, sound) = spell(&maker, model, &tags);
    for (tag, spelt) in spelt {
        let raw = maker_tag_name(&maker, tag).map(|name| format!("{maker} {name}"));
        let under = rows.iter().position(|(name, _)| Some(name) == raw.as_ref()).map_or(rows.len(), |i| i + 1);
        rows.splice(under..under, spelt.into_iter().map(|(name, v)| (format!("{maker} {name}"), v)));
    }
    let mut summary = format!("{maker}, {count} entries");
    if !sound {
        // The lists are not where the offsets say: an editor moved the notes
        // without fixing them. Worth knowing in itself.
        summary += "; their offsets point to the wrong place, as after an editor rewrote the file";
    }
    Some((summary, rows))
}

/// Where the value of entry `tag` of the directory at `ifd` is: the offset
/// it holds.
fn entry_offset(reader: &Tiff, ifd: usize, tag: u32) -> Option<usize> {
    let count = reader.u16(ifd)? as usize;
    (0..count)
        .map(|i| ifd + 2 + i * 12)
        .find(|&entry| reader.u16(entry).map(u32::from) == Some(tag))
        .and_then(|entry| reader.u32(entry + 8))
        .map(|offset| offset as usize)
}

/// Names in Olympus's directories (Equipment, Camera Settings…), as
/// ExifTool has them.
fn olympus_name(sub: u32, tag: u32) -> Option<&'static str> {
    Some(match (sub, tag) {
        (0x2010, 0x0000) => "Equipment Version",
        (0x2010, 0x0100) => "Camera Type 2",
        (0x2010, 0x0101) => "Serial Number",
        (0x2010, 0x0102) => "Internal Serial Number",
        (0x2010, 0x0103) => "Focal Plane Diagonal",
        (0x2010, 0x0104) => "Body Firmware Version",
        (0x2010, 0x0201) => "Lens Type",
        (0x2010, 0x0202) => "Lens Serial Number",
        (0x2010, 0x0203) => "Lens Model",
        (0x2010, 0x0204) => "Lens Firmware Version",
        (0x2010, 0x0205) => "Max Aperture At Min Focal",
        (0x2010, 0x0206) => "Max Aperture At Max Focal",
        (0x2010, 0x0207) => "Min Focal Length",
        (0x2010, 0x0208) => "Max Focal Length",
        (0x2010, 0x020a) => "Max Aperture",
        (0x2010, 0x0301) => "Extender",
        (0x2010, 0x1000) => "Flash Type",
        (0x2010, 0x1001) => "Flash Model",
        (0x2010, 0x1002) => "Flash Firmware Version",
        (0x2010, 0x1003) => "Flash Serial Number",
        (0x2020, 0x0000) => "Camera Settings Version",
        (0x2020, 0x0100) => "Preview Image Valid",
        (0x2020, 0x0101) => "Preview Image Start",
        (0x2020, 0x0102) => "Preview Image Length",
        (0x2020, 0x0200) => "Exposure Mode",
        (0x2020, 0x0201) => "AE Lock",
        (0x2020, 0x0202) => "Metering Mode",
        (0x2020, 0x0203) => "Exposure Shift",
        (0x2020, 0x0300) => "Macro Mode",
        (0x2020, 0x0301) => "Focus Mode",
        (0x2020, 0x0302) => "Focus Process",
        (0x2020, 0x0303) => "AF Search",
        (0x2020, 0x0304) => "AF Areas",
        (0x2020, 0x0400) => "Flash Mode",
        (0x2020, 0x0401) => "Flash Exposure Compensation",
        (0x2020, 0x0500) => "White Balance 2",
        (0x2020, 0x0501) => "White Balance Temperature",
        (0x2020, 0x0502) => "White Balance Bracket",
        (0x2020, 0x0503) => "Custom Saturation",
        (0x2020, 0x0504) => "Modified Saturation",
        (0x2020, 0x0505) => "Contrast Setting",
        (0x2020, 0x0506) => "Sharpness Setting",
        (0x2020, 0x0507) => "Color Space",
        (0x2020, 0x0509) => "Scene Mode",
        (0x2020, 0x050a) => "Noise Reduction",
        (0x2020, 0x050b) => "Distortion Correction",
        (0x2020, 0x050c) => "Shading Compensation",
        (0x2020, 0x0600) => "Drive Mode",
        (0x2020, 0x0603) => "Image Quality 2",
        (0x2020, 0x0604) => "Image Stabilization",
        (0x2030, 0x0000) => "Raw Development Version",
        (0x2040, 0x0000) => "Image Processing Version",
        (0x2040, 0x0100) => "WB RB Levels",
        (0x2040, 0x0200) => "Color Matrix",
        (0x2040, 0x0611) => "Valid Bits",
        (0x2040, 0x1010) => "Noise Reduction 2",
        (0x2050, 0x0000) => "Focus Info Version",
        (0x2050, 0x0209) => "Auto Focus",
        (0x2050, 0x0305) => "Focus Distance",
        (0x2050, 0x0308) => "AF Point",
        (0x2050, 0x1201) => "External Flash",
        (0x2050, 0x1500) => "Sensor Temperature",
        (0x2050, 0x1600) => "Image Stabilization",
        _ => return None,
    })
}

/// Olympus's directory entries that read better spelt out.
fn olympus_value(sub: u32, tag: u32, value: &Value) -> Option<String> {
    let first = value.number(0)? as u32;
    let pick = |names: &[(u32, &str)]| {
        Some(names.iter().find(|(n, _)| *n == first).map_or_else(|| first.to_string(), |(_, name)| (*name).to_owned()))
    };
    match (sub, tag) {
        (0x2020, 0x0200) => pick(&[
            (1, "Manual"),
            (2, "Program"),
            (3, "Aperture-priority AE"),
            (4, "Shutter speed priority AE"),
            (5, "Program-shift"),
        ]),
        (0x2020, 0x0202) => pick(&[
            (2, "Center-weighted average"),
            (3, "Spot"),
            (5, "ESP"),
            (261, "Pattern + AF"),
            (515, "Spot + Highlight control"),
            (1027, "Spot + Shadow control"),
        ]),
        (0x2020, 0x0300) => pick(&[(0, "Off"), (1, "On"), (2, "Super Macro")]),
        (0x2020, 0x0301) => pick(&[
            (0, "Single AF"),
            (1, "Sequential shooting AF"),
            (2, "Continuous AF"),
            (3, "Multi AF"),
            (4, "Face detect"),
            (10, "MF"),
        ]),
        (0x2020, 0x0507) => pick(&[(0, "sRGB"), (1, "Adobe RGB"), (2, "Pro Photo RGB")]),
        (0x2020, 0x0603) => pick(&[(1, "SQ"), (2, "HQ"), (3, "SHQ"), (4, "RAW")]),
        (0x2050, 0x0305) => value.number(0).map(|m| format!("{} m", trim_number(m))),
        (0x2050, 0x1500) => value.number(0).map(|c| format!("{} °C", trim_number(c))),
        _ => None,
    }
}

/// The lists in the notes whose places mean something, read out: by the tag
/// of the list, the fields under it; and whether the lists are sound (Canon's
/// start with their own length; garbage there means the offsets are broken,
/// and nothing is read out).
fn spell(maker: &str, model: &str, tags: &BTreeMap<u32, Value>) -> (Vec<(u32, Fields)>, bool) {
    // Canon's lists begin with their size in bytes.
    let numbers = |tag: u32| match tags.get(&tag) {
        Some(Value::Numbers(n)) => Some(n.as_slice()),
        _ => None,
    };
    let sized = |list: &[u32]| list.first().is_some_and(|&size| size as usize == list.len() * 2);
    let mut out = Vec::new();
    if maker.eq_ignore_ascii_case("canon") {
        let sound = [0x0001, 0x0004].iter().filter_map(|&tag| numbers(tag)).all(sized);
        if !sound {
            return (out, false);
        }
        let settings = numbers(0x0001);
        if let Some(settings) = settings {
            out.push((0x0001, canon_camera_settings(settings)));
        }
        // Focal lengths count in focal units per mm, from the settings.
        let units = settings.and_then(|s| s.get(25)).copied().filter(|&u| u > 0 && u != 0x7fff).unwrap_or(1);
        if let Some(focal) = numbers(0x0002) {
            let mut fields = Vec::new();
            if let Some(kind) = focal.first().and_then(|&k| [(1, "Fixed"), (2, "Zoom")].iter().find(|(n, _)| *n == k)) {
                fields.push(("Focal Type", kind.1.to_owned()));
            }
            if let Some(&f) = focal.get(1).filter(|&&f| f > 0) {
                fields.push(("Focal Length", format!("{:.1} mm", f as f64 / units as f64)));
            }
            out.push((0x0002, fields));
        }
        if let Some(shot) = numbers(0x0004) {
            out.push((0x0004, canon_shot_info(shot, model)));
        }
    }
    // Lists only the tables know: each place, named, with its meaning.
    for (&tag, value) in tags {
        // List 0 is the main directory's table, not a list's.
        let Some(list) = u16::try_from(tag).ok().filter(|&l| l != 0) else { continue };
        let (Value::Numbers(values), Some(table)) = (value, crate::maker_tags::table(&key(maker), list)) else {
            continue;
        };
        if out.iter().any(|(t, _)| *t == tag) {
            continue;
        }
        let fields = table
            .iter()
            .filter_map(|entry| {
                let v = *values.get(entry.tag as usize)? as u16 as i16 as i64;
                let text =
                    entry.values.iter().find(|(n, _)| *n == v).map_or_else(|| v.to_string(), |(_, m)| (*m).to_owned());
                Some((entry.name, text))
            })
            .collect();
        out.push((tag, fields));
    }
    (out, true)
}

/// The maker's name as the tables have it: `CANON`, `NIKON`, `OLYMPUS`…
fn key(maker: &str) -> String {
    maker.trim_end_matches([',', '.']).to_ascii_uppercase()
}

/// The tables' entry for `tag` in the maker's main directory (`list` 0) or
/// in its list `list`.
fn known(maker: &str, list: u16, tag: u32) -> Option<&'static crate::maker_tags::Tag> {
    crate::maker_tags::table(&key(maker), list)?.iter().find(|t| u32::from(t.tag) == tag)
}

/// What a single value means, as the tables have it.
fn meaning(maker: &str, list: u16, tag: u32, value: &Value) -> Option<String> {
    if value.count() != 1 {
        return None;
    }
    let number = value.number(0)? as i64;
    let entry = known(maker, list, tag)?;
    entry.values.iter().find(|(n, _)| *n == number).map(|(_, m)| (*m).to_owned())
}

/// Canon's way of writing EV: in 1/32 steps, with 1/3 and 2/3 stops coded
/// as 12/32 and 20/32.
fn canon_ev(value: i32) -> f64 {
    let sign = if value < 0 { -1.0 } else { 1.0 };
    let value = value.abs();
    let fraction = value & 0x1f;
    let whole = (value - fraction) as f64;
    let fraction = match fraction {
        0x0c => 32.0 / 3.0,
        0x14 => 64.0 / 3.0,
        f => f as f64,
    };
    sign * (whole + fraction) / 32.0
}

/// Canon's Shot Info, by place in the list, as ExifTool reads it.
fn canon_shot_info(values: &[u32], model: &str) -> Vec<(&'static str, String)> {
    // 0x7fff and similar mean "not known" in Canon's lists.
    let at = |i: usize| values.get(i).map(|&v| v as u16 as i16 as i32).filter(|&v| v != 0x7fff);
    let pick = |i: usize, names: &[(i32, &'static str)]| {
        let v = at(i)?;
        Some(names.iter().find(|(n, _)| *n == v).map_or_else(|| v.to_string(), |(_, name)| (*name).to_owned()))
    };
    let iso = |v: i32| (2f64.powf(v as f64 / 32.0) * 100.0 / 32.0).round();
    let auto = at(1).map(|v| 2f64.powf(v as f64 / 32.0) * 100.0);
    let base = at(2).filter(|&v| v != 0).map(iso);
    // Only what a camera could have done.
    let aperture =
        |v: i32| Some(2f64.powf(canon_ev(v) / 2.0)).filter(|f| (0.5..=128.0).contains(f)).map(|f| format!("f/{f:.1}"));
    let seconds = |v: i32| Some(2f64.powf(-canon_ev(v))).filter(|s| (1e-5..=3600.0).contains(s));
    let time = |v: i32| seconds(v).map(exposure);
    // The 20D and 350D count the exposure time in other units.
    let odd = ["20D", "350D", "REBEL XT", "Kiss Digital N"].iter().any(|m| model.contains(m));
    let taken = |v: i32| seconds(v).map(|s| if odd { s * 1000.0 / 32.0 } else { s }).map(exposure);
    let out = vec![
        // What the camera worked with: the base sensitivity, raised by Auto ISO.
        ("ISO", base.map(|b| trim_number((b * auto.unwrap_or(100.0) / 100.0).round()))),
        ("Base ISO", base.map(trim_number)),
        ("Auto ISO", auto.filter(|a| (a - 100.0).abs() > 0.5).map(|a| format!("{a:.0}%"))),
        ("Measured EV", at(3).map(|v| format!("{:.2}", v as f64 / 32.0 + 5.0))),
        ("Target Aperture", at(4).filter(|&v| v != 0).and_then(aperture)),
        ("Target Exposure Time", at(5).filter(|&v| v != 0).and_then(time)),
        ("Exposure Compensation", at(6).map(|v| format!("{:+.1} EV", canon_ev(v)))),
        (
            "White Balance",
            pick(
                7,
                &[
                    (0, "Auto"),
                    (1, "Daylight"),
                    (2, "Cloudy"),
                    (3, "Tungsten"),
                    (4, "Fluorescent"),
                    (5, "Flash"),
                    (6, "Custom"),
                    (7, "Black & white"),
                    (8, "Shade"),
                    (9, "Manual temperature (Kelvin)"),
                    (14, "Daylight fluorescent"),
                    (17, "Under water"),
                ],
            ),
        ),
        ("Slow Shutter", pick(8, &[(0, "Off"), (1, "Night scene"), (2, "On"), (3, "None")])),
        ("Sequence Number", at(9).map(|v| v.to_string())),
        ("Camera Temperature", at(12).filter(|&v| v != 0).map(|v| format!("{} °C", v - 128))),
        ("Flash Guide Number", at(13).filter(|&v| v != 0).map(|v| trim_number(v as f64 / 32.0))),
        ("Flash Exposure Compensation", at(15).map(|v| format!("{:+.1} EV", canon_ev(v)))),
        (
            "Auto Exposure Bracketing",
            pick(16, &[(-1, "On"), (0, "Off"), (1, "On (shot 1)"), (2, "On (shot 2)"), (3, "On (shot 3)")]),
        ),
        ("Control Mode", pick(18, &[(0, "n/a"), (1, "Camera local control"), (3, "Computer remote control")])),
        (
            "Focus Distance Upper",
            at(19).filter(|&v| v > 0).map(|v| {
                if v == 0xffff_u16 as i16 as i32 {
                    "Infinity".into()
                } else {
                    format!("{} m", trim_number(v as u16 as f64 / 100.0))
                }
            }),
        ),
        (
            "Focus Distance Lower",
            at(20).filter(|&v| v > 0).map(|v| format!("{} m", trim_number(v as u16 as f64 / 100.0))),
        ),
        ("F-Number", at(21).filter(|&v| v != 0).and_then(aperture)),
        ("Exposure Time", at(22).filter(|&v| v != 0).and_then(taken)),
        (
            "Camera Type",
            pick(26, &[(248, "EOS high-end"), (250, "Compact"), (252, "EOS mid-range"), (255, "DV camera")]),
        ),
        (
            "Auto Rotate",
            pick(27, &[(-1, "Unknown"), (0, "None"), (1, "Rotate 90 CW"), (2, "Rotate 180"), (3, "Rotate 270 CW")]),
        ),
        ("ND Filter", pick(28, &[(-1, "n/a"), (0, "Off"), (1, "On")])),
        ("Self Timer 2", at(29).filter(|&v| v > 0).map(|v| format!("{} s", trim_number(v as f64 / 10.0)))),
    ];
    out.into_iter().filter_map(|(name, value)| Some((name, value?))).collect()
}

/// Canon's Camera Settings, by their place in the list (as ExifTool reads
/// them); unknown values as numbers.
fn canon_camera_settings(values: &[u32]) -> Vec<(&'static str, String)> {
    // 0x7fff means "not known" in Canon's lists.
    let at = |i: usize| values.get(i).map(|&v| v as u16 as i16 as i32).filter(|&v| v != 0x7fff);
    let pick = |i: usize, names: &[(i32, &'static str)]| {
        let v = at(i)?;
        Some(names.iter().find(|(n, _)| *n == v).map_or_else(|| v.to_string(), |(_, name)| (*name).to_owned()))
    };
    let level = |i: usize| {
        at(i).map(|v| match v {
            0 => "Normal".to_owned(),
            v if v > 0 => format!("+{v}"),
            v => v.to_string(),
        })
    };
    let mut out = vec![
        ("Macro Mode", pick(1, &[(1, "Macro"), (2, "Normal")])),
        ("Self Timer", at(2).map(|v| if v == 0 { "Off".to_owned() } else { format!("{} s", v as f32 / 10.0) })),
        ("Quality", pick(3, &[(1, "Economy"), (2, "Normal"), (3, "Fine"), (4, "RAW"), (5, "Superfine")])),
        (
            "Flash Mode",
            pick(
                4,
                &[
                    (0, "Off"),
                    (1, "Auto"),
                    (2, "On"),
                    (3, "Red-eye reduction"),
                    (4, "Slow-sync"),
                    (5, "Red-eye reduction (Auto)"),
                    (6, "Red-eye reduction (On)"),
                    (16, "External flash"),
                ],
            ),
        ),
        (
            "Drive Mode",
            pick(
                5,
                &[
                    (0, "Single"),
                    (1, "Continuous"),
                    (2, "Movie"),
                    (3, "Continuous, speed priority"),
                    (4, "Continuous, low"),
                    (5, "Continuous, high"),
                    (6, "Silent single"),
                    (9, "Single, silent"),
                    (10, "Continuous, silent"),
                ],
            ),
        ),
        (
            "Focus Mode",
            pick(
                7,
                &[
                    (0, "One-shot AF"),
                    (1, "AI Servo AF"),
                    (2, "AI Focus AF"),
                    (3, "Manual focus"),
                    (4, "Single"),
                    (5, "Continuous"),
                    (6, "Manual focus"),
                ],
            ),
        ),
        (
            "Image Size",
            pick(
                10,
                &[
                    (0, "Large"),
                    (1, "Medium"),
                    (2, "Small"),
                    (5, "Medium 1"),
                    (6, "Medium 2"),
                    (7, "Medium 3"),
                    (8, "Postcard"),
                    (9, "Widescreen"),
                    (10, "Medium widescreen"),
                    (14, "Small 1"),
                    (15, "Small 2"),
                    (16, "Small 3"),
                    (128, "640x480 movie"),
                    (129, "Medium movie"),
                    (130, "Small movie"),
                    (137, "1280x720 movie"),
                    (142, "1920x1080 movie"),
                    (143, "4096x2160 movie"),
                ],
            ),
        ),
        (
            "Easy Mode",
            pick(
                11,
                &[
                    (0, "Full auto"),
                    (1, "Manual"),
                    (2, "Landscape"),
                    (3, "Fast shutter"),
                    (4, "Slow shutter"),
                    (5, "Night"),
                    (6, "Gray scale"),
                    (7, "Sepia"),
                    (8, "Portrait"),
                    (9, "Sports"),
                    (10, "Macro"),
                    (11, "Black & white"),
                    (12, "Pan focus"),
                    (13, "Vivid"),
                    (14, "Neutral"),
                    (15, "Flash off"),
                    (16, "Long shutter"),
                    (17, "Super macro"),
                    (18, "Foliage"),
                    (19, "Indoor"),
                    (20, "Fireworks"),
                    (21, "Beach"),
                    (22, "Underwater"),
                    (23, "Snow"),
                    (24, "Kids & pets"),
                    (25, "Night snapshot"),
                    (26, "Digital macro"),
                    (27, "My colors"),
                ],
            ),
        ),
        ("Digital Zoom", pick(12, &[(0, "None"), (1, "2×"), (2, "4×"), (3, "Other")])),
        ("Contrast", level(13)),
        ("Saturation", level(14)),
        ("Sharpness", level(15)),
        (
            "ISO Setting",
            at(16).map(|v| match v {
                15 => "Auto".to_owned(),
                16 => "50".to_owned(),
                17 => "100".to_owned(),
                18 => "200".to_owned(),
                19 => "400".to_owned(),
                v if v & 0x4000 != 0 => (v & 0x3fff).to_string(),
                v => v.to_string(),
            }),
        ),
        (
            "Metering Mode",
            pick(
                17,
                &[
                    (0, "Default"),
                    (1, "Spot"),
                    (2, "Average"),
                    (3, "Evaluative"),
                    (4, "Partial"),
                    (5, "Center-weighted average"),
                ],
            ),
        ),
        (
            "Focus Range",
            pick(
                18,
                &[
                    (0, "Manual"),
                    (1, "Auto"),
                    (2, "Not known"),
                    (3, "Macro"),
                    (4, "Very close"),
                    (5, "Close"),
                    (6, "Middle range"),
                    (7, "Far range"),
                    (8, "Pan focus"),
                    (9, "Super macro"),
                    (10, "Infinity"),
                ],
            ),
        ),
        (
            "AF Point",
            pick(
                19,
                &[
                    (0x2005, "Manual AF point selection"),
                    (0x3000, "None (MF)"),
                    (0x3001, "Auto AF point selection"),
                    (0x3002, "Right"),
                    (0x3003, "Center"),
                    (0x3004, "Left"),
                    (0x4001, "Auto AF point selection"),
                    (0x4006, "Face detect"),
                ],
            ),
        ),
        (
            "Exposure Mode",
            pick(
                20,
                &[
                    (0, "Easy"),
                    (1, "Program AE"),
                    (2, "Shutter speed priority AE"),
                    (3, "Aperture-priority AE"),
                    (4, "Manual"),
                    (5, "Depth-of-field AE"),
                    (6, "M-Dep"),
                    (7, "Bulb"),
                ],
            ),
        ),
    ];
    // The lens's focal lengths, in focal units per mm.
    if let (Some(long), Some(short), Some(units)) = (at(23), at(24), at(25).filter(|u| *u > 0)) {
        let mm = |v: i32| format!("{:.1}", v as f64 / units as f64).trim_end_matches(".0").to_owned();
        let lens = if long == short { format!("{} mm", mm(long)) } else { format!("{}–{} mm", mm(short), mm(long)) };
        out.push(("Lens", Some(lens)));
    }
    out.into_iter().filter_map(|(name, value)| Some((name, value?))).collect()
}

/// Where Canon's notes were first written, from the TIFF header-like footer
/// newer cameras end them with.
fn canon_footer(note: &[u8], make: &str) -> Option<usize> {
    if !make.starts_with("Canon") || note.len() < 8 {
        return None;
    }
    let footer = &note[note.len() - 8..];
    let offset = match &footer[..4] {
        b"II*\0" => u32::from_le_bytes(footer[4..].try_into().ok()?),
        b"MM\0*" => u32::from_be_bytes(footer[4..].try_into().ok()?),
        _ => return None,
    };
    Some(offset as usize)
}

/// Entries of the maker's notes that read better spelt out.
fn maker_value(maker: &str, tag: u32, value: &Value, little: bool) -> Option<String> {
    let first = value.number(0)? as u32;
    let one = value.count() == 1;
    let pick = |names: &[(u32, &str)]| {
        Some(names.iter().find(|(n, _)| *n == first).map_or_else(|| first.to_string(), |(_, name)| (*name).to_owned()))
    };
    // Nikon's EV values: signed bytes a, b, c meaning a·b/c.
    let nikon_ev = || match value {
        Value::Bytes(b) if b.len() == 4 && b[2] != 0 => {
            Some(format!("{:+.1} EV", (b[0] as i8 as f64) * (b[1] as f64) / (b[2] as f64)))
        }
        _ => None,
    };
    match (maker.to_ascii_uppercase().as_str(), tag) {
        // Folder and file of the photo on the card: 100-2418 for IMG_2418.
        ("CANON", 0x0008) if one => Some(format!("{}-{:04}", first / 10_000, first % 10_000)),
        ("CANON", 0x0010) if one => Some(format!("0x{first:08X}")),
        ("CANON", 0x00b4) if one => pick(&[(1, "sRGB"), (2, "Adobe RGB")]),
        // Nikon's ISO: what it was set to, and what it took.
        ("NIKON", 0x0002 | 0x0013) => value.number(1).filter(|&n| n > 0.0).map(trim_number),
        ("NIKON", 0x000d | 0x000e | 0x0012 | 0x0017 | 0x0018 | 0x0019) => nikon_ev(),
        ("NIKON", 0x001e) if one => pick(&[(1, "sRGB"), (2, "Adobe RGB")]),
        ("NIKON", 0x0022) if one => pick(&[
            (0, "Off"),
            (1, "Low"),
            (3, "Normal"),
            (5, "High"),
            (7, "Extra High"),
            (8, "Extra High 1"),
            (9, "Extra High 2"),
            (10, "Extra High 3"),
            (11, "Extra High 4"),
            (0xffff, "Auto"),
        ]),
        ("NIKON", 0x0084) => lens(value),
        // The camera's clock when it was switched on: big-endian year, then
        // month, day, hours, minutes, seconds.
        ("NIKON", 0x00b6) => match value {
            Value::Bytes(b) if b.len() >= 7 => Some(format!(
                "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
                u16::from_be_bytes([b[0], b[1]]),
                b[2],
                b[3],
                b[4],
                b[5],
                b[6]
            )),
            _ => None,
        },
        // The time zone the camera was set to, and daylight saving.
        ("NIKON", 0x0024) => match value {
            Value::Bytes(b) if b.len() >= 3 => {
                let minutes = if little { i16::from_le_bytes([b[0], b[1]]) } else { i16::from_be_bytes([b[0], b[1]]) };
                let sign = if minutes < 0 { '-' } else { '+' };
                let (h, m) = (minutes.unsigned_abs() / 60, minutes.unsigned_abs() % 60);
                let dst = if b[2] == 0 { "no daylight saving" } else { "daylight saving" };
                Some(format!("UTC{sign}{h:02}:{m:02}, {dst}"))
            }
            _ => None,
        },
        ("NIKON", 0x0087) if one => pick(&[
            (0, "Did not fire"),
            (1, "Fired, manual"),
            (3, "Not ready"),
            (7, "Fired, external"),
            (8, "Fired, commander mode"),
            (9, "Fired, TTL mode"),
            (18, "LED light"),
        ]),
        ("NIKON", 0x0093) if one => {
            pick(&[(1, "Lossy (type 1)"), (2, "Uncompressed"), (3, "Lossless"), (4, "Lossy (type 2)")])
        }
        ("NIKON", 0x00b1) if one => pick(&[
            (0, "Off"),
            (1, "Minimal"),
            (2, "Low"),
            (3, "Medium Low"),
            (4, "Normal"),
            (5, "Medium High"),
            (6, "High"),
        ]),
        ("OLYMPUS" | "OM", 0x0200) => pick(&[(0, "Normal"), (2, "Fast"), (3, "Panorama")]),
        ("OLYMPUS" | "OM", 0x0201) if one => pick(&[(1, "SQ"), (2, "HQ"), (3, "SHQ"), (4, "RAW")]),
        ("OLYMPUS" | "OM", 0x0202) if one => pick(&[(0, "Off"), (1, "On"), (2, "Super Macro")]),
        ("OLYMPUS" | "OM", 0x1004) if one => pick(&[(2, "On"), (3, "Off")]),
        ("FUJIFILM", 0x1001) if one => pick(&[
            (0, "-4 (softest)"),
            (1, "-3 (very soft)"),
            (2, "-2 (soft)"),
            (3, "0 (normal)"),
            (4, "+2 (hard)"),
            (5, "+3 (very hard)"),
            (6, "+4 (hardest)"),
            (130, "-1 (medium soft)"),
            (132, "+1 (medium hard)"),
        ]),
        ("FUJIFILM", 0x1002) if one => pick(&[
            (0, "Auto"),
            (256, "Daylight"),
            (512, "Cloudy"),
            (768, "Daylight Fluorescent"),
            (769, "Day White Fluorescent"),
            (770, "White Fluorescent"),
            (1024, "Incandescent"),
            (1280, "Flash"),
            (1536, "Underwater"),
            (3840, "Custom"),
        ]),
        ("FUJIFILM", 0x1003 | 0x1004) if one => {
            pick(&[(0, "Normal"), (128, "Medium High"), (256, "High"), (384, "Medium Low"), (512, "Low")])
        }
        ("FUJIFILM", 0x1010) if one => {
            pick(&[(0, "Auto"), (1, "On"), (2, "Off"), (3, "Red-eye reduction"), (4, "External")])
        }
        ("FUJIFILM", 0x1020 | 0x1030) if one => pick(&[(0, "Off"), (1, "On")]),
        ("FUJIFILM", 0x1021) if one => pick(&[(0, "Auto"), (1, "Manual")]),
        ("FUJIFILM", 0x1031) if one => pick(&[
            (0, "Auto"),
            (1, "Portrait"),
            (2, "Landscape"),
            (3, "Macro"),
            (4, "Sports"),
            (5, "Night scene"),
            (6, "Program AE"),
            (256, "Aperture-priority AE"),
            (512, "Shutter speed priority AE"),
            (768, "Manual"),
        ]),
        ("FUJIFILM", 0x1300..=0x1302) if one => pick(&[(0, "None"), (1, "Warning")]),
        ("APPLE", 0x000a) if one => pick(&[(2, "Live photo"), (3, "HDR image"), (4, "Original image")]),
        _ => None,
    }
}

/// Any entry of the maker's notes: text as it is, else in hex; numbers all
/// of them (up to a screenful); other values as `plain` has them.
fn maker_plain(value: &Value) -> Option<String> {
    const MOST: usize = 64;
    let list = |items: Vec<String>| {
        let total = items.len();
        let shown = items.into_iter().take(MOST).collect::<Vec<_>>().join(", ");
        if total > MOST { format!("{shown} … ({total} values)") } else { shown }
    };
    Some(match value {
        Value::Text(text) if text.chars().any(char::is_control) => hex(text.as_bytes()),
        // Text ended with zeros, as some makers pad it.
        Value::Bytes(b)
            if b.len() > 8
                && b.iter().rposition(|&c| c != 0).is_some_and(|end| {
                    b[..=end].iter().all(|c| c.is_ascii_graphic() || *c == b' ') && b[end + 1..].iter().all(|&c| c == 0)
                }) =>
        {
            String::from_utf8_lossy(b).trim_end_matches('\0').to_owned()
        }
        // Unreadable as text: in hex, short ones too.
        Value::Bytes(b) if !b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') => hex(b),
        Value::Numbers(n) => list(n.iter().map(u32::to_string).collect()),
        Value::Signed(n) => list(n.iter().map(i32::to_string).collect()),
        _ => plain(value)?,
    })
}

/// Names of the entries in the maker's notes, as ExifTool has them.
fn maker_tag_name(maker: &str, tag: u32) -> Option<&'static str> {
    Some(match (maker.to_ascii_uppercase().as_str(), tag) {
        ("CANON", 0x0001) => "Camera Settings",
        ("CANON", 0x0002) => "Focal Length Info",
        ("CANON", 0x0003) => "Flash Info",
        ("CANON", 0x0004) => "Shot Info",
        ("CANON", 0x0005) => "Panorama",
        ("CANON", 0x0008) => "File Number",
        ("CANON", 0x000c) => "Serial Number",
        ("CANON", 0x000d) => "Camera Info",
        ("CANON", 0x000e) => "File Length",
        ("CANON", 0x000f) => "Custom Functions",
        ("CANON", 0x0010) => "Model ID",
        ("CANON", 0x0012) => "AF Info",
        ("CANON", 0x0013) => "Thumbnail Image Valid Area",
        ("CANON", 0x0015) => "Serial Number Format",
        ("CANON", 0x001a) => "Super Macro",
        ("CANON", 0x001c) => "Date Stamp Mode",
        ("CANON", 0x001d) => "My Colors",
        ("CANON", 0x001e) => "Firmware Revision",
        ("CANON", 0x0023) => "Categories",
        ("CANON", 0x0024) => "Face Detect 1",
        ("CANON", 0x0026) => "AF Info 2",
        ("CANON", 0x0028) => "Image Unique ID",
        ("CANON", 0x0093) => "File Info",
        ("CANON", 0x0097) => "Dust Removal Data",
        ("CANON", 0x0099) => "Custom Functions 2",
        ("CANON", 0x00a0) => "Processing Info",
        ("CANON", 0x00aa) => "Measured Color",
        ("CANON", 0x00b4) => "Color Space",
        ("CANON", 0x00e0) => "Sensor Info",
        ("CANON", 0x4001) => "Color Data",
        ("NIKON", 0x0001) => "Maker Note Version",
        ("APPLE", 0x0001) => "Maker Note Version",
        ("APPLE", 0x0003) => "Run Time",
        ("APPLE", 0x0008) => "Acceleration Vector",
        ("APPLE", 0x000a) => "HDR Image Type",
        ("APPLE", 0x000b) => "Burst UUID",
        ("APPLE", 0x0011) => "Content Identifier",
        ("APPLE", 0x0015) => "Image Unique ID",
        ("APPLE", 0x0017) => "Live Photo Video Index",
        ("APPLE", 0x002e) => "Camera Type",
        ("NIKON", 0x0002) => "ISO",
        ("NIKON", 0x000b) => "White Balance Fine Tune",
        ("NIKON", 0x000c) => "WB RB Levels",
        ("NIKON", 0x000d) => "Program Shift",
        ("NIKON", 0x000e) => "Exposure Difference",
        ("NIKON", 0x0011) => "Preview IFD",
        ("NIKON", 0x0012) => "Flash Exposure Compensation",
        ("NIKON", 0x0013) => "ISO Setting",
        ("NIKON", 0x0016) => "Image Boundary",
        ("NIKON", 0x0018) => "Flash Exposure Bracket Value",
        ("NIKON", 0x0019) => "Exposure Bracket Value",
        ("NIKON", 0x001b) => "Crop High Speed",
        ("NIKON", 0x001e) => "Color Space",
        ("NIKON", 0x0022) => "Active D-Lighting",
        ("NIKON", 0x0083) => "Lens Type",
        ("NIKON", 0x0084) => "Lens",
        ("NIKON", 0x0087) => "Flash Mode",
        ("NIKON", 0x0088) => "AF Info",
        ("NIKON", 0x0089) => "Shooting Mode",
        ("NIKON", 0x008b) => "Lens F-Stops",
        ("NIKON", 0x0092) => "Hue Adjustment",
        ("NIKON", 0x0093) => "NEF Compression",
        ("NIKON", 0x0097) => "Color Balance",
        ("NIKON", 0x0098) => "Lens Data",
        ("NIKON", 0x00a2) => "Image Data Size",
        ("NIKON", 0x00a7) => "Shutter Count",
        ("NIKON", 0x00b1) => "High ISO Noise Reduction",
        ("NIKON", 0x0024) => "World Time",
        ("NIKON", 0x00b6) => "Power Up Time",
        ("NIKON" | "OLYMPUS" | "OM" | "EPSON", 0x0e00) => "Print IM",
        ("NIKON", 0x0e01) => "Nikon Capture Data",
        ("OLYMPUS" | "OM", 0x0200) => "Special Mode",
        ("OLYMPUS" | "OM", 0x0201) => "JPEG Quality",
        ("OLYMPUS" | "OM", 0x0202) => "Macro",
        ("OLYMPUS" | "OM", 0x0204) => "Digital Zoom",
        ("OLYMPUS" | "OM", 0x0208) => "Text Info",
        ("OLYMPUS" | "OM", 0x0f00) => "Data Dump",
        ("OLYMPUS" | "OM", 0x1004) => "Flash Mode",
        ("OLYMPUS" | "OM", 0x100f) => "Sharpness Factor",
        ("OLYMPUS" | "OM", 0x1029) => "Contrast",
        ("OLYMPUS" | "OM", 0x2010) => "Equipment",
        ("OLYMPUS" | "OM", 0x2020) => "Camera Settings",
        ("OLYMPUS" | "OM", 0x2030) => "Raw Development",
        ("OLYMPUS" | "OM", 0x2040) => "Image Processing",
        ("OLYMPUS" | "OM", 0x2050) => "Focus Info",
        ("FUJIFILM", 0x0000) => "Version",
        ("FUJIFILM", 0x1001) => "Sharpness",
        ("FUJIFILM", 0x1002) => "White Balance",
        ("FUJIFILM", 0x1003) => "Saturation",
        ("FUJIFILM", 0x1004) => "Contrast",
        ("FUJIFILM", 0x1010) => "Flash Mode",
        ("FUJIFILM", 0x1020) => "Macro",
        ("FUJIFILM", 0x1021) => "Focus Mode",
        ("FUJIFILM", 0x1030) => "Slow Sync",
        ("FUJIFILM", 0x1031) => "Picture Mode",
        ("FUJIFILM", 0x1100) => "Auto Bracketing",
        ("FUJIFILM", 0x1300) => "Blur Warning",
        ("FUJIFILM", 0x1301) => "Focus Warning",
        ("FUJIFILM", 0x1302) => "Exposure Warning",
        ("CANON", 0x0006) => "Image Type",
        ("CANON", 0x0007) => "Firmware Version",
        ("CANON", 0x0009) => "Owner Name",
        ("CANON", 0x0095) => "Lens Model",
        ("CANON", 0x0096) => "Internal Serial Number",
        ("NIKON", 0x0003) => "Color Mode",
        ("NIKON", 0x0004) => "Quality",
        ("NIKON", 0x0005) => "White Balance",
        ("NIKON", 0x0006) => "Sharpness",
        ("NIKON", 0x0007) => "Focus Mode",
        ("NIKON", 0x0008) => "Flash Setting",
        ("NIKON", 0x0009) => "Flash Type",
        ("NIKON", 0x000f) => "ISO Selection",
        ("NIKON", 0x001d) => "Serial Number",
        ("NIKON", 0x0080) => "Image Adjustment",
        ("NIKON", 0x0081) => "Tone Compensation",
        ("NIKON", 0x0082) => "Auxiliary Lens",
        ("NIKON", 0x008d) => "Color Hue",
        ("NIKON", 0x008f) => "Scene Mode",
        ("NIKON", 0x0090) => "Light Source",
        ("NIKON", 0x0095) => "Noise Reduction",
        ("NIKON", 0x00a0) => "Serial Number",
        ("NIKON", 0x00a9) => "Image Optimization",
        ("NIKON", 0x00aa) => "Saturation",
        ("NIKON", 0x00ab) => "Vari Program",
        ("FUJIFILM", 0x0010) => "Internal Serial Number",
        ("FUJIFILM", 0x1000) => "Quality",
        ("OLYMPUS" | "OM", 0x0207) => "Camera Type",
        ("OLYMPUS" | "OM", 0x0209) => "Camera ID",
        ("OLYMPUS" | "OM", 0x101a) => "Serial Number",
        ("PANASONIC", 0x0025) => "Internal Serial Number",
        ("PANASONIC", 0x0051) => "Lens Type",
        ("PANASONIC", 0x0052) => "Lens Serial Number",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(fields: &'a [(&str, String)], name: &str) -> Option<&'a str> {
        fields.iter().find(|(n, _)| *n == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn spells_out_canon_settings() {
        // Fine, Evaluative, a 5.9–13.2 mm lens; ISO not known (0x7fff).
        let mut settings = vec![0u32; 26];
        (settings[3], settings[16], settings[17]) = (3, 0x7fff, 3);
        (settings[23], settings[24], settings[25]) = (422, 189, 32);
        let spelt = canon_camera_settings(&settings);
        assert_eq!(get(&spelt, "Quality"), Some("Fine"));
        assert_eq!(get(&spelt, "Metering Mode"), Some("Evaluative"));
        assert_eq!(get(&spelt, "Lens"), Some("5.9–13.2 mm"));
        assert_eq!(get(&spelt, "ISO Setting"), None);
    }

    #[test]
    fn spells_out_canon_shot_info() {
        // An EOS 5D Mark IV at ISO 100, f/8, 1/128 s, 25 °C.
        let mut shot = vec![0u32; 34];
        (shot[0], shot[2], shot[4], shot[5], shot[7], shot[12]) = (68, 160, 192, 224, 5, 153);
        (shot[21], shot[22]) = (192, 224);
        let spelt = canon_shot_info(&shot, "Canon EOS 5D Mark IV");
        assert_eq!(get(&spelt, "ISO"), Some("100"));
        assert_eq!(get(&spelt, "F-Number"), Some("f/8.0"));
        assert_eq!(get(&spelt, "Exposure Time"), Some("1/128 s"));
        assert_eq!(get(&spelt, "White Balance"), Some("Flash"));
        assert_eq!(get(&spelt, "Camera Temperature"), Some("25 °C"));
        // The 350D counts the time in other units: 1/1024 s there is 1/33 s.
        let mut shot = vec![0u32; 34];
        (shot[0], shot[22]) = (68, 320);
        assert_eq!(get(&canon_shot_info(&shot, "Canon EOS 350D DIGITAL"), "Exposure Time"), Some("1/33 s"));
        // Thirds of a stop.
        assert!((canon_ev(0x0c) - 1.0 / 3.0).abs() < 1e-9 && (canon_ev(-0x14) + 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn broken_canon_lists_are_not_spelt_out() {
        let mut tags = BTreeMap::new();
        tags.insert(0x0001, Value::Numbers(vec![8192, 0, 48384, 0]));
        let (spelt, sound) = spell("Canon", "Canon PowerShot A400", &tags);
        assert!(spelt.is_empty() && !sound);
    }

    #[test]
    fn reads_nikon_clock_and_time_zone() {
        let power_up = Value::Bytes(vec![0x07, 0xdd, 10, 17, 11, 7, 4, 0]);
        assert_eq!(maker_value("NIKON", 0x00b6, &power_up, false).as_deref(), Some("2013:10:17 11:07:04"));
        let world = Value::Bytes(vec![0x00, 0x78, 0, 2]);
        assert_eq!(maker_value("NIKON", 0x0024, &world, false).as_deref(), Some("UTC+02:00, no daylight saving"));
    }
}
