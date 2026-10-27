//! Scanning watched folders into a list of folders with media.

use crate::ini::PicasaIni;
use crate::media::MediaKind;
use crate::meta::PhotoMeta;
use std::io;
use std::path::{Path, PathBuf};

/// Directory names never scanned. Mirrors Picasa's `runtime/filters.txt`,
/// plus Pico's and Picasa's own backup folders.
const SKIPPED_DIRS: &[&str] = &["windows", "winnt", "temp", "program files", "originals", ".picasaoriginals"];

#[derive(Debug, Clone)]
pub struct MediaFile {
    pub path: PathBuf,
    pub kind: MediaKind,
    /// Parsed from the folder's INI section named after the file.
    pub meta: PhotoMeta,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
    /// EXIF "date taken" (`YYYY:MM:DD HH:MM:SS`), for JPEG photos that have it.
    pub taken: Option<String>,
    /// Where the photo was taken, from EXIF GPS: (latitude, longitude).
    pub gps: Option<(f64, f64)>,
}

impl MediaFile {
    /// Reads the file's size, modification time and, for JPEGs, the EXIF date.
    pub fn new(path: PathBuf, kind: MediaKind) -> Self {
        let stat = std::fs::metadata(&path).ok();
        let is_jpeg = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "jpe" | "jfif"));
        let exif = if is_jpeg { crate::exif::read(&path) } else { None };
        let (taken, gps) = exif.map_or((None, None), |e| (e.taken, e.gps));
        Self {
            path,
            kind,
            meta: PhotoMeta::default(),
            size: stat.as_ref().map_or(0, |s| s.len()),
            modified: stat.and_then(|s| s.modified().ok()),
            taken,
            gps,
        }
    }

    /// The date to sort by: when the photo was taken, else when the file changed,
    /// as text that sorts chronologically.
    pub fn sort_date(&self) -> String {
        if let Some(taken) = &self.taken {
            return taken.clone();
        }
        let secs = self.modified.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        crate::fileops::exif_style_date(secs)
    }

    pub fn name(&self) -> String {
        Folder::file_name(&self.path)
    }
}

#[derive(Debug, Clone)]
pub struct Folder {
    pub path: PathBuf,
    /// Display name: the `[Picasa] name=` override, or the directory name.
    pub name: String,
    pub files: Vec<MediaFile>,
    pub ini: PicasaIni,
    /// The INI file that was read, or where a new `.picasa.ini` will be created.
    pub ini_path: PathBuf,
}

impl Folder {
    /// Writes one file's metadata into the folder INI and saves it to disk.
    /// Keys Pico does not know about are preserved.
    pub fn save_meta(&mut self, index: usize) -> io::Result<()> {
        self.save_metas(&[index])
    }

    /// Like [`Folder::save_meta`] for several files, writing the INI once.
    pub fn save_metas(&mut self, indices: &[usize]) -> io::Result<()> {
        for &index in indices {
            let file = &self.files[index];
            let section = self.ini.section_mut(&file.name());
            file.meta.write_to(section);
            if section.entries.is_empty() {
                let name = section.name.clone();
                self.ini.sections.retain(|s| s.name != name);
            }
        }
        self.ini.save(&self.ini_path)
    }

    pub fn file_name(path: &Path) -> String {
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
    }
}

/// Recursively collects every folder under `root` that directly contains media.
/// Unreadable directories are skipped silently; a photo library must not fail
/// to open because of one locked folder.
pub fn scan(root: &Path) -> Vec<Folder> {
    scan_excluding(root, &[])
}

/// Like [`scan`], but leaves out the `excluded` folders and everything in them.
pub fn scan_excluding(root: &Path, excluded: &[PathBuf]) -> Vec<Folder> {
    let mut folders = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if excluded.contains(&dir) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                if !is_skipped_dir(&path) {
                    pending.push(path);
                }
            } else if let Some(kind) = MediaKind::of(&path) {
                files.push(MediaFile::new(path, kind));
            }
        }
        if files.is_empty() {
            continue;
        }
        files.sort_by(|a, b| natural_cmp(&a.name(), &b.name()));
        let (ini_path, ini) = PicasaIni::load_for_folder(&dir)
            .ok()
            .flatten()
            .unwrap_or_else(|| (dir.join(crate::ini::FILE_NAME), PicasaIni::default()));
        for file in &mut files {
            if let Some(section) = ini.section(&file.name()) {
                file.meta = PhotoMeta::from_section(section);
            }
        }
        let name = ini
            .get("Picasa", "name")
            .filter(|n| !n.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| Folder::file_name(&dir));
        folders.push(Folder { path: dir, name, files, ini, ini_path });
    }
    sort_folders(&mut folders);
    folders
}

