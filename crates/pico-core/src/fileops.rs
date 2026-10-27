//! File operations that keep `.picasa.ini` in step: rename, move to another
//! folder, move to trash, and handing files to the desktop (open in another
//! program, show in folder).

use crate::export::ORIGINALS_DIR;
use crate::library::Folder;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Renames a file in place, carrying its INI section and saved original along.
pub fn rename(folder: &mut Folder, index: usize, new_name: &str) -> io::Result<()> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains(['/', '\\']) || new_name == "." || new_name == ".." {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a valid file name"));
    }
    let old_name = folder.files[index].name();
    if new_name == old_name {
        return Ok(());
    }
    let target = folder.path.join(new_name);
    if target.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{new_name} already exists")));
    }
    std::fs::rename(&folder.files[index].path, &target)?;
    let originals = folder.path.join(ORIGINALS_DIR);
    if originals.join(&old_name).is_file() {
        let _ = std::fs::rename(originals.join(&old_name), originals.join(new_name));
    }
    if let Some(section) = folder.ini.sections.iter_mut().find(|s| s.name == old_name) {
        section.name = new_name.to_owned();
    }
    folder.files[index].path = target;
    folder.ini.save(&folder.ini_path)
}

/// Moves a file into another folder with its INI section, its saved original
/// and the declarations of the albums it is in. Never overwrites: a file of
/// the same name in `dest` is an error. Works across disks by copying.
pub fn move_to_folder(src: &mut Folder, index: usize, dest: &mut Folder) -> io::Result<()> {
    let name = src.files[index].name();
    let target = dest.path.join(&name);
    if src.path == dest.path {
        return Ok(());
    }
    if target.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{name} is already there")));
    }
    move_path(&src.files[index].path, &target)?;
    let original = src.path.join(ORIGINALS_DIR).join(&name);
    if original.is_file() {
        let originals = dest.path.join(ORIGINALS_DIR);
        std::fs::create_dir_all(&originals)?;
        let _ = move_path(&original, &originals.join(&name));
    }
    if let Some(position) = src.ini.sections.iter().position(|s| s.name == name) {
        let section = src.ini.sections.remove(position);
        for token in &src.files[index].meta.albums {
            let album = format!("{}{token}", crate::library::ALBUM_PREFIX);
            if dest.ini.section(&album).is_none()
                && let Some(declaration) = src.ini.section(&album).cloned()
            {
                dest.ini.sections.push(declaration);
            }
        }
        dest.ini.sections.retain(|s| s.name != name);
        dest.ini.sections.push(section);
        dest.ini.save(&dest.ini_path)?;
        src.ini.save(&src.ini_path)?;
    }
    let mut file = src.files.remove(index);
    file.path = target;
    dest.files.push(file);
    Ok(())
}

/// `rename`, or copy and delete when the target is on another disk.
fn move_path(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)?;
    std::fs::remove_file(from)
}

/// Moves a file to the trash and drops its INI section. Returns where it went.
pub fn trash(folder: &mut Folder, index: usize) -> io::Result<PathBuf> {
    let file = folder.files[index].clone();
    let trashed = move_to_trash(&file.path)?;
    let name = file.name();
    folder.ini.sections.retain(|s| s.name != name);
    folder.files.remove(index);
    if folder.ini_path.exists() {
        folder.ini.save(&folder.ini_path)?;
    }
    Ok(trashed)
}

/// Moves a file into the desktop trash: the freedesktop.org Trash on Linux and
/// other Unix desktops, the Recycle Bin on Windows, the Finder's Trash on macOS.
pub fn move_to_trash(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    if cfg!(windows) {
        return run_trash_command(
            Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(windows_recycle_script(&absolute)),
        )
        .map(|()| absolute);
    }
    if cfg!(target_os = "macos") {
        return run_trash_command(Command::new("osascript").arg("-e").arg(finder_delete_script(&absolute)))
            .map(|()| absolute);
    }
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home directory"))?;
    move_to_trash_in(path, &data_home.join("Trash"))
}

fn run_trash_command(command: &mut Command) -> io::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned()))
    }
}

/// PowerShell that sends one file to the Recycle Bin. Single quotes in the
/// path are doubled, which is how PowerShell escapes them in literals.
fn windows_recycle_script(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\'', "''");
    format!(
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile('{path}', 'OnlyErrorDialogs', 'SendToRecycleBin')"
    )
}

