//! Reading and writing `.picasa.ini` / `Picasa.ini` files.
//!
//! The file is a plain INI with CRLF line endings. Picasa marks UTF-8 content
//! with an `[encoding]` section containing `utf8=1`; without it the text is in
//! the Windows ANSI code page of the machine that wrote it. Pico tells the two
//! likely ones apart: Windows-1251 (Cyrillic) or Windows-1252 (Western).
//! Saving always writes UTF-8 with the marker, which Picasa 3 reads too.
//!
//! Windows does not care about file name case, so `picasa.ini` and
//! `.Picasa.ini` copied from there are found as well.
//!
//! Section and key order is preserved, and unknown keys are kept untouched so
//! that saving never loses data written by Picasa or other tools.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

/// File name Picasa 3 writes into every folder it touches.
pub const FILE_NAME: &str = ".picasa.ini";
/// File name used by older Picasa versions.
pub const LEGACY_FILE_NAME: &str = "Picasa.ini";

const ENCODING_SECTION: &str = "encoding";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub entries: Vec<(String, String)>,
}

impl Section {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), entries: Vec::new() }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Sets a key, keeping its position if it already exists.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(entry) => entry.1 = value,
            None => self.entries.push((key.to_owned(), value)),
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        let index = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(index).1)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PicasaIni {
    pub sections: Vec<Section>,
}

impl PicasaIni {
    /// Parses file content. Never fails: malformed lines are skipped, as Picasa does.
    pub fn parse(text: &str) -> Self {
        let mut ini = Self::default();
        for raw in text.lines() {
            let line = raw.trim_end_matches('\r').trim_start_matches('\u{feff}');
            if line.trim().is_empty() || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                ini.sections.push(Section::new(name));
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            if ini.sections.is_empty() {
                // Keys before any section header have nowhere to go in Picasa either.
                continue;
            }
            let section = ini.sections.last_mut().expect("checked above");
            section.entries.push((key.to_owned(), value.to_owned()));
        }
        ini
    }

