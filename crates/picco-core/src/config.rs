//! Picco's own settings, kept in the platform config directory.
//!
//! The folder lists, one path per line, in the same spirit as Picasa's
//! `watchedfolders.txt` and `frexcludefolders.txt`, and `settings.txt` with
//! `key=value` lines for everything else the window remembers.

use std::io;
use std::path::{Path, PathBuf};

pub fn config_dir() -> Option<PathBuf> {
    take_over_pico();
    Some(config_base()?.join("picco"))
}

/// Where disposable data such as thumbnails goes.
pub fn cache_dir() -> Option<PathBuf> {
    take_over_pico();
    Some(cache_base()?.join("picco"))
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The platform's folder for settings, where Picco keeps its own.
fn config_base() -> Option<PathBuf> {
    if cfg!(windows) {
        env_dir("APPDATA")
    } else if cfg!(target_os = "macos") {
        env_dir("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        env_dir("XDG_CONFIG_HOME").or_else(|| env_dir("HOME").map(|home| home.join(".config")))
    }
}

/// The platform's folder for caches.
fn cache_base() -> Option<PathBuf> {
    if cfg!(windows) {
        env_dir("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env_dir("HOME").map(|home| home.join("Library/Caches"))
    } else {
        env_dir("XDG_CACHE_HOME").or_else(|| env_dir("HOME").map(|home| home.join(".cache")))
    }
}

/// Once per run, before Picco's folders are used: see [`take_over`].
fn take_over_pico() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let (Some(config), Some(cache)) = (config_base(), cache_base()) {
            take_over(&config, &cache);
        }
    });
}

/// The program used to be called Pico. Its settings folder (with the folder
/// list, settings and albums) and its cache are taken over under the new name
/// the first time — but only when the settings folder is Pico's: other
/// programs called pico (the text editor) keep theirs.
fn take_over(config: &Path, cache: &Path) {
    let (old, new) = (config.join("pico"), config.join("picco"));
    if new.exists() || !is_ours(&old) {
        return;
    }
    if std::fs::rename(&old, &new).is_err() {
        return;
    }
    let (old, new) = (cache.join("pico"), cache.join("picco"));
    if !new.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &new);
    }
}

/// Whether a settings folder holds Pico's files: its folder lists, or its
/// `settings.txt` with keys only Pico writes.
fn is_ours(dir: &Path) -> bool {
    if ["watched_folders.txt", "excluded_folders.txt"].iter().any(|name| dir.join(name).is_file()) {
        return true;
    }
    std::fs::read_to_string(dir.join("settings.txt")).is_ok_and(|text| {
        let settings = parse_settings(&text);
        ["sidebar_width", "thumb_size", "left_sections"].iter().any(|key| settings.contains_key(*key))
    })
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
    // Line breaks would split a value; nothing Picco stores has them.
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

    #[test]
    fn takes_over_only_the_folders_of_our_pico() {
        let base = std::env::temp_dir().join(format!("picco-test-take-over-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (config, cache) = (base.join("config"), base.join("cache"));
        let make = |dir: &Path, file: &str, text: &str| {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(file), text).unwrap();
        };

        // Another program's pico (the text editor): left alone, cache too.
        make(&config.join("pico"), "settings.txt", "fill=72\n");
        make(&cache.join("pico"), "search_history", "x\n");
        take_over(&config, &cache);
        assert!(config.join("pico").exists() && !config.join("picco").exists());
        assert!(cache.join("pico").exists() && !cache.join("picco").exists());

        // Our Pico: the settings and the cache move.
        make(&config.join("pico"), "settings.txt", "theme=dark\nsidebar_width=260\n");
        take_over(&config, &cache);
        assert!(!config.join("pico").exists() && config.join("picco/settings.txt").is_file());
        assert!(!cache.join("pico").exists() && cache.join("picco/search_history").is_file());

        // Once there, an old folder appearing again is left alone.
        make(&config.join("pico"), "watched_folders.txt", "/photos\n");
        take_over(&config, &cache);
        assert!(config.join("pico/watched_folders.txt").is_file());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