/// AppleScript that asks the Finder to move one file to the Trash.
fn finder_delete_script(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!("tell application \"Finder\" to delete POSIX file \"{path}\"")
}

fn move_to_trash_in(path: &Path, trash: &Path) -> io::Result<PathBuf> {
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let name = name.to_string_lossy();
    // Pick a name free in both folders, as the spec requires.
    let unique = (0..)
        .map(|n| if n == 0 { name.to_string() } else { format!("{name}.{n}") })
        .find(|n| !files.join(n).exists() && !info.join(format!("{n}.trashinfo")).exists())
        .expect("a free name");
    let absolute = std::path::absolute(path)?;
    let trashinfo =
        format!("[Trash Info]\nPath={}\nDeletionDate={}\n", url_encode(&absolute.to_string_lossy()), local_timestamp());
    std::fs::write(info.join(format!("{unique}.trashinfo")), trashinfo)?;
    let target = files.join(&unique);
    if std::fs::rename(path, &target).is_err() {
        // Across file systems: copy, then remove.
        std::fs::copy(path, &target)?;
        std::fs::remove_file(path)?;
    }
    Ok(target)
}

fn url_encode(path: &str) -> String {
    path.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `YYYY-MM-DDThh:mm:ss` in UTC (the spec asks for local time; UTC avoids a
/// time zone dependency and file managers accept it).
fn local_timestamp() -> String {
    timestamp_at(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// `YYYY:MM:DD HH:MM:SS` in UTC, the EXIF date layout.
pub fn exif_style_date(secs: u64) -> String {
    timestamp_at(secs).replacen('-', ":", 2).replacen('T', " ", 1)
}

fn timestamp_at(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Opens a file with the program the desktop associates with it.
pub fn open_external(path: &Path) -> io::Result<()> {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]).arg(path);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path);
        c
    };
    command.spawn().map(|_| ())
}

/// Makes the picture the desktop wallpaper (GNOME and other desktops that use
/// the GNOME background settings).
pub fn set_wallpaper(path: &Path) -> io::Result<()> {
    let uri = format!("file://{}", url_encode(&std::path::absolute(path)?.to_string_lossy()));
    for key in ["picture-uri", "picture-uri-dark"] {
        // The dark-style key only exists on newer GNOME; ignore it failing.
        let status = Command::new("gsettings").args(["set", "org.gnome.desktop.background", key, &uri]).status()?;
        if !status.success() && key == "picture-uri" {
            return Err(io::Error::other("this desktop does not take wallpapers from gsettings"));
        }
    }
    Ok(())
}

/// Shows the file in the system file manager, selected where possible.
pub fn reveal(path: &Path) -> io::Result<()> {
    if cfg!(windows) {
        return Command::new("explorer").arg(format!("/select,{}", path.display())).spawn().map(|_| ());
    }
    if cfg!(target_os = "macos") {
        return Command::new("open").arg("-R").arg(path).spawn().map(|_| ());
    }
    // The FileManager1 D-Bus interface selects the file in Nautilus, Dolphin, Nemo…
    let uri = format!("file://{}", url_encode(&std::path::absolute(path)?.to_string_lossy()));
    let selected = Command::new("dbus-send")
        .args(["--session", "--dest=org.freedesktop.FileManager1", "--type=method_call"])
        .args(["/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems"])
        .arg(format!("array:string:{uri}"))
        .arg("string:")
        .status()
        .is_ok_and(|s| s.success());
    if selected { Ok(()) } else { open_external(path.parent().unwrap_or(path)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pico-test-fileops-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rename_moves_ini_section_and_backup() {
        let dir = temp_dir("rename");
        image::RgbImage::new(4, 4).save(dir.join("a.png")).unwrap();
        image::RgbImage::new(4, 4).save(dir.join("b.png")).unwrap();
        std::fs::create_dir_all(dir.join(ORIGINALS_DIR)).unwrap();
        std::fs::write(dir.join(ORIGINALS_DIR).join("a.png"), b"original").unwrap();
        std::fs::write(dir.join(".picasa.ini"), "[a.png]\r\nstar=yes\r\n").unwrap();
        let mut folders = library::scan(&dir);
        let folder = &mut folders[0];
        assert!(rename(folder, 0, "b.png").is_err(), "never overwrites");
        assert!(rename(folder, 0, "../x.png").is_err());
        rename(folder, 0, "sunset.png").unwrap();
        assert!(dir.join("sunset.png").is_file() && !dir.join("a.png").exists());
        assert!(dir.join(ORIGINALS_DIR).join("sunset.png").is_file());
        assert!(std::fs::read_to_string(dir.join(".picasa.ini")).unwrap().contains("[sunset.png]\r\nstar=yes"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_carries_metadata_backup_and_albums() {
        let dir = temp_dir("move");
        let (from, to) = (dir.join("from"), dir.join("to"));
        for d in [&from, &to] {
            std::fs::create_dir_all(d).unwrap();
        }
        image::RgbImage::new(4, 4).save(from.join("a.png")).unwrap();
        image::RgbImage::new(4, 4).save(from.join("b.png")).unwrap();
        image::RgbImage::new(4, 4).save(to.join("b.png")).unwrap();
        std::fs::create_dir_all(from.join(ORIGINALS_DIR)).unwrap();
        std::fs::write(from.join(ORIGINALS_DIR).join("a.png"), b"original").unwrap();
        std::fs::write(
            from.join(".picasa.ini"),
            "[.album:t1]\r\nname=Trip\r\ntoken=t1\r\n[a.png]\r\nstar=yes\r\nalbums=t1\r\n[b.png]\r\nstar=yes\r\n",
        )
        .unwrap();
        let mut folders = library::scan(&dir);
        let (src, dest) = folders.split_at_mut(1);
        let (src, dest) = (&mut src[0], &mut dest[0]);
        assert_eq!((src.path.clone(), dest.path.clone()), (from.clone(), to.clone()));
        assert!(move_to_folder(src, 1, dest).is_err(), "b.png is already there");
        move_to_folder(src, 0, dest).unwrap();
        assert!(to.join("a.png").is_file() && !from.join("a.png").exists());
        assert!(to.join(ORIGINALS_DIR).join("a.png").is_file());
        let to_ini = std::fs::read_to_string(to.join(".picasa.ini")).unwrap();
        assert!(
            to_ini.contains("[.album:t1]\r\nname=Trip") && to_ini.contains("[a.png]\r\nstar=yes\r\nalbums=t1"),
            "{to_ini}"
        );
        let from_ini = std::fs::read_to_string(from.join(".picasa.ini")).unwrap();
        assert!(!from_ini.contains("[a.png]") && from_ini.contains("[b.png]"), "{from_ini}");
        assert_eq!(dest.files.iter().map(|f| f.name()).collect::<Vec<_>>(), ["b.png", "a.png"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_follows_the_freedesktop_layout() {
        let dir = temp_dir("trash");
        let trash = dir.join("Trash");
        std::fs::write(dir.join("photo 1.jpg"), b"x").unwrap();
        std::fs::write(dir.join("other.jpg"), b"y").unwrap();
        let first = move_to_trash_in(&dir.join("photo 1.jpg"), &trash).unwrap();
        assert_eq!(first, trash.join("files/photo 1.jpg"));
        let info = std::fs::read_to_string(trash.join("info/photo 1.jpg.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"), "{info}");
        assert!(info.contains("photo%201.jpg\nDeletionDate=20"), "{info}");
        // Same name again gets a different trash name.
        std::fs::write(dir.join("photo 1.jpg"), b"z").unwrap();
        let second = move_to_trash_in(&dir.join("photo 1.jpg"), &trash).unwrap();
        assert_eq!(second, trash.join("files/photo 1.jpg.1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_scripts_escape_paths() {
        let win = windows_recycle_script(Path::new(r"C:\Photos\Ann's day.jpg"));
        assert!(
            win.contains(r"DeleteFile('C:\Photos\Ann''s day.jpg', 'OnlyErrorDialogs', 'SendToRecycleBin')"),
            "{win}"
        );
        let mac = finder_delete_script(Path::new(r#"/Users/a/say "cheese".jpg"#));
        assert_eq!(mac, r#"tell application "Finder" to delete POSIX file "/Users/a/say \"cheese\".jpg""#);
    }

    #[test]
    fn timestamps_look_like_iso_dates() {
        let t = local_timestamp();
        assert_eq!(t.len(), 19);
        assert_eq!(&t[4..5], "-");
        assert_eq!(&t[10..11], "T");
        assert_eq!(timestamp_at(0), "1970-01-01T00:00:00");
        assert_eq!(exif_style_date(0), "1970:01:01 00:00:00");
        assert_eq!(timestamp_at(951_782_400), "2000-02-29T00:00:00");
        assert_eq!(timestamp_at(1_793_035_686), "2026-10-26T17:28:06");
    }
}
