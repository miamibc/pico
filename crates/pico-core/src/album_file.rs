//! An album's own file, `<config dir>/<album token>.pal`, in the format of
//! Picasa's album files (`Picasa2Albums/<database id>/*.pal`): the album's
//! name, description, date and token, and its photos in the album's order.
//!
//! ```xml
//! <picasa2album>
//!  <AlbumID>aa11</AlbumID>
//!  <property name="uid" type="string" value="aa11"/>
//!  <property name="name" type="string" value="Trip"/>
//!  <property name="token" type="string" value="]album:aa11"/>
//!  <files>
//!   <filename id="2049:1234">/home/me/Pictures/a.jpg</filename>
//!  </files>
//! </picasa2album>
//! ```
//!
//! As in Picasa, edits stay in the folders' `.picasa.ini`; the album only
//! lists its photos. Pico adds `id`, the file's identity on disk, which
//! survives renames and moves within a disk, so the order follows the photos.

use crate::config;
use crate::ini::Section;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

/// A photo of the album: who it is on disk and where it was last seen.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// From `file_id`; `None` where the system has no such thing.
    pub id: Option<String>,
    pub path: PathBuf,
}

impl Entry {
    pub fn of(path: &Path) -> Entry {
        Entry { id: file_id(path), path: path.to_path_buf() }
    }

    /// The same photo: by identity on disk where both have one, else by path.
    pub fn matches(&self, other: &Entry) -> bool {
        match (&self.id, &other.id) {
            (Some(a), Some(b)) => a == b,
            _ => self.path == other.path,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Album {
    pub token: String,
    pub name: String,
    pub description: Option<String>,
    /// As Picasa writes it: days since 1899-12-30.
    pub date: Option<String>,
    /// In the album's order.
    pub files: Vec<Entry>,
}

/// The identity of a file on disk: device and inode.
#[cfg(unix)]
pub fn file_id(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some(format!("{}:{}", meta.dev(), meta.ino()))
}

/// Windows keeps file IDs out of stable Rust; there the path stands in.
#[cfg(not(unix))]
pub fn file_id(_path: &Path) -> Option<String> {
    None
}

/// Tokens are hex in Picasa's files; anything else never names a file here.
fn album_path(token: &str) -> Option<PathBuf> {
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    config::config_dir().map(|dir| dir.join(format!("{token}.pal")))
}

pub fn load(token: &str) -> Album {
    let text = album_path(token).and_then(|path| std::fs::read_to_string(path).ok());
    text.map_or_else(|| Album { token: token.to_owned(), ..Album::default() }, |text| parse(&text))
}

pub fn save(album: &Album) -> io::Result<()> {
    let path =
        album_path(&album.token).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let tmp = path.with_extension("pal.pico-tmp");
    std::fs::write(&tmp, to_text(album))?;
    std::fs::rename(&tmp, path)
}

/// Every album with a file of its own, as read from Pico's config.
pub fn list() -> Vec<Album> {
    let Some(dir) = config::config_dir() else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".pal").map(str::to_owned))
        .filter(|token| album_path(token).is_some())
        .map(|token| load(&token))
        .collect()
}

/// Deletes the album's file, if it has one.
pub fn remove(token: &str) -> io::Result<()> {
    let Some(path) = album_path(token) else { return Ok(()) };
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The album brought up to date with its folders: its declaration there
/// (`[.album:<token>]`), and `members` (in the order new ones should follow)
/// in the album's order. A photo of `previous` that matches no member stays
/// only when `keep` says so: its folder is not in Pico.
pub fn update(previous: &Album, declaration: &Section, members: &[Entry], keep: impl Fn(&Entry) -> bool) -> Album {
    let mut placed = vec![false; members.len()];
    let mut files = Vec::new();
    for entry in &previous.files {
        match members.iter().position(|m| m.matches(entry)) {
            Some(i) if !placed[i] => {
                placed[i] = true;
                files.push(members[i].clone());
            }
            Some(_) => {}
            None if keep(entry) => files.push(entry.clone()),
            None => {}
        }
    }
    files.extend(members.iter().zip(&placed).filter(|(_, placed)| !**placed).map(|(m, _)| m.clone()));
    let text = |key: &str| declaration.get(key).filter(|v| !v.is_empty()).map(str::to_owned);
    Album {
        token: previous.token.clone(),
        name: text("name").unwrap_or_else(|| previous.name.clone()),
        description: text("description").or_else(|| previous.description.clone()),
        date: text("date").and_then(|d| album_date(&d)).or_else(|| previous.date.clone()),
        files,
    }
}

/// The album with the photos of `order` first, in that order, and the rest
/// after them as they were.
pub fn arrange(album: &Album, order: &[Entry]) -> Album {
    let mut used = vec![false; album.files.len()];
    let mut files = Vec::new();
    for wanted in order {
        if let Some(i) = album.files.iter().enumerate().position(|(i, e)| !used[i] && e.matches(wanted)) {
            used[i] = true;
            files.push(album.files[i].clone());
        }
    }
    files.extend(album.files.iter().zip(&used).filter(|(_, used)| !**used).map(|(e, _)| e.clone()));
    Album { files, ..album.clone() }
}

/// A date in Picasa's form, from Picasa's form or older Pico's.
fn album_date(text: &str) -> Option<String> {
    crate::library::parse_album_date(text).map(crate::library::format_album_date)
}

/// The album as its `.pal` file holds it.
pub fn to_text(album: &Album) -> String {
    let mut out = String::from("<picasa2album>\r\n");
    let token = escape(&album.token);
    let _ = write!(out, "\t<AlbumID>{token}</AlbumID>\r\n");
    let mut property = |name: &str, kind: &str, value: &str| {
        let _ = write!(out, "\t<property name=\"{name}\" type=\"{kind}\" value=\"{}\"/>\r\n", escape(value));
    };
    property("uid", "string", &album.token);
    if let Some(date) = &album.date {
        property("date", "real64", date);
    }
    if let Some(description) = &album.description {
        property("description", "string", description);
    }
    property("name", "string", &album.name);
    property("token", "string", &format!("]album:{}", album.token));
    out.push_str("\t<files>\r\n");
    for entry in &album.files {
        let id = entry.id.as_ref().map(|id| format!(" id=\"{}\"", escape(id))).unwrap_or_default();
        let _ = write!(out, "\t\t<filename{id}>{}</filename>\r\n", escape(&entry.path.display().to_string()));
    }
    out.push_str("\t</files>\r\n</picasa2album>\r\n");
    out
}

fn parse(text: &str) -> Album {
    let mut album = Album::default();
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("<property ") {
            let (Some(name), Some(value)) = (attribute(rest, "name"), attribute(rest, "value")) else { continue };
            match name.as_str() {
                "uid" => album.token = value,
                "name" => album.name = value,
                "description" => album.description = Some(value),
                "date" => album.date = album_date(&value),
                _ => {}
            }
        } else if let Some(rest) = line.strip_prefix("<filename") {
            let Some((attributes, path)) = rest.split_once('>') else { continue };
            let Some(path) = path.strip_suffix("</filename>") else { continue };
            album.files.push(Entry { id: attribute(attributes, "id"), path: PathBuf::from(unescape(path)) });
        }
    }
    album
}

/// The value of `name="…"` among XML attributes.
fn attribute(attributes: &str, name: &str) -> Option<String> {
    let start = attributes
        .find(&format!(" {name}=\""))
        .map(|i| i + name.len() + 3)
        .or_else(|| attributes.starts_with(&format!("{name}=\"")).then_some(name.len() + 2))?;
    let end = attributes[start..].find('"')? + start;
    Some(unescape(&attributes[start..end]))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, id: &str) -> Entry {
        Entry { id: Some(id.into()), path: PathBuf::from(path) }
    }

