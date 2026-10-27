//! Pico's own settings, kept in the platform config directory.
//!
//! The folder lists, one path per line, in the same spirit as Picasa's
//! `watchedfolders.txt` and `frexcludefolders.txt`, and `settings.txt` with
//! `key=value` lines for everything else the window remembers.

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

pub type Settings = std::collections::BTreeMap<String, String>;

pub fn load_settings() -> Settings {
    let Some(file) = list_file("settings.txt") else { return Settings::new() };
    parse_settings(&std::fs::read_to_string(file).unwrap_or_default())
}

pub fn save_settings(settings: &Settings) -> io::Result<()> {
    let file =
        list_file("settings.txt").ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory"))?;
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(file, format_settings(settings))
}

fn parse_settings(text: &str) -> Settings {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .filter(|(key, _)| !key.is_empty() && !key.starts_with('#'))
        .collect()
}

fn format_settings(settings: &Settings) -> String {
    // Line breaks would split a value; nothing Pico stores has them.
    settings.iter().map(|(key, value)| format!("{key}={}\n", value.replace('\n', " "))).collect()
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The user's pictures folder, used when nothing is watched yet.
pub fn default_pictures_dir() -> Option<PathBuf> {
    let pictures = home_dir()?.join("Pictures");
    pictures.is_dir().then_some(pictures)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip() {
        let mut settings = Settings::new();
        settings.insert("thumb_size".into(), "200".into());
        settings.insert("last_folder".into(), "/home/me/Pictures/a=b".into());
        let text = format_settings(&settings);
        assert_eq!(parse_settings(&text), settings, "{text}");
        assert_eq!(parse_settings("# a comment\nbroken line\n x = 1 \n").get("x").map(String::as_str), Some("1"));
    }
}