/// Orders folders by path, comparing names naturally at each level.
pub fn sort_folders(folders: &mut [Folder]) {
    folders.sort_by(|a, b| {
        let parts =
            |f: &Folder| f.path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>();
        let (a, b) = (parts(a), parts(b));
        a.iter().zip(&b).map(|(x, y)| natural_cmp(x, y)).find(|o| o.is_ne()).unwrap_or(a.len().cmp(&b.len()))
    });
}

/// Compares file names the way people expect: case-insensitive, numbers by
/// value (`img2` before `img10`), and the name before the extension
/// (`head.png` before `head-rotated.png`).
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| match s.rfind('.') {
        Some(dot) if dot > 0 => (s[..dot].to_owned(), s[dot..].to_owned()),
        _ => (s.to_owned(), String::new()),
    };
    let ((a_stem, a_ext), (b_stem, b_ext)) = (split(a), split(b));
    compare_chunks(&a_stem, &b_stem).then_with(|| compare_chunks(&a_ext, &b_ext)).then_with(|| a.cmp(b))
}

fn compare_chunks(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut digits = String::new();
                    while let Some(d) = it.next_if(char::is_ascii_digit) {
                        digits.push(d);
                    }
                    digits
                };
                let (da, db) = (take(&mut a), take(&mut b));
                let (ta, tb) = (da.trim_start_matches('0'), db.trim_start_matches('0'));
                let order = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if order.is_ne() {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order.is_ne() {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

fn is_skipped_dir(path: &Path) -> bool {
    let name = Folder::file_name(path);
    (name.starts_with('.') && name != ".picasaoriginals") || SKIPPED_DIRS.contains(&name.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_names_naturally() {
        let mut names = vec!["IMG10.jpg", "img2.jpg", "head-rotate1.png", "head.png", "Img1.JPG", "a.png"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["a.png", "head.png", "head-rotate1.png", "Img1.JPG", "img2.jpg", "IMG10.jpg"]);
    }

    #[test]
    fn saves_edits_and_keeps_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("pico-test-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbImage::new(4, 4).save(dir.join("a.png")).unwrap();
        image::RgbImage::new(4, 4).save(dir.join("b.png")).unwrap();
        std::fs::write(dir.join("Picasa.ini"), "[a.png]\r\nbackuphash=123\r\n").unwrap();

        let mut folders = scan(&dir);
        let folder = &mut folders[0];
        assert_eq!(folder.files.len(), 2);
        folder.files[0].meta.star = true;
        folder.files[0].meta.caption = Some("Кот".into());
        folder.save_meta(0).unwrap();
        folder.files[1].meta.rotate = 3;
        folder.save_meta(1).unwrap();

        // The legacy file is updated in place rather than duplicated.
        assert!(!dir.join(".picasa.ini").exists());
        let text = std::fs::read_to_string(dir.join("Picasa.ini")).unwrap();
        assert!(text.contains("backuphash=123\r\n"), "{text}");
        assert!(text.contains("[encoding]\r\nutf8=1\r\n"));

        let reread = scan(&dir);
        assert!(reread[0].files[0].meta.star);
        assert_eq!(reread[0].files[0].meta.caption.as_deref(), Some("Кот"));
        assert_eq!(reread[0].files[1].meta.rotate, 3);

        // Clearing everything removes the section instead of leaving it empty.
        let mut folders = reread;
        folders[0].files[1].meta.rotate = 0;
        folders[0].save_meta(1).unwrap();
        assert!(!std::fs::read_to_string(dir.join("Picasa.ini")).unwrap().contains("[b.png]"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A Picasa album: a named, virtual collection of photos from any folders.
#[derive(Debug, Clone, PartialEq)]
pub struct Album {
    pub token: String,
    pub name: String,
    /// `(folder index, file index)` of every photo in the album.
    pub members: Vec<(usize, usize)>,
}

const ALBUM_PREFIX: &str = ".album:";

/// Collects albums from every folder's INI. An album is declared by a
/// `[.album:<token>]` section in each folder holding its photos; photos list
/// the tokens of their albums in `albums=`.
pub fn albums(folders: &[Folder]) -> Vec<Album> {
    let mut found: Vec<Album> = Vec::new();
    for folder in folders {
        for section in &folder.ini.sections {
            let Some(token) = section.name.strip_prefix(ALBUM_PREFIX) else { continue };
            if !found.iter().any(|a| a.token == token) {
                let name = section.get("name").filter(|n| !n.is_empty()).unwrap_or(token).to_owned();
                found.push(Album { token: token.to_owned(), name, members: Vec::new() });
            }
        }
    }
    for (fi, folder) in folders.iter().enumerate() {
        for (i, file) in folder.files.iter().enumerate() {
            for token in &file.meta.albums {
                if let Some(album) = found.iter_mut().find(|a| &a.token == token) {
                    album.members.push((fi, i));
                }
            }
        }
    }
    found.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    found
}

/// A new album token: 16 hex digits, unique enough for one library.
pub fn new_album_token() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut x = (nanos as u64) ^ ((std::process::id() as u64) << 32) ^ 0x9e37_79b9_7f4a_7c15;
    // splitmix64 finalizer spreads the bits.
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    format!("{:016x}", x ^ (x >> 31))
}

impl Folder {
    /// Makes sure this folder's INI declares the album, so Picasa sees it too.
    /// The INI is written by the next [`Folder::save_metas`].
    pub fn declare_album(&mut self, token: &str, name: &str) {
        let section = self.ini.section_mut(&format!("{ALBUM_PREFIX}{token}"));
        if section.get("name").is_none() {
            section.set("name", name);
            section.set("token", token);
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            section.set("date", crate::fileops::exif_style_date(secs).replacen(':', "-", 2).replacen(' ', "T", 1));
        }
    }
}

#[cfg(test)]
mod album_tests {
    use super::*;

    #[test]
    fn albums_span_folders() {
        let root = std::env::temp_dir().join(format!("pico-test-albums-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (dir, ini) in [
            ("one", "[.album:aa11]\r\nname=Best of\r\ntoken=aa11\r\n[x.png]\r\nalbums=aa11\r\n"),
            (
                "two",
                "[.album:aa11]\r\nname=Best of\r\ntoken=aa11\r\n[.album:bb22]\r\nname=Empty\r\n[y.png]\r\nalbums=aa11\r\n",
            ),
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
            image::RgbImage::new(2, 2).save(root.join(dir).join(if dir == "one" { "x.png" } else { "y.png" })).unwrap();
            std::fs::write(root.join(dir).join(".picasa.ini"), ini).unwrap();
        }
        let mut folders = scan(&root);
        let found = albums(&folders);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Best of");
        assert_eq!(found[0].members, [(0, 0), (1, 0)]);
        assert!(found[1].members.is_empty());

        let token = new_album_token();
        assert_eq!(token.len(), 16);
        folders[0].declare_album(&token, "New");
        folders[0].files[0].meta.albums.push(token.clone());
        folders[0].save_metas(&[0]).unwrap();
        let reread = albums(&scan(&root));
        let new = reread.iter().find(|a| a.token == token).expect("new album");
        assert_eq!((new.name.as_str(), new.members.len()), ("New", 1));
        std::fs::remove_dir_all(&root).unwrap();
    }
}

/// Groups files with identical content. Only files of equal size are read,
/// so this is quick on a typical library.
pub fn find_duplicates(files: &[(PathBuf, u64)]) -> Vec<Vec<PathBuf>> {
    use std::collections::HashMap;
    let mut by_size: HashMap<u64, Vec<&PathBuf>> = HashMap::new();
    for (path, size) in files {
        if *size > 0 {
            by_size.entry(*size).or_default().push(path);
        }
    }
    let mut groups = Vec::new();
    for same_size in by_size.into_values().filter(|v| v.len() > 1) {
        let mut by_content: HashMap<Vec<u8>, Vec<PathBuf>> = HashMap::new();
        for path in same_size {
            // Equal size and equal bytes: compare full content via a digest.
            if let Ok(bytes) = std::fs::read(path) {
                by_content.entry(content_digest(&bytes)).or_default().push(path.clone());
            }
        }
        groups.extend(by_content.into_values().filter(|g| g.len() > 1));
    }
    for group in &mut groups {
        group.sort();
    }
    groups.sort();
    groups
}

/// Two independent 64-bit FNV-1a variants; a collision on both is negligible
/// for files that already have the same size.
fn content_digest(bytes: &[u8]) -> Vec<u8> {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x8422_2325_cbf2_9ce4;
    for &byte in bytes {
        a = (a ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
        b = (b ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
    }
    [a.to_le_bytes(), b.to_le_bytes()].concat()
}

#[cfg(test)]
mod duplicate_tests {
    use super::*;

    #[test]
    fn groups_identical_files_only() {
        let dir = std::env::temp_dir().join(format!("pico-test-dupes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let write = |name: &str, data: &[u8]| {
            std::fs::write(dir.join(name), data).unwrap();
            (dir.join(name), data.len() as u64)
        };
        let files = [
            write("a.jpg", b"same bytes"),
            write("sub/a copy.jpg", b"same bytes"),
            write("b.jpg", b"diff bytes"), // same size, other content
            write("c.jpg", b"unique"),
        ];
        let groups = find_duplicates(&files);
        assert_eq!(groups, [vec![dir.join("a.jpg"), dir.join("sub/a copy.jpg")]]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A place for the Places view: photos taken within the same ~10 km cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    /// Cell key, e.g. "59.4,24.7"; stable for the same location.
    pub key: String,
    /// Average position of the photos in the cell.
    pub center: (f64, f64),
    pub members: Vec<(usize, usize)>,
}

/// A photo reference with its position.
type Located = ((usize, usize), (f64, f64));

/// Groups photos with GPS into places, largest first.
pub fn places(folders: &[Folder]) -> Vec<Place> {
    let mut by_cell: std::collections::BTreeMap<String, Vec<Located>> = Default::default();
    for (fi, folder) in folders.iter().enumerate() {
        for (i, file) in folder.files.iter().enumerate() {
            if let Some((lat, lon)) = file.gps {
                by_cell.entry(place_key(lat, lon)).or_default().push(((fi, i), (lat, lon)));
            }
        }
    }
    let mut places: Vec<Place> = by_cell
        .into_iter()
        .map(|(key, items)| {
            let n = items.len() as f64;
            let center =
                (items.iter().map(|(_, p)| p.0).sum::<f64>() / n, items.iter().map(|(_, p)| p.1).sum::<f64>() / n);
            Place { key, center, members: items.into_iter().map(|(at, _)| at).collect() }
        })
        .collect();
    places.sort_by(|a, b| b.members.len().cmp(&a.members.len()).then_with(|| a.key.cmp(&b.key)));
    places
}

/// One decimal degree is about 11 km of latitude.
pub fn place_key(lat: f64, lon: f64) -> String {
    format!("{:.1},{:.1}", (lat * 10.0).round() / 10.0, (lon * 10.0).round() / 10.0)
}

/// "59.4° N, 24.7° E" for a place's center.
pub fn place_label(center: (f64, f64)) -> String {
    let (lat, lon) = center;
    format!(
        "{:.1}° {}, {:.1}° {}",
        lat.abs(),
        if lat >= 0.0 { "N" } else { "S" },
        lon.abs(),
        if lon >= 0.0 { "E" } else { "W" }
    )
}

/// `YYYY:MM` month keys with photo counts, newest first, for the Timeline.
pub fn months(folders: &[Folder]) -> Vec<(String, usize)> {
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for file in folders.iter().flat_map(|f| &f.files) {
        *counts.entry(file.sort_date()[..7].to_owned()).or_default() += 1;
    }
    counts.into_iter().rev().collect()
}

/// "May 2018" for a `YYYY:MM` key.
pub fn month_label(key: &str) -> String {
    const NAMES: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let month: usize = key.get(5..7).and_then(|m| m.parse().ok()).unwrap_or(1);
    format!("{} {}", NAMES[month.clamp(1, 12) - 1], &key[..4])
}

/// "14 March 2021, 09:05" for a `YYYY:MM:DD HH:MM:SS` date.
pub fn date_label(date: &str) -> String {
    let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok());
    let time = date.get(11..16).filter(|t| *t != "00:00");
    match (day, time) {
        (Some(day), Some(time)) => format!("{day} {}, {time}", month_label(date)),
        (Some(day), None) => format!("{day} {}", month_label(date)),
        _ => date.to_owned(),
    }
}

#[cfg(test)]
mod place_tests {
    use super::*;

    #[test]
    fn keys_and_labels() {
        assert_eq!(date_label("2021:03:14 09:05:59"), "14 March 2021, 09:05");
        assert_eq!(date_label("2021:12:01 00:00:00"), "1 December 2021", "no time when the camera gave none");
        assert_eq!(date_label("garbage"), "garbage");
        assert_eq!(place_key(59.393503, 24.666169), "59.4,24.7");
        assert_eq!(place_key(59.43, 24.74), "59.4,24.7", "same ~10 km cell");
        assert_eq!(place_label((-33.86, 151.21)), "33.9° S, 151.2° E");
        assert_eq!(month_label("2018:05"), "May 2018");
    }

    #[test]
    fn scanned_jpegs_land_in_places_and_months() {
        let dir = std::env::temp_dir().join(format!("pico-test-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut jpeg = Vec::new();
        image::RgbImage::new(8, 8).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        std::fs::write(dir.join("tallinn.jpg"), crate::exif::insert(&jpeg, &crate::exif::read_tests::sample_app1()))
            .unwrap();
        image::RgbImage::new(8, 8).save(dir.join("nowhere.png")).unwrap();
        let folders = scan(&dir);
        let found = places(&folders);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].key, "59.4,24.8");
        assert_eq!(found[0].members.len(), 1);
        assert!(months(&folders).iter().any(|(key, n)| key == "2009:07" && *n == 1));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