    fn declaration() -> Section {
        let mut section = Section::new(".album:aa11");
        section.set("name", "Trip & Co");
        section.set("token", "aa11");
        section.set("date", "43880.743600");
        section
    }

    #[test]
    fn writes_picasa_album_files() {
        let previous = Album { token: "aa11".into(), ..Album::default() };
        let album = update(&previous, &declaration(), &[entry("/p/a.jpg", "1:1")], |_| true);
        assert_eq!(
            to_text(&album),
            "<picasa2album>\r\n\t<AlbumID>aa11</AlbumID>\r\n\
             \t<property name=\"uid\" type=\"string\" value=\"aa11\"/>\r\n\
             \t<property name=\"date\" type=\"real64\" value=\"43880.743600\"/>\r\n\
             \t<property name=\"name\" type=\"string\" value=\"Trip &amp; Co\"/>\r\n\
             \t<property name=\"token\" type=\"string\" value=\"]album:aa11\"/>\r\n\
             \t<files>\r\n\t\t<filename id=\"1:1\">/p/a.jpg</filename>\r\n\t</files>\r\n</picasa2album>\r\n"
        );
        assert_eq!(parse(&to_text(&album)), album);
    }

    #[test]
    fn reads_files_written_by_picasa() {
        let text = "<picasa2album>\n<DBID>7f</DBID>\n<AlbumID>aa11</AlbumID>\n\
                    <property name=\"uid\" type=\"string\" value=\"aa11\"/>\n\
                    <property name=\"name\" type=\"string\" value=\"Trip\"/>\n\
                    <property name=\"date\" type=\"real64\" value=\"2020-02-19T17:50:47\"/>\n\
                    <files>\n<filename>C:\\Photos\\a.jpg</filename>\n</files>\n</picasa2album>\n";
        let album = parse(text);
        assert_eq!((album.token.as_str(), album.name.as_str()), ("aa11", "Trip"));
        // The date older Pico wrote turns into Picasa's.
        assert_eq!(album.date.as_deref(), Some("43880.743600"));
        assert_eq!(album.files, [Entry { id: None, path: PathBuf::from("C:\\Photos\\a.jpg") }]);
    }

    #[test]
    fn keeps_the_order_as_photos_come_and_go() {
        let previous = Album { token: "aa11".into(), ..Album::default() };
        let album =
            update(&previous, &declaration(), &[entry("/p/a.jpg", "1:1"), entry("/away/x.jpg", "2:7")], |_| true);
        // b joins at the end; a, renamed, keeps its place.
        let album = update(&album, &declaration(), &[entry("/p/b.jpg", "1:2"), entry("/p/a2.jpg", "1:1")], |e| {
            e.path.starts_with("/away")
        });
        let paths: Vec<_> = album.files.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(paths, ["/p/a2.jpg", "/away/x.jpg", "/p/b.jpg"]);
        let arranged = arrange(&album, &[entry("/p/b.jpg", "1:2")]);
        let paths: Vec<_> = arranged.files.iter().map(|e| e.path.to_str().unwrap()).collect();
        assert_eq!(paths, ["/p/b.jpg", "/p/a2.jpg", "/away/x.jpg"]);
    }

    #[test]
    fn odd_tokens_name_no_file() {
        assert!(album_path("../evil").is_none());
        assert!(album_path("").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn identity_survives_a_rename() {
        let dir = std::env::temp_dir().join(format!("pico-test-album-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.jpg"), dir.join("b.jpg"));
        std::fs::write(&a, "x").unwrap();
        let id = file_id(&a).unwrap();
        std::fs::rename(&a, &b).unwrap();
        assert_eq!(file_id(&b), Some(id));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
