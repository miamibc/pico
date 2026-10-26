//! Pico's own settings, kept in the platform config directory.
//!
//! For now these are the folder lists, one path per line, in the same spirit as
//! Picasa's `watchedfolders.txt` and `frexcludefolders.txt`.

use std::io;
use std::path::{Path, PathBuf};

pub fn config_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|home| home.join(".config")))
    }?;
    Some(base.join("pico"))
}

/// Where disposable data such as thumbnails goes.
pub fn cache_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|home| home.join("Library/Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|home| home.join(".cache")))
    }?;
    Some(base.join("pico"))
}

fn list_file(name: &str) -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(name))
}

fn load_list(name: &str) -> Vec<PathBuf> {
    let Some(file) = list_file(name) else { return Vec::new() };
    std::fs::read_to_string(file)
        .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default()
}

fn save_list(name: &str, folders: &[PathBuf]) -> io::Result<()> {
    let file = list_file(name).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    let text: String = folders.iter().map(|p| format!("{}\n", p.display())).collect();
    std::fs::write(file, text)
}

pub fn load_watched_folders() -> Vec<PathBuf> {
    load_list("watched_folders.txt")
}

pub fn save_watched_folders(folders: &[PathBuf]) -> io::Result<()> {
    save_list("watched_folders.txt", folders)
}

/// Folders inside watched ones that the user left out of the library.
pub fn load_excluded_folders() -> Vec<PathBuf> {
    load_list("excluded_folders.txt")
}

pub fn save_excluded_folders(folders: &[PathBuf]) -> io::Result<()> {
    save_list("excluded_folders.txt", folders)
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The user's pictures folder, used when nothing is watched yet.
pub fn default_pictures_dir() -> Option<PathBuf> {
    let pictures = home_dir()?.join("Pictures");
    pictures.is_dir().then_some(pictures)
}