    /// Decodes raw file bytes: UTF-8 when valid, otherwise the guessed ANSI code page.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(text) => Self::parse(text),
            Err(_) if looks_cyrillic(bytes) => Self::parse(&decode_windows_1251(bytes)),
            Err(_) => Self::parse(&decode_windows_1252(bytes)),
        }
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self::from_bytes(&std::fs::read(path)?))
    }

    /// Loads the INI of a folder, preferring `.picasa.ini` over the legacy
    /// `Picasa.ini`, and exact names over other letter cases.
    /// Returns `None` when the folder has neither.
    pub fn load_for_folder(folder: &Path) -> io::Result<Option<(PathBuf, Self)>> {
        let names: Vec<String> = match std::fs::read_dir(folder) {
            Ok(entries) => entries.flatten().filter_map(|e| e.file_name().into_string().ok()).collect(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let found = [FILE_NAME, LEGACY_FILE_NAME].iter().find_map(|wanted| {
            names.iter().find(|n| n == wanted).or_else(|| names.iter().find(|n| n.eq_ignore_ascii_case(wanted)))
        });
        let Some(name) = found else { return Ok(None) };
        let path = folder.join(name);
        Ok(Some((path.clone(), Self::load(&path)?)))
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn section_mut(&mut self, name: &str) -> &mut Section {
        if let Some(index) = self.sections.iter().position(|s| s.name == name) {
            return &mut self.sections[index];
        }
        self.sections.push(Section::new(name));
        self.sections.last_mut().expect("just pushed")
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)?.get(key)
    }

    /// Serializes with CRLF line endings, always as UTF-8. The `[encoding]`
    /// marker is added when needed and dropped when the text is plain ASCII.
    pub fn to_text(&self) -> String {
        let needs_utf8 = self.sections.iter().any(|s| {
            s.name != ENCODING_SECTION
                && (!s.name.is_ascii() || s.entries.iter().any(|(k, v)| !k.is_ascii() || !v.is_ascii()))
        });
        let mut out = String::new();
        for section in self.sections.iter().filter(|s| s.name != ENCODING_SECTION) {
            write_section(&mut out, section);
        }
        if needs_utf8 {
            let mut marker = Section::new(ENCODING_SECTION);
            marker.set("utf8", "1");
            write_section(&mut out, &marker);
        }
        out
    }

    /// Writes the file atomically (temporary file + rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("ini.pico-tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

fn write_section(out: &mut String, section: &Section) {
    let _ = write!(out, "[{}]\r\n", section.name);
    for (key, value) in &section.entries {
        let _ = write!(out, "{key}={value}\r\n");
    }
}

/// Cyrillic words in Windows-1251 are runs of bytes in 0xC0..=0xFF; Western
/// text in Windows-1252 has accented letters mostly one or two at a time
/// ("Größe"), so three in a row is a reliable sign of Cyrillic.
fn looks_cyrillic(bytes: &[u8]) -> bool {
    bytes.windows(3).any(|w| w.iter().all(|&b| b >= 0xc0))
}

fn decode_windows_1251(bytes: &[u8]) -> String {
    const HIGH: [char; 64] = [
        '\u{0402}', '\u{0403}', '\u{201a}', '\u{0453}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{20ac}',
        '\u{2030}', '\u{0409}', '\u{2039}', '\u{040a}', '\u{040c}', '\u{040b}', '\u{040f}', '\u{0452}', '\u{2018}',
        '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}', '\u{fffd}', '\u{2122}', '\u{0459}',
        '\u{203a}', '\u{045a}', '\u{045c}', '\u{045b}', '\u{045f}', '\u{00a0}', '\u{040e}', '\u{045e}', '\u{0408}',
        '\u{00a4}', '\u{0490}', '\u{00a6}', '\u{00a7}', '\u{0401}', '\u{00a9}', '\u{0404}', '\u{00ab}', '\u{00ac}',
        '\u{00ad}', '\u{00ae}', '\u{0407}', '\u{00b0}', '\u{00b1}', '\u{0406}', '\u{0456}', '\u{0491}', '\u{00b5}',
        '\u{00b6}', '\u{00b7}', '\u{0451}', '\u{2116}', '\u{0454}', '\u{00bb}', '\u{0458}', '\u{0405}', '\u{0455}',
        '\u{0457}',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0xbf => HIGH[(b - 0x80) as usize],
            // А..я are contiguous: 0xC0 is U+0410.
            0xc0..=0xff => char::from_u32(0x0410 + (b - 0xc0) as u32).expect("Cyrillic range"),
            _ => b as char,
        })
        .collect()
}

/// Windows-1252 differs from Latin-1 only in 0x80..=0x9F.
fn decode_windows_1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9f => HIGH[(b - 0x80) as usize],
            _ => b as char,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[Picasa]\r\nname=Summit\r\nP2category=Exported Pictures\r\ndate=43880.743600\r\n\
[IMG_1.jpg]\r\nstar=yes\r\nfilters=enhance=1;\r\n";

    #[test]
    fn round_trips_ascii_without_encoding_marker() {
        let ini = PicasaIni::parse(SAMPLE);
        assert_eq!(ini.get("Picasa", "date"), Some("43880.743600"));
        assert_eq!(ini.get("IMG_1.jpg", "star"), Some("yes"));
        assert_eq!(ini.to_text(), SAMPLE);
    }

    #[test]
    fn adds_encoding_marker_for_non_ascii() {
        let mut ini = PicasaIni::parse(SAMPLE);
        ini.section_mut("IMG_1.jpg").set("caption", "Привет");
        let text = ini.to_text();
        assert!(text.ends_with("[encoding]\r\nutf8=1\r\n"));
        assert_eq!(PicasaIni::parse(&text).get("IMG_1.jpg", "caption"), Some("Привет"));
    }

    #[test]
    fn reads_ansi_files() {
        let ini = PicasaIni::from_bytes(b"[a.jpg]\r\ncaption=caf\xe9 \x80\r\n");
        assert_eq!(ini.get("a.jpg", "caption"), Some("café €"));
    }

    #[test]
    fn tells_cyrillic_and_western_ansi_apart() {
        // "Старая машинка" in Windows-1251.
        let cp1251 = b"[a.jpg]\r\ncaption=\xd1\xf2\xe0\xf0\xe0\xff \xec\xe0\xf8\xe8\xed\xea\xe0\r\n";
        assert_eq!(PicasaIni::from_bytes(cp1251).get("a.jpg", "caption"), Some("Старая машинка"));
        // German with two accented letters in a row stays Western.
        let cp1252 = b"[a.jpg]\r\ncaption=Gr\xf6\xdfe\r\n";
        assert_eq!(PicasaIni::from_bytes(cp1252).get("a.jpg", "caption"), Some("Größe"));
    }

    #[test]
    fn finds_ini_regardless_of_case_and_prefers_dot_file() {
        let dir = std::env::temp_dir().join(format!("pico-test-ini-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(PicasaIni::load_for_folder(&dir).unwrap().is_none());

        std::fs::write(dir.join("picasa.ini"), "[Picasa]\nname=legacy\n").unwrap();
        let (path, ini) = PicasaIni::load_for_folder(&dir).unwrap().unwrap();
        assert_eq!(path.file_name().unwrap(), "picasa.ini", "saves go back to the file that was read");
        assert_eq!(ini.get("Picasa", "name"), Some("legacy"));

        std::fs::write(dir.join(".Picasa.INI"), "[Picasa]\nname=dot\n").unwrap();
        let (_, ini) = PicasaIni::load_for_folder(&dir).unwrap().unwrap();
        assert_eq!(ini.get("Picasa", "name"), Some("dot"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_values_containing_equals_signs() {
        let ini = PicasaIni::parse("[a.jpg]\nfilters=crop64=1,cbc3ffff;\n");
        assert_eq!(ini.get("a.jpg", "filters"), Some("crop64=1,cbc3ffff;"));
    }
}
