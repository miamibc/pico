use crate::collage_view::{CollageAction, CollageView};
use crate::filmstrip;
use crate::folder_manager::{FolderManager, FolderManagerAction};
use crate::grid::{
    self, CaptionMode, FileRef, GridAction, GridSection, GridView, MAX_THUMB, MIN_THUMB, PhotoDrag, SectionHeader,
};
use crate::info::{InfoPanel, TagAction};
use crate::slideshow::{Slideshow, SlideshowAction};
use crate::thumbs::ThumbCache;
use crate::viewer::{Viewer, ViewerAction};
use eframe::egui;
use pico_core::album_file::{self, Album, Entry};
use pico_core::export::{self, ExportOptions};
use pico_core::filters::{Filter, FilterStack};
use pico_core::folders::FolderRules;
use pico_core::library::{self, Folder, MediaFile};
use pico_core::meta::PhotoMeta;
use pico_core::movie::{self, MovieOptions};
use pico_core::{config, fileops, video};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

#[derive(PartialEq, Clone, Copy)]
enum View {
    Library,
    Photo,
    /// Full-screen slideshow; remembers where to return.
    Slideshow(ReturnTo),
    /// The Picture Collage workspace.
    Collage,
}

#[derive(PartialEq, Clone, Copy)]
enum ReturnTo {
    Library,
    Photo,
}

#[derive(PartialEq, Clone, Copy)]
enum SortBy {
    Name,
    /// Date taken (EXIF), else file date.
    Date,
    Size,
    /// The album's own order, arranged by dragging; albums only.
    Custom,
}

/// How the folder list is ordered.
#[derive(PartialEq, Clone, Copy)]
enum FolderSort {
    Name,
    /// By the date of the first photo, newest first, in year groups.
    Date,
    /// By the latest file change, newest first.
    Recent,
    /// Biggest first.
    Size,
}

/// A folder in the tree view; folders without photos of their own (just
/// subfolders) have no index.
struct FolderNode {
    name: String,
    folder: Option<usize>,
    children: Vec<FolderNode>,
}

/// What the grid lists when the search box is empty.
#[derive(PartialEq, Clone)]
enum Source {
    Folder(usize),
    /// Every starred photo in the library.
    Starred,
    /// A Picasa album, by token.
    Album(String),
    /// Result of Tools > Show Duplicate Files.
    Duplicates,
    /// Timeline: photos from one month, by `YYYY:MM` key.
    Month(String),
    /// Places: photos taken in one ~10 km cell, by place key.
    Place(String),
    /// The photos held in the Photo Tray.
    Tray,
}

pub struct PicoApp {
    /// Watched and excluded folders: what the library is made of.
    rules: FolderRules,
    folders: Vec<Folder>,
    source: Option<Source>,
    /// The photo the keyboard and the viewer work on.
    current: Option<FileRef>,
    /// Photos picked for batch commands (Ctrl/Shift-click, Ctrl+A).
    selection: BTreeSet<FileRef>,
    /// Photos held in the Photo Tray across folders (Ctrl+H), by path so they
    /// survive rescans.
    tray: Vec<PathBuf>,
    /// A folder, album, month or place picked in the list, for the ribbon to
    /// scroll to.
    ribbon_jump: Option<Source>,
    /// The ribbon's scroll offset last frame, to notice scrolling by hand.
    ribbon_offset: f32,
    /// Frames left in which offset changes come from the grid's own scrolling.
    ribbon_settle: u8,
    /// The photo last scrolled into view.
    revealed: Option<FileRef>,
    /// The selection before a selection rectangle started.
    band_base: Option<BTreeSet<FileRef>>,
    search: String,
    focus_search: bool,
    starred_only: bool,
    /// Show only videos (the ▶ filter).
    videos_only: bool,
    show_hidden: bool,
    captions: CaptionMode,
    sort_by: SortBy,
    reverse: bool,
    show_info: bool,
    /// Picasa's strip of neighbouring photos above the viewer.
    show_filmstrip: bool,
    /// Folders as a tree of their places on disk, rather than a flat list.
    folder_tree: bool,
    folder_sort: FolderSort,
    info: InfoPanel,
    /// "Copy All Effects" clipboard.
    copied_filters: Option<FilterStack>,
    /// Width of the tray's actions and size slider, as of the last frame.
    tray_actions_width: f32,
    /// Albums' own files (their photos in Custom Order), by token, read on
    /// first use.
    album_files: RefCell<HashMap<String, Album>>,
    /// A folder INI changed: the album files catch up in the next frame.
    albums_dirty: bool,
    /// Files' identities on disk, for matching them to album orders.
    file_ids: RefCell<HashMap<PathBuf, Option<String>>>,
    /// Kept open: on X11 the clipboard is served by whoever set it.
    clipboard: Option<arboard::Clipboard>,
    /// What Ctrl+C put on the clipboard, for tests (which leave the real one alone).
    #[cfg(test)]
    clipboard_files: Vec<PathBuf>,
    view: View,
    viewer: Viewer,
    slideshow: Option<Slideshow>,
    collage: Option<CollageView>,
    scan: Option<Receiver<Vec<Folder>>>,
    /// Media files found so far by the running scan.
    scan_found: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// The photo under the pointer in the grid, last frame.
    hovered: Option<FileRef>,
    thumbs: ThumbCache,
    thumb_size: f32,
    export: Option<ExportDialog>,
    export_job: Option<Receiver<ExportProgress>>,
    /// Pictures exported so far, and in all, for the progress window.
    export_count: (usize, usize),
    rename: Option<(FileRef, String)>,
    /// The photo clicked in the tray strip; Unhold works on it.
    tray_pick: Option<PathBuf>,
    new_album: Option<String>,
    /// Album being renamed: its token and the name typed so far.
    album_rename: Option<(String, String)>,
    /// Album to delete once confirmed, by token.
    confirm_album_delete: Option<String>,
    /// Duplicate files, group after group, once a search finished.
    duplicates: Vec<FileRef>,
    duplicate_job: Option<Receiver<Vec<Vec<PathBuf>>>>,
    folder_manager: Option<FolderManager>,
    movie: Option<MovieDialog>,
    movie_job: Option<Receiver<MovieProgress>>,
    /// Photos prepared so far, and in all; ffmpeg runs once all are.
    movie_frames: (usize, usize),
    /// A running system folder dialog and what its answer is for.
    folder_picker: Option<(FolderPurpose, Receiver<Option<PathBuf>>)>,
    picked_export_dir: Option<PathBuf>,
    show_options: bool,
    confirm_trash: Option<Vec<FileRef>>,
    /// Photos dropped on a folder, waiting for "Move" (folder index).
    confirm_move: Option<(Vec<FileRef>, usize)>,
    show_about: bool,
    /// The logo with the name; loaded the first time About opens.
    logo: Option<egui::TextureHandle>,
    show_shortcuts: bool,
    status: String,
    /// Light, dark, or as the system says.
    theme: egui::ThemePreference,
    /// The desktop's light or dark wish, where Pico reads it itself (Linux).
    system_dark: Option<bool>,
    system_theme: Option<Receiver<bool>>,
    /// Width of the folder panel, remembered between runs.
    sidebar_width: f32,
    /// Width of the Properties panel on the right.
    info_width: f32,
    /// Open (true) or closed sections of the left panel, by name, as last
    /// seen; remembered between runs.
    sections: RefCell<BTreeMap<String, bool>>,
    /// Sections whose remembered state was put back this run.
    sections_restored: RefCell<BTreeSet<String>>,
    /// The folder to open once the first scan is done.
    last_folder: Option<PathBuf>,
    /// Settings as last saved, and since when they have differed.
    saved_settings: config::Settings,
    settings_changed: Option<f64>,
    /// Whether the folder lists are written to the user's config.
    persist_settings: bool,
}

impl PicoApp {
    /// `watched` are the library roots. With `persist_settings` off (tests), the
    /// user's config and thumbnail cache are left untouched.
    pub fn new(ctx: &egui::Context, watched: Vec<PathBuf>, persist_settings: bool) -> Self {
        let excluded = if persist_settings { config::load_excluded_folders() } else { Vec::new() };
        let mut app = Self {
            rules: FolderRules { watched, excluded },
            folders: Vec::new(),
            source: None,
            current: None,
            selection: BTreeSet::new(),
            tray: Vec::new(),
            band_base: None,
            ribbon_jump: None,
            ribbon_offset: 0.0,
            ribbon_settle: 0,
            revealed: None,
            search: String::new(),
            focus_search: false,
            starred_only: false,
            videos_only: false,
            show_hidden: false,
            captions: CaptionMode::None,
            sort_by: SortBy::Name,
            reverse: false,
            show_info: false,
            show_filmstrip: true,
            folder_tree: false,
            folder_sort: FolderSort::Name,
            info: InfoPanel::default(),
            copied_filters: None,
            tray_actions_width: 480.0,
            album_files: RefCell::default(),
            albums_dirty: false,
            file_ids: RefCell::default(),
            clipboard: None,
            #[cfg(test)]
            clipboard_files: Vec::new(),
            view: View::Library,
            viewer: Viewer::default(),
            slideshow: None,
            collage: None,
            scan: None,
            scan_found: Default::default(),
            hovered: None,
            thumbs: ThumbCache::new(ctx, persist_settings),
            thumb_size: 160.0,
            export: None,
            export_job: None,
            export_count: (0, 0),
            rename: None,
            tray_pick: None,
            new_album: None,
            album_rename: None,
            confirm_album_delete: None,
            duplicates: Vec::new(),
            duplicate_job: None,
            folder_manager: None,
            movie: None,
            movie_job: None,
            movie_frames: (0, 0),
            folder_picker: None,
            picked_export_dir: None,
            show_options: false,
            confirm_trash: None,
            confirm_move: None,
            show_about: false,
            logo: None,
            show_shortcuts: false,
            status: String::new(),
            persist_settings,
            theme: egui::ThemePreference::System,
            system_dark: None,
            system_theme: None,
            sidebar_width: 260.0,
            info_width: 280.0,
            sections: RefCell::default(),
            sections_restored: RefCell::default(),
            last_folder: None,
            saved_settings: config::Settings::new(),
            settings_changed: None,
        };
        if persist_settings {
            // Albums whose photos are all in folders Pico does not watch
            // live only in their own files.
            for album in album_file::list() {
                app.album_files.get_mut().insert(album.token.clone(), album);
            }
            app.saved_settings = config::load_settings();
            app.apply_settings(&app.saved_settings.clone());
        }
        if persist_settings {
            app.system_theme = crate::system_theme::watch(ctx);
        }
        app.apply_theme(ctx);
        roomier(ctx);
        app.rescan(ctx);
        app
    }

    /// Saved folders plus any given on the command line; `~/Pictures` if none.
    pub fn startup_folders(extra: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut watched = config::load_watched_folders();
        for folder in extra {
            if !watched.contains(&folder) {
                watched.push(folder);
            }
        }
        if watched.is_empty() {
            watched.extend(config::default_pictures_dir());
        }
        watched
    }

    fn rescan(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let rules = self.rules.clone();
        let ctx = ctx.clone();
        let found = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        self.scan_found = found.clone();
        std::thread::spawn(move || {
            let mut folders: Vec<Folder> =
                rules.watched.iter().flat_map(|root| library::scan_counting(root, &rules.excluded, &found)).collect();
            folders.sort_by(|a, b| a.path.cmp(&b.path));
            folders.dedup_by(|a, b| a.path == b.path);
            library::sort_folders(&mut folders);
            let _ = tx.send(folders);
            ctx.request_repaint();
        });
        self.scan = Some(rx);
        self.status = tr!("Looking for photos… (they're usually hiding in Downloads)").into();
    }

    /// What Pico remembers between runs.
    fn settings(&self, ctx: &egui::Context) -> config::Settings {
        let mut s = self.saved_settings.clone();
        let mut set = |key: &str, value: String| {
            s.insert(key.to_owned(), value);
        };
        set("thumb_size", format!("{}", self.thumb_size.round()));
        set("captions", self.captions.key().to_owned());
        set(
            "sort",
            match self.sort_by {
                SortBy::Name => "name",
                SortBy::Date => "date",
                SortBy::Size => "size",
                SortBy::Custom => "custom",
            }
            .into(),
        );
        set("reverse", self.reverse.to_string());
        set("filmstrip", self.show_filmstrip.to_string());
        set("info_panel", self.show_info.to_string());
        set("info_sections", self.info.open_sections.iter().cloned().collect::<Vec<_>>().join(","));
        set("folder_tree", self.folder_tree.to_string());
        let folder_sort = match self.folder_sort {
            FolderSort::Name => "name",
            FolderSort::Date => "date",
            FolderSort::Recent => "recent",
            FolderSort::Size => "size",
        };
        set("folder_sort", folder_sort.into());
        set("sidebar_width", format!("{}", self.sidebar_width));
        set("info_width", format!("{}", self.info_width));
        let sections: Vec<String> = self
            .sections
            .borrow()
            .iter()
            .map(|(key, open)| format!("{}={}", escape_key(key), u8::from(*open)))
            .collect();
        set("left_sections", sections.join("|"));
        let theme = match self.theme {
            egui::ThemePreference::Dark => "dark",
            egui::ThemePreference::Light => "light",
            egui::ThemePreference::System => "system",
        };
        set("theme", theme.into());
        if let Some(Source::Folder(i)) = self.source
            && let Some(folder) = self.folders.get(i)
        {
            set("last_folder", folder.path.display().to_string());
        }
        // Not the size of a full-screen slideshow or photo.
        let viewport = ctx.input(|i| i.viewport().clone());
        if !viewport.fullscreen.unwrap_or(false) {
            set("maximized", viewport.maximized.unwrap_or(false).to_string());
            if !viewport.maximized.unwrap_or(false)
                && let Some(rect) = viewport.inner_rect
            {
                set("window_width", format!("{}", rect.width().round()));
                set("window_height", format!("{}", rect.height().round()));
            }
        }
        s
    }

    fn apply_settings(&mut self, s: &config::Settings) {
        let get = |key: &str| s.get(key).map(String::as_str);
        let flag = |key: &str, default: bool| get(key).map_or(default, |v| v == "true");
        if let Some(size) = get("thumb_size").and_then(|v| v.parse::<f32>().ok()) {
            self.thumb_size = size.clamp(MIN_THUMB, MAX_THUMB);
        }
        if let Some(mode) = get("captions").and_then(CaptionMode::from_key) {
            self.captions = mode;
        }
        self.sort_by = match get("sort") {
            Some("date") => SortBy::Date,
            Some("size") => SortBy::Size,
            Some("custom") => SortBy::Custom,
            _ => SortBy::Name,
        };
        self.reverse = flag("reverse", false);
        self.show_filmstrip = flag("filmstrip", true);
        self.show_info = flag("info_panel", false);
        self.info.open_sections =
            get("info_sections").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(str::to_owned).collect();
        self.folder_tree = flag("folder_tree", false);
        self.folder_sort = match get("folder_sort") {
            Some("date") => FolderSort::Date,
            Some("recent") => FolderSort::Recent,
            Some("size") => FolderSort::Size,
            _ => FolderSort::Name,
        };
        if let Some(width) = get("sidebar_width").and_then(|v| v.parse::<f32>().ok()) {
            self.sidebar_width = width.clamp(150.0, 600.0);
        }
        if let Some(width) = get("info_width").and_then(|v| v.parse::<f32>().ok()) {
            self.info_width = width.clamp(200.0, 900.0);
        }
        *self.sections.get_mut() = get("left_sections")
            .unwrap_or_default()
            .split('|')
            .filter_map(|item| {
                let (key, open) = item.rsplit_once('=')?;
                Some((unescape_key(key), open == "1"))
            })
            .collect();
        self.sections_restored.get_mut().clear();
        self.last_folder = get("last_folder").map(PathBuf::from);
        self.theme = match get("theme") {
            Some("dark") => egui::ThemePreference::Dark,
            Some("light") => egui::ThemePreference::Light,
            _ => egui::ThemePreference::System,
        };
    }

    /// Sets the chosen theme; "as the system" uses the desktop's wish when
    /// Pico knows it, else egui's own guess from the window system.
    fn apply_theme(&self, ctx: &egui::Context) {
        let theme = match (self.theme, self.system_dark) {
            (egui::ThemePreference::System, Some(true)) => egui::ThemePreference::Dark,
            (egui::ThemePreference::System, Some(false)) => egui::ThemePreference::Light,
            (theme, _) => theme,
        };
        ctx.set_theme(theme);
    }

    fn poll_system_theme(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.system_theme else { return };
        let mut changed = false;
        while let Ok(dark) = rx.try_recv() {
            self.system_dark = Some(dark);
            changed = true;
        }
        if changed {
            self.apply_theme(ctx);
        }
    }

    /// Saves the settings a second after they last changed, so dragging a
    /// window edge does not write the file every frame.
    fn save_settings_when_settled(&mut self, ctx: &egui::Context) {
        if !self.persist_settings {
            return;
        }
        let now = ctx.input(|i| i.time);
        let settings = self.settings(ctx);
        if settings == self.saved_settings {
            self.settings_changed = None;
            return;
        }
        let since = *self.settings_changed.get_or_insert(now);
        // Closing does not wait for the settings to settle.
        let closing = ctx.input(|i| i.viewport().close_requested());
        if now - since < 1.0 && !closing {
            ctx.request_repaint_after(std::time::Duration::from_millis(1100));
            return;
        }
        if let Err(e) = config::save_settings(&settings) {
            self.status = tr!("Could not save the settings: {e}", e = e);
        }
        self.saved_settings = settings;
        self.settings_changed = None;
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan else { return };
        let Ok(folders) = rx.try_recv() else { return };
        let total: usize = folders.iter().map(|f| f.files.len()).sum();
        self.status = format!("{}, {}", plural(folders.len(), "folder"), plural(total, "file"));
        let previous = match self.source {
            Some(Source::Folder(i)) => self.folders.get(i).map(|f| f.path.clone()),
            // On the first scan, the folder open when Pico was last closed.
            _ => self.last_folder.take(),
        };
        self.folders = folders;
        self.source = match previous.and_then(|p| self.folders.iter().position(|f| f.path == p)) {
            Some(i) => Some(Source::Folder(i)),
            None if matches!(self.source, Some(Source::Starred | Source::Album(_))) => self.source.take(),
            None if self.source == Some(Source::Duplicates) => Some(Source::Starred),
            None if self.source == Some(Source::Tray) => Some(Source::Tray),
            None => (!self.folders.is_empty()).then_some(Source::Folder(0)),
        };
        self.ribbon_jump = self.source.clone();
        self.current = None;
        self.selection.clear();
        self.view = View::Library;
        self.scan = None;
        self.albums_dirty = true;
    }

    fn watch_folder(&mut self, ctx: &egui::Context, path: PathBuf) {
        let mut rules = self.rules.clone();
        rules.set_in_library(&path, true);
        self.set_rules(ctx, rules);
    }

    /// Replaces the folder rules, saves them and rescans.
    fn set_rules(&mut self, ctx: &egui::Context, rules: FolderRules) {
        if rules != self.rules {
            self.rules = rules;
            if self.persist_settings
                && let Err(e) = config::save_watched_folders(&self.rules.watched)
                    .and_then(|()| config::save_excluded_folders(&self.rules.excluded))
            {
                self.status = tr!("Could not save the folder list: {e}", e = e);
            }
        }
        self.rescan(ctx);
    }

    /// Where the tray's photos are now; files that went away are skipped.
    fn tray_refs(&self) -> Vec<FileRef> {
        self.tray
            .iter()
            .filter_map(|path| {
                let fi = self.folders.iter().position(|f| Some(f.path.as_path()) == path.parent())?;
                let i = self.folders[fi].files.iter().position(|m| &m.path == path)?;
                Some((fi, i))
            })
            .collect()
    }

    /// What the tray shows and its buttons work on, as in Picasa: the held
    /// photos, then the selection not held yet.
    fn tray_contents(&self) -> Vec<FileRef> {
        let held = self.tray_refs();
        let more = self.selection.iter().copied().filter(|at| !held.contains(at));
        held.iter().copied().chain(more).collect()
    }

    /// Holds the selection in the tray (Ctrl+H).
    fn hold(&mut self) {
        for at in self.targets() {
            if let Some(file) = self.file(at)
                && !self.tray.contains(&file.path)
            {
                self.tray.push(file.path.clone());
            }
        }
    }

    /// Opens a tray photo in the viewer; with photos held, the viewer steps
    /// through the tray, else through the photos around it.
    fn view_in_tray(&mut self, at: FileRef) {
        if !self.tray.is_empty() {
            self.select_source(Source::Tray);
        }
        self.current = Some(at);
        self.selection = BTreeSet::from([at]);
        self.view = View::Photo;
    }

    /// Picks the photos tray actions work on: everything in the tray. It
    /// becomes the listing and the selection, so the existing commands
    /// (slideshow, export, collage, movie) see it.
    fn take_tray(&mut self) -> bool {
        let refs = self.tray_contents();
        self.select_source(Source::Tray);
        self.selection = refs.iter().copied().collect();
        self.current = refs.first().copied();
        !refs.is_empty()
    }

    fn tray_panel(&mut self, ui: &mut egui::Ui) {
        // Held photos first, then the selection not held yet, as in Picasa.
        let held_refs = self.tray_refs();
        let held = !self.tray.is_empty();
        let refs = self.tray_contents();
        let path_of = |app: &Self, at: FileRef| app.file(at).map(|f| f.path.clone());
        let picked = self
            .tray_pick
            .as_ref()
            .and_then(|p| refs.iter().copied().find(|&at| path_of(self, at).as_ref() == Some(p)));
        if picked.is_none() {
            self.tray_pick = None;
        }
        let mut actions_beside = true;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                ui.strong(tr!("Photo Tray"));
                let more = refs.len() - held_refs.len();
                let note = match (held_refs.len(), more) {
                    (0, 0) => tr!("Select photos, then Hold").to_owned(),
                    (0, n) => tr!("{photos} selected", photos = plural(n, "photo")),
                    (h, 0) => tr!("{photos} held", photos = plural(h, "photo")),
                    (h, n) => tr!("{photos} held, {n} selected", photos = plural(h, "photo"), n = n),
                };
                ui.label(egui::RichText::new(note).weak());
                ui.horizontal(|ui| {
                    let any = !self.targets().is_empty();
                    let hold = ui.add_enabled(any, egui::Button::new(tr!("Hold")));
                    if hold.on_hover_text(tr!("Keep the selection in the tray (Ctrl+H)")).clicked() {
                        self.hold();
                    }
                    // The photo picked in the tray, else the held ones selected.
                    let picked_held = picked.filter(|at| held_refs.contains(at));
                    let leaving: Vec<PathBuf> = match picked_held {
                        Some(at) => path_of(self, at).into_iter().collect(),
                        None => self.selection.iter().filter_map(|&at| path_of(self, at)).collect(),
                    };
                    let any_held = leaving.iter().any(|p| self.tray.contains(p));
                    let unhold = ui.add_enabled(any_held, egui::Button::new(tr!("Unhold")));
                    if unhold.on_hover_text(tr!("Take the picked or selected photos out of the tray")).clicked() {
                        self.tray.retain(|p| !leaving.contains(p));
                    }
                    let clear = ui.add_enabled(held, egui::Button::new(tr!("Clear")));
                    if clear.on_hover_text(tr!("Empty the tray; the photos stay where they are")).clicked() {
                        self.tray.clear();
                        if self.source == Some(Source::Tray) {
                            self.source = None;
                        }
                    }
                });
            });
            ui.separator();
            // The strip gets what the actions leave, as wide as they were last
            // frame; where even that is too tight, the actions go below.
            actions_beside = ui.available_width() >= self.tray_actions_width + 120.0;
            let reserve = if actions_beside { self.tray_actions_width } else { 0.0 };
            let strip_width = (ui.available_width() - reserve).max(0.0);
            self.tray_strip(ui, strip_width, &refs, &held_refs, picked);
            if actions_beside {
                ui.separator();
                self.tray_actions(ui, !refs.is_empty(), held);
            }
        });
        if !actions_beside {
            self.tray_actions(ui, !refs.is_empty(), held);
        }
    }

    /// Slideshow, Export, Collage and Movie for the tray, and the thumbnail
    /// size; remembers how wide they are.
    fn tray_actions(&mut self, ui: &mut egui::Ui, any: bool, held: bool) {
        let actions = egui::Grid::new("tray actions").num_columns(2).show(ui, |ui| {
            if ui.add_enabled(any, egui::Button::new(tr!("▶ Slideshow"))).clicked() && self.take_tray() {
                self.start_slideshow(ui.ctx());
            }
            if ui.add_enabled(any, egui::Button::new(tr!("Export…"))).clicked() && self.take_tray() {
                self.open_export();
            }
            ui.end_row();
            if ui.add_enabled(any, egui::Button::new(tr!("Make a Collage…"))).clicked() && self.take_tray() {
                self.open_collage(ui.ctx());
            }
            if ui.add_enabled(any, egui::Button::new(tr!("Make a Movie…"))).clicked() && self.take_tray() {
                self.open_movie_dialog();
            }
            ui.end_row();
            if held && ui.button(tr!("Show Tray")).clicked() {
                self.select_source(Source::Tray);
            }
            if !held {
                ui.label("");
            }
            // The thumbnail size, under the actions to leave the strip room.
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("🗖").weak());
                let slider = egui::Slider::new(&mut self.thumb_size, MIN_THUMB..=MAX_THUMB).show_value(false);
                ui.add_sized([110.0, 18.0], slider).on_hover_text(tr!("Thumbnail size (Ctrl+wheel over the photos)"));
            });
        });
        let wanted = actions.response.rect.width() + 4.0 * ui.spacing().item_spacing.x;
        if (wanted - self.tray_actions_width).abs() > 0.5 {
            self.tray_actions_width = wanted;
            ui.ctx().request_repaint();
        }
    }

    /// The tray's photos in a strip, in whatever width is left.
    fn tray_strip(
        &mut self,
        ui: &mut egui::Ui,
        strip_width: f32,
        refs: &[FileRef],
        held_refs: &[FileRef],
        picked: Option<FileRef>,
    ) {
        let path_of = |app: &Self, at: FileRef| app.file(at).map(|f| f.path.clone());
        let items: Vec<(FileRef, &MediaFile)> =
            refs.iter().filter_map(|&at| Some((at, self.folders.get(at.0)?.files.get(at.1)?))).collect();
        let mut chosen = None;
        let mut menu = |ui: &mut egui::Ui, at: FileRef| {
            if ui.button(tr!("View")).clicked() {
                chosen = Some(TrayChoice::View(at));
            }
            if held_refs.contains(&at) {
                if ui.button(tr!("Unhold")).on_hover_text(tr!("Take the photo out of the tray")).clicked() {
                    chosen = Some(TrayChoice::Unhold(at));
                }
            } else if ui.button(tr!("Hold")).on_hover_text(tr!("Keep the photo in the tray")).clicked() {
                chosen = Some(TrayChoice::Hold(at));
            }
        };
        let clicked = ui
            .allocate_ui_with_layout(
                egui::vec2(strip_width, filmstrip::CELL),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let label = |name: &str| tr!("Tray: {name}", name = name);
                    filmstrip::show(ui, &items, picked, held_refs, &mut self.thumbs, label, Some(&mut menu))
                },
            )
            .inner;
        match clicked {
            Some(filmstrip::Pick::Clicked(at)) => {
                self.tray_pick = path_of(self, at);
                self.current = Some(at);
            }
            Some(filmstrip::Pick::DoubleClicked(at)) => chosen = Some(TrayChoice::View(at)),
            None => {}
        }
        match chosen {
            Some(TrayChoice::View(at)) => self.view_in_tray(at),
            Some(TrayChoice::Hold(at)) => {
                if let Some(path) = path_of(self, at)
                    && !self.tray.contains(&path)
                {
                    self.tray.push(path);
                }
            }
            Some(TrayChoice::Unhold(at)) => {
                if let Some(path) = path_of(self, at) {
                    self.tray.retain(|p| p != &path);
                }
            }
            None => {}
        }
    }

    fn file(&self, at: FileRef) -> Option<&MediaFile> {
        self.folders.get(at.0)?.files.get(at.1)
    }

    /// The files the grid shows, in order: the source folder (or all folders
    /// while searching or showing starred photos), minus hidden photos and,
    /// optionally, unstarred ones.
    fn listing(&self) -> Vec<FileRef> {
        self.listing_of(self.source.as_ref())
    }

    /// What `source` lists, with the search, filters and sort order applied.
    fn listing_of(&self, source: Option<&Source>) -> Vec<FileRef> {
        let query = self.search.trim().to_lowercase();
        let album = match source {
            Some(Source::Album(token)) if query.is_empty() => Some(token.as_str()),
            _ => None,
        };
        if source == Some(&Source::Duplicates) && query.is_empty() {
            return self.duplicates.iter().copied().filter(|&at| self.file(at).is_some()).collect();
        }
        if source == Some(&Source::Tray) && query.is_empty() {
            return self.tray_contents();
        }
        let month = match source {
            Some(Source::Month(key)) if query.is_empty() => Some(key.as_str()),
            _ => None,
        };
        let place = match source {
            Some(Source::Place(key)) if query.is_empty() => Some(key.as_str()),
            _ => None,
        };
        let by_date = month.is_some() || place.is_some();
        let all_folders = !query.is_empty()
            || matches!(source, Some(Source::Starred | Source::Album(_) | Source::Month(_) | Source::Place(_)));
        let folders: Vec<usize> = match source {
            _ if all_folders => (0..self.folders.len()).collect(),
            Some(&Source::Folder(i)) if i < self.folders.len() => vec![i],
            _ => Vec::new(),
        };
        let starred = self.starred_only || source == Some(&Source::Starred);
        let query = query.as_str();
        let mut listing: Vec<FileRef> = folders
            .into_iter()
            .flat_map(|fi| {
                let folder = &self.folders[fi];
                folder.files.iter().enumerate().filter_map(move |(i, f)| {
                    let in_album = album.is_none_or(|token| f.meta.albums.iter().any(|t| t == token));
                    let in_month = month.is_none_or(|key| f.sort_date().starts_with(key));
                    let in_place =
                        place.is_none_or(|key| f.gps.is_some_and(|(lat, lon)| library::place_key(lat, lon) == key));
                    let visible = in_album
                        && in_month
                        && in_place
                        && (self.show_hidden || !f.meta.hidden)
                        && (!starred || f.meta.star)
                        && (!self.videos_only || f.kind == pico_core::media::MediaKind::Video);
                    let matches = query.is_empty() || matches_query(folder, f, query);
                    (visible && matches).then_some((fi, i))
                })
            })
            .collect();
        // Custom Order is the albums' own; elsewhere it falls back to names.
        let sort_by = match (self.sort_by, album) {
            (SortBy::Custom, None) => SortBy::Name,
            (sort_by, _) => sort_by,
        };
        // Folders are already in name order; other orders (and names across
        // several folders) need sorting.
        if sort_by != SortBy::Name || all_folders {
            let file = |at: &FileRef| &self.folders[at.0].files[at.1];
            // Timeline and Places read best in the order the photos were taken.
            let sort_by = if by_date && sort_by == SortBy::Name { SortBy::Date } else { sort_by };
            match sort_by {
                SortBy::Name => listing.sort_by(|a, b| library::natural_cmp(&file(a).name(), &file(b).name())),
                SortBy::Date => listing.sort_by_cached_key(|at| file(at).sort_date()),
                SortBy::Size => listing.sort_by_key(|at| file(at).size),
                SortBy::Custom => {
                    // Photos the order does not know yet (added since, or in
                    // Picasa) follow, by date.
                    let order = self.album_order(album.unwrap_or_default());
                    let rank = self.ranks(&order);
                    listing.sort_by_cached_key(|at| {
                        let f = file(at);
                        (rank(&f.path).unwrap_or(usize::MAX), f.sort_date())
                    });
                }
            }
        }
        if self.reverse {
            listing.reverse();
        }
        listing
    }

    /// An album's own file, read from Pico's config on first use (tests keep
    /// it in memory only).
    fn album_file(&self, token: &str) -> Album {
        let mut files = self.album_files.borrow_mut();
        let persist = self.persist_settings;
        files
            .entry(token.to_owned())
            .or_insert_with(|| {
                if persist { album_file::load(token) } else { Album { token: token.to_owned(), ..Album::default() } }
            })
            .clone()
    }

    fn store_album_file(&mut self, album: Album) {
        if self.persist_settings
            && let Err(e) = album_file::save(&album)
        {
            self.status = tr!("Could not save the album: {e}", e = e);
        }
        self.album_files.borrow_mut().insert(album.token.clone(), album);
    }

    /// The album's photos in its Custom Order.
    fn album_order(&self, token: &str) -> Vec<Entry> {
        self.album_file(token).files
    }

    /// Brings every album's own file up to date with the folders' INIs: its
    /// photos and its declaration. Photos new to an album join at the end,
    /// by date.
    fn sync_album_files(&mut self) {
        for album in library::albums(&self.folders) {
            let declaration = format!("{}{}", library::ALBUM_PREFIX, album.token);
            let declaration =
                self.folders.iter().find_map(|f| f.ini.section(&declaration)).cloned().unwrap_or_default();
            let mut members: Vec<(String, Entry)> = album
                .members
                .iter()
                .filter_map(|&(fi, i)| {
                    let file = self.folders.get(fi)?.files.get(i)?;
                    Some((file.sort_date(), Entry { id: self.file_id(&file.path), path: file.path.clone() }))
                })
                .collect();
            members.sort_by(|a, b| a.0.cmp(&b.0));
            let members: Vec<Entry> = members.into_iter().map(|(_, m)| m).collect();
            let previous = self.album_file(&album.token);
            let updated = album_file::update(&previous, &declaration, &members, |e| !self.watched_path(&e.path));
            if updated != previous {
                self.store_album_file(updated);
            }
        }
    }

    /// Whether `path` is in one of the library's folders.
    fn watched_path(&self, path: &Path) -> bool {
        self.folders.iter().any(|f| Some(f.path.as_path()) == path.parent())
    }

    fn file_id(&self, path: &Path) -> Option<String> {
        self.file_ids.borrow_mut().entry(path.to_path_buf()).or_insert_with(|| album_file::file_id(path)).clone()
    }

    /// Where a path stands in `order`: by identity on disk, so renamed and
    /// moved photos keep their place, else by path.
    fn ranks<'a>(&'a self, order: &'a [Entry]) -> impl Fn(&Path) -> Option<usize> + 'a {
        let mut by_id = HashMap::new();
        let mut by_path = HashMap::new();
        for (i, e) in order.iter().enumerate().rev() {
            if let Some(id) = &e.id {
                by_id.insert(id.as_str(), i);
            }
            by_path.insert(e.path.as_path(), i);
        }
        move |path| {
            self.file_id(path).and_then(|id| by_id.get(id.as_str()).copied()).or_else(|| by_path.get(path).copied())
        }
    }

    /// Moves `photos` and the missing photos at tiles `missing` to stand
    /// before tile `to` (or last), and switches to Custom Order.
    fn move_in_album(&mut self, token: &str, photos: &[FileRef], missing: &[usize], to: usize) {
        self.sync_album_files();
        // The tiles as on screen: the photos, and the missing ones among them.
        let listing = self.listing_of(Some(&Source::Album(token.to_owned())));
        let mut tiles: Vec<Entry> = listing
            .iter()
            .filter_map(|&at| self.file(at))
            .map(|f| Entry { id: self.file_id(&f.path), path: f.path.clone() })
            .collect();
        for (place, entry) in self.album_missing(token, &listing) {
            tiles.insert(place.at.min(tiles.len()), entry);
        }
        let moving: BTreeSet<usize> = photos
            .iter()
            .filter_map(|&at| self.slot_of(&tiles, at))
            .chain(missing.iter().copied().filter(|&slot| slot < tiles.len()))
            .collect();
        if moving.is_empty() {
            return;
        }
        // In their order on screen, which a BTreeSet keeps.
        let moved: Vec<Entry> = moving.iter().map(|&slot| tiles[slot].clone()).collect();
        let mut arranged: Vec<Entry> =
            tiles.iter().enumerate().filter(|(slot, _)| !moving.contains(slot)).map(|(_, e)| e.clone()).collect();
        let at = (to - moving.iter().filter(|&&slot| slot < to).count()).min(arranged.len());
        arranged.splice(at..at, moved);
        // Let go where it was picked up (or a click that moved a little).
        if arranged == tiles {
            return;
        }
        // Reversed, the album's order is the other way round from the screen.
        if self.reverse {
            arranged.reverse();
        }
        // The tiles on screen take the places of the tiles on screen, in their
        // new order; photos hidden by a filter keep theirs.
        let album = self.album_file(token);
        let mut shown = arranged.iter();
        let mut order: Vec<Entry> = album
            .files
            .iter()
            .map(|e| if tiles.iter().any(|t| t.matches(e)) { shown.next().unwrap_or(e).clone() } else { e.clone() })
            .collect();
        order.extend(shown.cloned());
        let arranged = album_file::arrange(&album, &order);
        self.store_album_file(arranged);
        self.sort_by = SortBy::Custom;
    }

    /// Where the photo `at` is among `tiles`.
    fn slot_of(&self, tiles: &[Entry], at: FileRef) -> Option<usize> {
        let file = self.file(at)?;
        let entry = Entry { id: self.file_id(&file.path), path: file.path.clone() };
        tiles.iter().position(|t| t.matches(&entry))
    }

    /// Photos in the album's order that are not in the library: their folder
    /// is not watched, or they were moved or deleted.
    fn missing_from_album(&self, token: &str) -> Vec<Entry> {
        let watched = |path: &Path| self.folders.iter().any(|f| Some(f.path.as_path()) == path.parent());
        // A photo moved into a watched folder is in the album under its new path.
        let members: BTreeSet<String> = self
            .folders
            .iter()
            .flat_map(|f| &f.files)
            .filter(|f| f.meta.albums.iter().any(|t| t == token))
            .filter_map(|f| self.file_id(&f.path))
            .collect();
        self.album_order(token)
            .into_iter()
            .filter(|e| !watched(&e.path) && e.id.as_ref().is_none_or(|id| !members.contains(id)))
            .collect()
    }

    /// Says which photos of the album's Custom Order are not in the library:
    /// where they are, if their folders are just not in Pico.
    fn album_missing_note(&self, token: &str) -> Option<String> {
        let missing = self.missing_from_album(token);
        let (found, lost): (Vec<&Entry>, Vec<&Entry>) = missing
            .iter()
            .partition(|e| self.file_id(&e.path).is_some_and(|id| e.id.as_ref().is_none_or(|known| known == &id)));
        let mut folders: Vec<String> =
            found.iter().filter_map(|e| e.path.parent()).map(|p| p.display().to_string()).collect();
        folders.sort();
        folders.dedup();
        let mut lines = Vec::new();
        if !found.is_empty() {
            lines.push(trn!(
                "{n} photo of this album is in folders Pico does not watch; add them with File > Add Folder to Pico: {folders}",
                "{n} photos of this album are in folders Pico does not watch; add them with File > Add Folder to Pico: {folders}",
                found.len(),
                folders = folders.join(", ")
            ));
        }
        if !lost.is_empty() {
            lines.push(tr!(
                "{photos} of this album could not be found: moved or deleted.",
                photos = plural(lost.len(), "photo")
            ));
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// The album's photos that are not in the library, for black tiles: with
    /// their folder when they are there but the folder is not in Pico. In
    /// Custom Order each takes its place among the `listing` (mirrored when
    /// reversed); otherwise they follow it. By place.
    fn album_missing(&self, token: &str, listing: &[FileRef]) -> Vec<(grid::MissingPhoto, Entry)> {
        let order = self.album_order(token);
        let rank = self.ranks(&order);
        let mut ranks: Vec<usize> =
            listing.iter().filter_map(|&at| self.file(at)).map(|f| rank(&f.path).unwrap_or(usize::MAX)).collect();
        if self.reverse {
            ranks.reverse();
        }
        let custom = self.sort_by == SortBy::Custom;
        let missing = self.missing_from_album(token);
        let last = (listing.len() + missing.len()).saturating_sub(1);
        let mut tiles: Vec<(grid::MissingPhoto, Entry)> = missing
            .into_iter()
            .enumerate()
            .map(|(k, e)| {
                let place = order.iter().position(|o| o == &e).unwrap_or(usize::MAX);
                let in_order = k + ranks.iter().filter(|&&r| r < place).count();
                let at = match (custom, self.reverse) {
                    (false, _) => listing.len() + k,
                    (true, false) => in_order,
                    (true, true) => last - in_order,
                };
                let found = self.file_id(&e.path).is_some_and(|id| e.id.as_ref().is_none_or(|known| known == &id));
                let tile = grid::MissingPhoto {
                    at,
                    name: Folder::file_name(&e.path),
                    folder: found.then(|| e.path.parent().map(Path::to_path_buf)).flatten(),
                };
                (tile, e)
            })
            .collect();
        tiles.sort_by_key(|(tile, _)| tile.at);
        tiles
    }

    /// What batch commands act on: the selection, or the current photo.
    fn targets(&self) -> Vec<FileRef> {
        if self.view == View::Library && !self.selection.is_empty() {
            self.selection.iter().copied().collect()
        } else {
            self.current.into_iter().collect()
        }
    }

    /// Changes the metadata of `files` and writes each affected folder's INI once.
    fn edit_files(&mut self, files: &[FileRef], change: impl Fn(&mut PhotoMeta)) {
        let mut by_folder: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &(folder, file) in files {
            by_folder.entry(folder).or_default().push(file);
        }
        let mut saved = 0;
        for (folder_index, indices) in by_folder {
            let Some(folder) = self.folders.get_mut(folder_index) else { continue };
            let mut before = Vec::new();
            for &i in &indices {
                let Some(file) = folder.files.get_mut(i) else { continue };
                before.push((i, file.meta.clone()));
                change(&mut file.meta);
            }
            let changed: Vec<usize> =
                before.iter().filter(|(i, old)| folder.files[*i].meta != *old).map(|(i, _)| *i).collect();
            if changed.is_empty() {
                continue;
            }
            match folder.save_metas(&changed) {
                Ok(()) => {
                    self.albums_dirty = true;
                    saved += changed.len();
                    for (i, old) in &before {
                        let file = &folder.files[*i];
                        if file.meta.rotate != old.rotate || file.meta.filters != old.filters {
                            self.thumbs.invalidate(&file.path);
                        }
                    }
                    self.status = tr!("Saved to {path}", path = folder.ini_path.display());
                }
                Err(e) => {
                    // Keep the screen honest: undo changes that did not reach the disk.
                    for (i, old) in before {
                        folder.files[i].meta = old;
                    }
                    self.status = tr!("Could not write {path}: {e}", path = folder.ini_path.display(), e = e);
                    return;
                }
            }
        }
        if saved > 1 {
            self.status = tr!("Updated {photos}", photos = plural(saved, "photo"));
        }
    }

    fn edit_targets(&mut self, change: impl Fn(&mut PhotoMeta)) {
        let targets = self.targets();
        self.edit_files(&targets, change);
    }

    fn rotate_targets(&mut self, quarter_turns: i8) {
        self.edit_targets(|meta| meta.rotate = (meta.rotate as i8 + quarter_turns).rem_euclid(4) as u8);
    }

    /// Adds a one-click edit (I'm Feeling Lucky, B&W, …) to every target.
    fn add_effect(&mut self, id: &str) {
        let filter = Filter::effect(id, []);
        self.edit_targets(|meta| meta.filters.0.push(filter.clone()));
    }

    /// Ctrl+C: the selected files (or the open one) go on the clipboard as
    /// files, as a file manager copies them, for pasting into a chat, a mail
    /// or a folder.
    fn copy_files(&mut self) {
        let paths: Vec<PathBuf> =
            self.targets().iter().filter_map(|&at| self.file(at)).map(|f| f.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        self.status = match self.put_on_clipboard(&paths) {
            Ok(()) => tr!("Copied {files} to the clipboard", files = plural(paths.len(), "file")),
            Err(e) => tr!("Could not copy to the clipboard: {e}", e = e),
        };
    }

    #[cfg(not(test))]
    fn put_on_clipboard(&mut self, paths: &[PathBuf]) -> Result<(), arboard::Error> {
        let clipboard = match &mut self.clipboard {
            Some(clipboard) => clipboard,
            None => self.clipboard.insert(arboard::Clipboard::new()?),
        };
        clipboard.set().file_list(paths)
    }

    #[cfg(test)]
    fn put_on_clipboard(&mut self, paths: &[PathBuf]) -> Result<(), arboard::Error> {
        let _ = &self.clipboard;
        self.clipboard_files = paths.to_vec();
        Ok(())
    }

    fn copy_effects(&mut self) {
        if let Some(filters) = self.current.and_then(|at| self.file(at)).map(|f| f.meta.filters.clone()) {
            self.status = tr!("Copied {effects}", effects = plural(filters.0.len(), "effect"));
            self.copied_filters = Some(filters);
        }
    }

    fn paste_effects(&mut self) {
        if let Some(filters) = self.copied_filters.clone() {
            self.edit_targets(|meta| meta.filters.0.extend(filters.0.iter().cloned()));
        }
    }

    fn find_duplicates(&mut self, ctx: &egui::Context) {
        let files: Vec<(PathBuf, u64)> =
            self.folders.iter().flat_map(|f| &f.files).map(|f| (f.path.clone(), f.size)).collect();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(library::find_duplicates(&files));
            ctx.request_repaint();
        });
        self.duplicate_job = Some(rx);
        self.duplicates.clear();
        self.select_source(Source::Duplicates);
    }

    fn poll_duplicates(&mut self) {
        let Some(rx) = &self.duplicate_job else { return };
        let Ok(groups) = rx.try_recv() else { return };
        self.duplicate_job = None;
        let find = |path: &PathBuf| {
            self.folders
                .iter()
                .enumerate()
                .find_map(|(fi, f)| f.files.iter().position(|m| &m.path == path).map(|i| (fi, i)))
        };
        self.duplicates = groups.iter().flatten().filter_map(find).collect();
        self.status = match groups.len() {
            0 => tr!("No duplicates. Every photo is one of a kind.").into(),
            n => tr!("{groups} of identical files", groups = plural(n, "group")),
        };
    }

    /// Opens the system folder dialog on a helper thread so the window stays live.
    fn pick_folder(&mut self, ctx: &egui::Context, purpose: FolderPurpose) {
        if self.folder_picker.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let title = match purpose {
                FolderPurpose::Watch => tr!("Add a folder to Pico"),
                FolderPurpose::ExportTo => tr!("Export to folder"),
            };
            let _ = tx.send(rfd::FileDialog::new().set_title(title).pick_folder());
            ctx.request_repaint();
        });
        self.folder_picker = Some((purpose, rx));
    }

    fn poll_folder_picker(&mut self, ctx: &egui::Context) {
        let Some((purpose, rx)) = &self.folder_picker else { return };
        let Ok(answer) = rx.try_recv() else { return };
        let purpose = *purpose;
        self.folder_picker = None;
        match (purpose, answer) {
            (FolderPurpose::Watch, Some(dir)) => match &mut self.folder_manager {
                Some(manager) => manager.pick(dir),
                None => self.watch_folder(ctx, dir),
            },
            (FolderPurpose::ExportTo, Some(dir)) => self.picked_export_dir = Some(dir),
            (_, None) => {}
        }
    }

    fn open_folder_manager(&mut self) {
        if self.folder_manager.is_none() {
            self.folder_manager = Some(FolderManager::new(self.rules.clone()));
        }
    }

    fn open_collage(&mut self, ctx: &egui::Context) {
        let mut refs = self.targets();
        if refs.len() < 2 {
            refs = self.listing();
        }
        let files: Vec<MediaFile> = refs.iter().filter_map(|&at| self.file(at).cloned()).collect();
        if files.is_empty() {
            return;
        }
        let title = match &self.source {
            Some(Source::Folder(i)) => self.folders.get(*i).map(|f| f.name.clone()).unwrap_or_default(),
            Some(Source::Album(token)) => self.album_name(token).unwrap_or_default(),
            _ => String::new(),
        };
        self.collage = Some(CollageView::new(ctx, files, title));
        self.view = View::Collage;
    }

    fn collage(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.collage.as_mut() else {
            self.view = View::Library;
            return;
        };
        match view.show(ui) {
            Some(CollageAction::Close) => {
                self.collage = None;
                self.view = View::Library;
            }
            Some(CollageAction::Status(message)) => self.status = message,
            Some(CollageAction::Saved { path, wallpaper }) => {
                self.add_file(path.clone());
                self.add_to_collages_album(&path);
                self.status = tr!("Collage saved to {path}", path = path.display());
                if wallpaper {
                    match pico_core::fileops::set_wallpaper(&path) {
                        Ok(()) => {
                            self.status =
                                tr!("Collage saved to {path} and set as the desktop background", path = path.display())
                        }
                        Err(e) => self.status = tr!("Saved, but could not set the wallpaper: {e}", e = e),
                    }
                }
            }
            None => {}
        }
    }

    /// Picasa keeps collages in a "Collages" album; Pico does the same.
    fn add_to_collages_album(&mut self, path: &std::path::Path) {
        let Some(at) = self
            .folders
            .iter()
            .enumerate()
            .find_map(|(fi, f)| f.files.iter().position(|m| m.path == path).map(|i| (fi, i)))
        else {
            return;
        };
        let token = library::albums(&self.folders)
            .into_iter()
            .find(|a| a.name == "Collages")
            .map_or_else(library::new_album_token, |a| a.token);
        self.folders[at.0].declare_album(&token, "Collages");
        self.edit_files(&[at], |meta| {
            if !meta.albums.iter().any(|t| t == &token) {
                meta.albums.push(token.clone());
            }
        });
    }

    fn open_movie_dialog(&mut self) {
        let base = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(|h| PathBuf::from(h).join("Videos"))
            .filter(|p| p.is_dir())
            .or_else(config::default_pictures_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let out = (1..).map(|n| base.join(format!("Pico Movie {n}.mp4"))).find(|p| !p.exists()).expect("a free name");
        self.movie = Some(MovieDialog {
            out: out.display().to_string(),
            full_hd: false,
            seconds: 3.0,
            fade: 1.0,
            audio: String::new(),
            pick_audio: None,
        });
    }

    fn start_movie(&mut self, ctx: &egui::Context, dialog: &MovieDialog) {
        let files: Vec<MediaFile> = self.targets().into_iter().filter_map(|at| self.file(at).cloned()).collect();
        let files_count = files.len();
        let (width, height) = if dialog.full_hd { (1920, 1080) } else { (1280, 720) };
        let audio = Some(PathBuf::from(dialog.audio.trim())).filter(|p| !p.as_os_str().is_empty());
        let options = MovieOptions { width, height, seconds: dialog.seconds, fade: dialog.fade, audio };
        let out = PathBuf::from(dialog.out.trim());
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let progress = |done, total| {
                let _ = tx.send(MovieProgress::Frames(done, total));
                ctx.request_repaint();
            };
            let result = movie::make_movie(&files, &out, &options, progress).map(|()| out).map_err(|e| e.to_string());
            let _ = tx.send(MovieProgress::Finished(result));
            ctx.request_repaint();
        });
        self.movie_job = Some(rx);
        self.movie_frames = (0, files_count);
        self.status = tr!("Making a movie… popcorn recommended.").into();
    }

    fn poll_movie(&mut self) {
        let Some(rx) = &self.movie_job else { return };
        let mut finished = None;
        while let Ok(progress) = rx.try_recv() {
            match progress {
                MovieProgress::Frames(done, total) => {
                    self.movie_frames = (done, total);
                    self.status = tr!("Preparing photo {done} of {total}…", done = done, total = total);
                }
                MovieProgress::Finished(result) => finished = Some(result),
            }
        }
        if let Some(result) = finished {
            self.movie_job = None;
            match result {
                Ok(path) => {
                    self.status = tr!("Movie saved to {path}", path = path.display());
                    self.add_file(path);
                }
                Err(e) => self.status = tr!("Could not make the movie: {e}", e = e),
            }
        }
    }

    fn album_name(&self, token: &str) -> Option<String> {
        self.album_list().into_iter().find(|a| a.0 == token).map(|a| a.1)
    }

    /// Every album: token, name and its photos, missing ones included. Those
    /// declared in the folders first, then those with only a file of their
    /// own (none of their photos' folders is in Pico).
    fn album_list(&self) -> Vec<(String, String, usize)> {
        let mut list: Vec<(String, String, usize)> = library::albums(&self.folders)
            .into_iter()
            .map(|a| {
                let count = a.members.len() + self.missing_from_album(&a.token).len();
                (a.token, a.name, count)
            })
            .collect();
        let mut own: Vec<(String, String, usize)> = self
            .album_files
            .borrow()
            .values()
            .filter(|a| !list.iter().any(|l| l.0 == a.token))
            .map(|a| (a.token.clone(), a.name.clone(), a.files.len()))
            .collect();
        own.sort_by(|a, b| library::natural_cmp(&a.1, &b.1));
        list.extend(own);
        list
    }

    /// Renames the album in every folder that declares it.
    fn rename_album(&mut self, token: &str, name: &str) {
        for folder in &mut self.folders {
            if folder.rename_album(token, name)
                && let Err(e) = folder.ini.save(&folder.ini_path)
            {
                self.status = tr!("Could not write {path}: {e}", path = folder.ini_path.display(), e = e);
                return;
            }
        }
        // An album with only a file of its own is renamed there.
        let mut album = self.album_file(token);
        if album.name != name {
            album.name = name.to_owned();
            self.store_album_file(album);
        }
        self.albums_dirty = true;
        self.status = tr!("Renamed the album to \"{name}\"", name = name);
    }

    /// Deletes the album: its declarations, its photos' membership and its
    /// own file. The photos stay.
    fn delete_album(&mut self, token: &str) {
        let name = self.album_name(token).unwrap_or_default();
        for folder in &mut self.folders {
            let (left, changed) = folder.forget_album(token);
            if changed && let Err(e) = folder.save_metas(&left) {
                self.status = tr!("Could not write {path}: {e}", path = folder.ini_path.display(), e = e);
                return;
            }
        }
        if self.persist_settings
            && let Err(e) = album_file::remove(token)
        {
            self.status = tr!("Could not delete the album file: {e}", e = e);
            return;
        }
        self.album_files.borrow_mut().remove(token);
        if self.source == Some(Source::Album(token.to_owned())) {
            self.source = (!self.folders.is_empty()).then_some(Source::Folder(0));
        }
        self.status = tr!("Deleted the album \"{name}\"", name = name);
    }

    /// Delete: in an album the targets leave the album and the files stay;
    /// elsewhere they go to the trash, once confirmed.
    fn delete_targets(&mut self) {
        let targets = self.targets();
        let Some(Source::Album(token)) = self.source.clone() else {
            self.confirm_trash = Some(targets);
            return;
        };
        self.edit_files(&targets, |meta| meta.albums.retain(|t| t != &token));
        self.selection.clear();
        let name = self.album_name(&token).unwrap_or_default();
        self.status = tr!(
            "Took {photos} out of \"{name}\"; the files stay",
            photos = plural(targets.len(), "photo"),
            name = name
        );
    }

    /// Puts the targets into an album, declaring it in each affected folder.
    fn add_to_album(&mut self, token: &str, name: &str) {
        let targets = self.targets();
        for &(fi, _) in &targets {
            if let Some(folder) = self.folders.get_mut(fi) {
                folder.declare_album(token, name);
            }
        }
        self.edit_files(&targets, |meta| {
            if !meta.albums.iter().any(|t| t == token) {
                meta.albums.push(token.to_owned());
            }
        });
        self.status = tr!("Added {photos} to \"{name}\"", photos = plural(targets.len(), "photo"), name = name);
    }

    /// Folders (or files, by their folder) dropped from the file manager join
    /// the library.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| p.is_absolute()).collect()
        });
        if dropped.is_empty() {
            return;
        }
        let mut rules = self.rules.clone();
        let mut added = 0;
        for path in dropped {
            let dir = if path.is_dir() { Some(path) } else { path.parent().map(PathBuf::from) };
            if let Some(dir) = dir.filter(|d| !rules.state(d).in_library()) {
                rules.set_in_library(&dir, true);
                added += 1;
            }
        }
        if added > 0 {
            self.set_rules(ctx, rules);
        }
        self.status = match added {
            0 => tr!("Those are already in Pico.").to_owned(),
            n => tr!("Added {folders} to Pico", folders = plural(n, "folder")),
        };
    }

    /// While dragging: a note at the pointer, and a hint when files from the
    /// file manager hover over the window.
    fn drag_feedback(&self, ctx: &egui::Context) {
        let pointer = ctx.pointer_latest_pos();
        if let (Some(drag), Some(pos)) = (egui::DragAndDrop::payload::<PhotoDrag>(ctx), pointer) {
            egui::Area::new(egui::Id::unique("photo drag"))
                .order(egui::Order::Tooltip)
                .fixed_pos(pos + egui::vec2(16.0, 16.0))
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let photos = plural(drag.0.len() + drag.1.len(), "photo");
                        ui.label(if matches!(self.source, Some(Source::Album(_))) {
                            tr!(
                                "{photos}: drop on a folder, an album or the tray, or between photos to arrange them",
                                photos = photos
                            )
                        } else {
                            tr!("{photos}: drop on a folder, an album or the tray", photos = photos)
                        });
                    });
                });
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let screen = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("file drop")));
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(160));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                tr!("Drop folders here to add them to Pico"),
                egui::FontId::proportional(24.0),
                egui::Color32::WHITE,
            );
        }
    }

    /// Moves files into folder `dest` on disk, metadata included, then rescans.
    fn move_files(&mut self, ctx: &egui::Context, files: &[FileRef], dest: usize) {
        let mut files: Vec<FileRef> = files.iter().copied().filter(|at| at.0 != dest).collect();
        // From the back, so the indices of the files still to move hold.
        files.sort_unstable_by(|a, b| b.cmp(a));
        let (mut moved, mut failed) = (0, Vec::new());
        for (fi, i) in files {
            let (src, target) = if fi < dest {
                let (left, right) = self.folders.split_at_mut(dest);
                (&mut left[fi], &mut right[0])
            } else {
                let (left, right) = self.folders.split_at_mut(fi);
                (&mut right[0], &mut left[dest])
            };
            match fileops::move_to_folder(src, i, target) {
                Ok(()) => moved += 1,
                Err(e) => failed.push(e.to_string()),
            }
        }
        let name = self.folders.get(dest).map(|f| f.name.clone()).unwrap_or_default();
        self.rescan(ctx);
        self.status = match failed.first() {
            None => tr!("Moved {files} to \"{name}\"", files = plural(moved, "file"), name = name),
            Some(first) => {
                tr!(
                    "Moved {files}, {stayed} stayed: {first}",
                    files = plural(moved, "file"),
                    stayed = failed.len(),
                    first = first
                )
            }
        };
    }

    /// Full-screen photo viewing: the window fills the screen and only the
    /// photo is drawn.
    fn set_full_screen(&mut self, ctx: &egui::Context, on: bool) {
        if self.viewer.is_full_screen() != on {
            self.viewer.set_full_screen(on);
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(on));
        }
    }

    fn start_slideshow(&mut self, ctx: &egui::Context) {
        let listing = self.listing();
        if listing.is_empty() {
            return;
        }
        let start = self.current.and_then(|c| listing.iter().position(|&r| r == c)).unwrap_or(0);
        let back = if self.view == View::Photo { ReturnTo::Photo } else { ReturnTo::Library };
        self.slideshow = Some(Slideshow::new(start));
        self.view = View::Slideshow(back);
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
    }

    fn slideshow(&mut self, ui: &mut egui::Ui, back: ReturnTo) {
        let listing = self.listing();
        let files: Vec<MediaFile> = listing.iter().filter_map(|&at| self.file(at).cloned()).collect();
        let Some(show) = self.slideshow.as_mut() else {
            self.view = View::Library;
            return;
        };
        let action = show.show(ui, &files);
        // Leaving the slideshow selects the photo it stopped on.
        self.current = listing.get(show.index).copied();
        if let Some(SlideshowAction::Exit) = action {
            self.slideshow = None;
            self.view = if back == ReturnTo::Photo { View::Photo } else { View::Library };
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
    }

    /// Shows a file created by Pico (e.g. a saved video frame) without a rescan.
    fn add_file(&mut self, path: PathBuf) {
        let Some(kind) = pico_core::media::MediaKind::of(&path) else { return };
        let Some(folder_index) = self.folders.iter().position(|f| Some(f.path.as_path()) == path.parent()) else {
            return;
        };
        let current = self.current.and_then(|at| self.file(at)).map(|f| f.path.clone());
        let folder = &mut self.folders[folder_index];
        if folder.files.iter().any(|f| f.path == path) {
            return;
        }
        self.status = tr!("Saved {path}", path = path.display());
        folder.files.push(MediaFile::new(path, kind));
        folder.files.sort_by(|a, b| library::natural_cmp(&a.name(), &b.name()));
        // Indices in this folder moved; keep pointing at the same photo.
        self.selection.retain(|&(f, _)| f != folder_index);
        self.current = current.and_then(|p| folder.files.iter().position(|f| f.path == p)).map(|i| (folder_index, i));
    }

    /// Moves the current photo by `delta` within the listing.
    fn step(&mut self, delta: isize) {
        let listing = self.listing();
        if delta == 0 || listing.is_empty() {
            return;
        }
        let position = self.current.and_then(|c| listing.iter().position(|&r| r == c));
        let next = match position {
            Some(p) => (p as isize + delta).clamp(0, listing.len() as isize - 1) as usize,
            None => 0,
        };
        self.current = Some(listing[next]);
        if self.view == View::Library {
            self.selection = BTreeSet::from([listing[next]]);
        }
    }

    fn click(&mut self, at: FileRef, toggle: bool, range: bool) {
        if range && let Some(anchor) = self.current {
            let listing = self.listing();
            let (a, b) = (listing.iter().position(|&r| r == anchor), listing.iter().position(|&r| r == at));
            if let (Some(a), Some(b)) = (a, b) {
                if !toggle {
                    self.selection.clear();
                }
                self.selection.extend(&listing[a.min(b)..=a.max(b)]);
            }
            return;
        }
        if toggle {
            if !self.selection.remove(&at) {
                self.selection.insert(at);
            }
        } else {
            self.selection = BTreeSet::from([at]);
        }
        self.current = Some(at);
    }

    fn select_all(&mut self) {
        self.selection = self.listing().into_iter().collect();
    }

    fn invert_selection(&mut self) {
        let all: BTreeSet<FileRef> = self.listing().into_iter().collect();
        self.selection = all.difference(&self.selection).copied().collect();
    }

    fn select_source(&mut self, source: Source) {
        self.ribbon_jump = Some(source.clone());
        self.source = Some(source);
        self.current = None;
        self.selection.clear();
        self.search.clear();
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button(tr!("File"), |ui| {
                if ui.button(tr!("Add Folder to Pico…")).clicked() {
                    self.open_folder_manager();
                }
                if ui.button(tr!("Rescan Folders")).clicked() {
                    self.rescan(ui.ctx());
                }
                ui.separator();
                let any = !self.targets().is_empty();
                ui.add_enabled_ui(any, |ui| self.file_menu(ui));
                ui.separator();
                if ui.button(tr!("Exit")).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button(tr!("Edit"), |ui| {
                if ui.button(tr!("Select All")).on_hover_text("Ctrl+A").clicked() {
                    self.select_all();
                }
                if ui.button(tr!("Invert Selection")).on_hover_text("Ctrl+I").clicked() {
                    self.invert_selection();
                }
                if ui.button(tr!("Clear Selection")).on_hover_text("Ctrl+D").clicked() {
                    self.selection.clear();
                }
                ui.separator();
                let copy = ui.add_enabled(!self.targets().is_empty(), egui::Button::new(tr!("Copy")));
                if copy.on_hover_text(tr!("Ctrl+C: copy the files, to paste into a chat or a folder")).clicked() {
                    self.copy_files();
                }
                if ui.add_enabled(self.current.is_some(), egui::Button::new(tr!("Copy All Effects"))).clicked() {
                    self.copy_effects();
                }
                if ui.add_enabled(self.copied_filters.is_some(), egui::Button::new(tr!("Paste All Effects"))).clicked()
                {
                    self.paste_effects();
                }
            });
            ui.menu_button(tr!("View"), |ui| {
                if ui.button(tr!("Slideshow")).on_hover_text("Ctrl+4").clicked() {
                    self.start_slideshow(ui.ctx());
                }
                ui.add(egui::Slider::new(&mut self.thumb_size, MIN_THUMB..=MAX_THUMB).text(tr!("Thumbnail size")));
                ui.menu_button(tr!("Thumbnail Captions"), |ui| {
                    for (mode, label) in CaptionMode::ALL {
                        ui.radio_value(&mut self.captions, mode, crate::i18n::tr(label));
                    }
                });
                ui.menu_button(tr!("Sort"), |ui| {
                    ui.radio_value(&mut self.sort_by, SortBy::Name, tr!("By Name"));
                    ui.radio_value(&mut self.sort_by, SortBy::Date, tr!("By Date Taken"));
                    ui.radio_value(&mut self.sort_by, SortBy::Size, tr!("By Size"));
                    let in_album = matches!(self.source, Some(Source::Album(_)));
                    ui.add_enabled_ui(in_album, |ui| {
                        ui.radio_value(&mut self.sort_by, SortBy::Custom, tr!("Custom Order"))
                            .on_hover_text(tr!("The album's own order: drag photos to arrange them"))
                            .on_disabled_hover_text(tr!("Albums only"));
                    });
                    ui.separator();
                    ui.checkbox(&mut self.reverse, tr!("Reverse Order"));
                });
                ui.checkbox(&mut self.starred_only, tr!("Only Starred Photos"));
                ui.checkbox(&mut self.videos_only, tr!("Only Videos"));
                ui.checkbox(&mut self.show_info, tr!("Properties and Tags")).on_hover_text("Alt+Enter, Ctrl+K");
                ui.menu_button(tr!("Folder List"), |ui| self.folder_list_options(ui));
                ui.checkbox(&mut self.show_filmstrip, tr!("Filmstrip"))
                    .on_hover_text(tr!("Neighbouring photos above the open one"));
                ui.checkbox(&mut self.show_hidden, tr!("Hidden Pictures"));
            });
            ui.menu_button(tr!("Picture"), |ui| {
                let any = !self.targets().is_empty();
                ui.add_enabled_ui(any, |ui| self.picture_menu(ui));
            });
            ui.menu_button(tr!("Create"), |ui| {
                let photos = self.targets().len();
                if ui
                    .add_enabled(photos > 0, egui::Button::new(tr!("Movie…")))
                    .on_hover_text(tr!("From the selected photos"))
                    .clicked()
                {
                    self.open_movie_dialog();
                }
                if ui
                    .button(tr!("Picture Collage…"))
                    .on_hover_text(tr!("From the selected photos, or all shown"))
                    .clicked()
                {
                    self.open_collage(ui.ctx());
                }
            });
            ui.menu_button(tr!("Tools"), |ui| {
                if ui.button(tr!("Show Duplicate Files")).clicked() {
                    self.find_duplicates(ui.ctx());
                }
                if ui.button(tr!("Folder Manager…")).clicked() {
                    self.open_folder_manager();
                }
                if ui.button(tr!("Options…")).clicked() {
                    self.show_options = true;
                }
            });
            ui.menu_button(tr!("Help"), |ui| {
                if ui.button(tr!("Keyboard Shortcuts")).on_hover_text("F1").clicked() {
                    self.show_shortcuts = true;
                }
                if ui.button(tr!("About Pico")).clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(tr!("Export Picture to Folder…")).on_hover_text("Ctrl+Shift+S").clicked() {
            self.open_export();
        }
        if ui.button(tr!("Save")).on_hover_text(tr!("Write the edits into the file; the original is kept")).clicked() {
            self.save_targets();
        }
        if ui.button(tr!("Save a Copy")).clicked() {
            self.save_copies();
        }
        if ui.button(tr!("Revert")).on_hover_text(tr!("Bring back the original saved by Save")).clicked() {
            self.revert_targets();
        }
        ui.separator();
        if ui.button(tr!("Rename…")).on_hover_text("F2").clicked() {
            self.start_rename();
        }
        if ui.button(tr!("Locate on Disk")).on_hover_text("Ctrl+Enter").clicked() {
            self.locate_on_disk();
        }
        if ui.button(tr!("Open in Editor")).on_hover_text("Ctrl+Shift+O").clicked() {
            self.open_in_editor();
        }
        // In an album, Delete takes photos out of the album; the files stay.
        let label = if matches!(self.source, Some(Source::Album(_))) {
            tr!("Remove from Album")
        } else {
            tr!("Move to Trash…")
        };
        if ui.button(label).on_hover_text("Delete").clicked() {
            self.delete_targets();
        }
    }

    fn open_export(&mut self) {
        let name = match &self.source {
            Some(Source::Folder(i)) => self.folders.get(*i).map(|f| f.name.clone()),
            Some(Source::Starred) => Some("Starred".to_owned()),
            Some(Source::Album(token)) => self.album_name(token),
            Some(Source::Duplicates) => Some("Duplicates".to_owned()),
            Some(Source::Tray) => Some("Photo Tray".to_owned()),
            Some(Source::Month(key)) => Some(library::month_label(key)),
            Some(Source::Place(key)) => Some(format!("Place {key}")),
            None => None,
        };
        let base = config::default_pictures_dir().unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("Pico Exports").join(name.unwrap_or_else(|| "Export".into()));
        self.export = Some(ExportDialog { dir: dir.display().to_string(), size: 0, custom: 1200, quality: 85 });
    }

    fn start_export(&mut self, ctx: &egui::Context, dialog: &ExportDialog) {
        let files: Vec<MediaFile> = self.targets().into_iter().filter_map(|at| self.file(at).cloned()).collect();
        let dir = PathBuf::from(dialog.dir.trim());
        let options = ExportOptions { max_side: dialog.max_side(), quality: dialog.quality };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let total = files.len();
            let mut failed = 0;
            for (done, file) in files.iter().enumerate() {
                if let Err(e) = export::export(file, &dir, options) {
                    failed += 1;
                    let _ = tx.send(ExportProgress::Failed(format!("{}: {e}", file.name())));
                }
                let _ = tx.send(ExportProgress::Done { done: done + 1, total, failed, dir: dir.clone() });
                ctx.request_repaint();
            }
        });
        self.export_job = Some(rx);
        self.export_count = (0, self.targets().len());
    }

    fn poll_export(&mut self) {
        let Some(rx) = &self.export_job else { return };
        while let Ok(progress) = rx.try_recv() {
            match progress {
                ExportProgress::Failed(message) => self.status = message,
                ExportProgress::Done { done, total, failed, dir } => {
                    self.export_count = (done, total);
                    self.status = if done < total {
                        tr!("Exporting {done} of {total}…", done = done, total = total)
                    } else if failed == 0 {
                        tr!(
                            "Exported {pictures} to {folder}",
                            pictures = plural(total, "picture"),
                            folder = dir.display()
                        )
                    } else {
                        tr!(
                            "Exported {done} of {total}; {failed} failed",
                            done = total - failed,
                            total = total,
                            failed = failed
                        )
                    };
                    if done == total {
                        self.export_job = None;
                        return;
                    }
                }
            }
        }
    }

    /// Runs a file operation on every target, reporting the first failure
    /// with `failed`, a text with `{path}` and `{e}` in it.
    fn for_each_target(&mut self, failed: &str, op: impl Fn(&mut Folder, usize) -> std::io::Result<()>) -> usize {
        let mut done = 0;
        for (fi, i) in self.targets() {
            let Some(folder) = self.folders.get_mut(fi) else { continue };
            let path = folder.files[i].path.clone();
            match op(folder, i) {
                Ok(()) => {
                    done += 1;
                    self.albums_dirty = true;
                    self.thumbs.invalidate(&path);
                }
                Err(e) => {
                    self.status = crate::i18n::fill(crate::i18n::tr(failed), &[("path", &path.display()), ("e", &e)]);
                    return done;
                }
            }
        }
        done
    }

    fn save_targets(&mut self) {
        let done = self.for_each_target(tr_mark!("Could not save {path}: {e}"), export::save_in_place);
        if done > 0 {
            self.status = format!(
                "Saved {}. Originals are in .picasaoriginals; Revert brings them back.",
                plural(done, "picture")
            );
        }
    }

    fn save_copies(&mut self) {
        let mut added = Vec::new();
        for at in self.targets() {
            let Some(file) = self.file(at) else { continue };
            match export::save_copy(file) {
                Ok(path) => added.push(path),
                Err(e) => self.status = tr!("Could not save a copy of {file}: {e}", file = file.name(), e = e),
            }
        }
        for path in added {
            self.add_file(path);
        }
    }

    fn revert_targets(&mut self) {
        let done = self.for_each_target(tr_mark!("Could not revert {path}: {e}"), |folder, i| {
            if export::has_backup(folder, i) { export::revert(folder, i) } else { Ok(()) }
        });
        if done > 0 {
            self.status = tr!("Reverted. The edits are back in the edit stack, where Undo can reach them.").into();
        }
    }

    fn start_rename(&mut self) {
        if let Some(at) = self.current
            && let Some(file) = self.file(at)
        {
            self.rename = Some((at, file.name()));
        }
    }

    fn locate_on_disk(&mut self) {
        if let Some(file) = self.current.and_then(|at| self.file(at))
            && let Err(e) = fileops::reveal(&file.path)
        {
            self.status = tr!("Could not open the file manager: {e}", e = e);
        }
    }

    fn open_in_editor(&mut self) {
        if let Some(file) = self.current.and_then(|at| self.file(at))
            && let Err(e) = fileops::open_external(&file.path)
        {
            self.status = tr!("Could not open {file}: {e}", file = file.name(), e = e);
        }
    }

    fn trash(&mut self, files: Vec<FileRef>) {
        // Highest indices first, so earlier ones stay valid.
        let mut files = files;
        files.sort_unstable_by(|a, b| b.cmp(a));
        let mut moved = 0;
        for (fi, i) in files {
            let Some(folder) = self.folders.get_mut(fi) else { continue };
            match fileops::trash(folder, i) {
                Ok(_) => {
                    moved += 1;
                    self.albums_dirty = true;
                }
                Err(e) => {
                    self.status = tr!("Could not move to the trash: {e}", e = e);
                    break;
                }
            }
        }
        self.selection.clear();
        self.current = None;
        if self.view == View::Photo {
            self.view = View::Library;
        }
        if moved > 0 {
            self.status = tr!("Moved {files} to the trash. It's not gone gone.", files = plural(moved, "file"));
        }
    }

    /// Right-click on a thumbnail; works on the selection, which holds `at`.
    fn photo_context_menu(&mut self, ui: &mut egui::Ui, at: FileRef) {
        let many = self.selection.len() > 1;
        if !many && ui.button(tr!("View and Edit")).clicked() {
            self.current = Some(at);
            self.view = View::Photo;
        }
        let star = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.star));
        if ui.button(if star { tr!("Star") } else { tr!("Unstar") }).on_hover_text("S").clicked() {
            self.edit_targets(|meta| meta.star = star);
        }
        if ui.button(tr!("Rotate Clockwise")).on_hover_text("Ctrl+R").clicked() {
            self.rotate_targets(1);
        }
        if ui.button(tr!("Rotate Counterclockwise")).on_hover_text("Ctrl+Shift+R").clicked() {
            self.rotate_targets(-1);
        }
        self.album_menu(ui);
        if ui.button(tr!("Hold in Tray")).on_hover_text("Ctrl+H").clicked() {
            self.hold();
        }
        let hide = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.hidden));
        if ui.button(if hide { tr!("Hide") } else { tr!("Unhide") }).clicked() {
            self.edit_targets(|meta| meta.hidden = hide);
        }
        ui.separator();
        if ui.button(tr!("Export to Folder…")).on_hover_text("Ctrl+Shift+S").clicked() {
            self.open_export();
        }
        if !many {
            if ui.button(tr!("Locate on Disk")).on_hover_text("Ctrl+Enter").clicked() {
                self.locate_on_disk();
            }
            if ui.button(tr!("Open in Editor")).on_hover_text("Ctrl+Shift+O").clicked() {
                self.open_in_editor();
            }
            if ui.button(tr!("Rename…")).on_hover_text("F2").clicked() {
                self.start_rename();
            }
            if ui.button(tr!("Copy Full Path")).clicked()
                && let Some(file) = self.file(at)
            {
                ui.ctx().copy_text(file.path.display().to_string());
            }
        }
        // In an album, Delete takes photos out of the album; the files stay.
        let label = if matches!(self.source, Some(Source::Album(_))) {
            tr!("Remove from Album")
        } else {
            tr!("Move to Trash…")
        };
        if ui.button(label).on_hover_text("Delete").clicked() {
            self.delete_targets();
        }
    }

    /// Right-click on a folder in the list.
    fn folder_context_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        let Some(path) = self.folders.get(index).map(|f| f.path.clone()) else { return };
        if ui.button(tr!("View Slideshow")).clicked() {
            self.select_source(Source::Folder(index));
            self.start_slideshow(ui.ctx());
        }
        if ui.button(tr!("Locate on Disk")).clicked()
            && let Err(e) = fileops::open_external(&path)
        {
            self.status = tr!("Could not open the file manager: {e}", e = e);
        }
        if ui.button(tr!("Copy Full Path")).clicked() {
            ui.ctx().copy_text(path.display().to_string());
        }
        ui.separator();
        if ui.button(tr!("Rescan Folders")).clicked() {
            self.rescan(ui.ctx());
        }
        if ui
            .button(tr!("Remove from Pico"))
            .on_hover_text(tr!("Leave this folder out; no files are touched"))
            .clicked()
        {
            let mut rules = self.rules.clone();
            rules.set_in_library(&path, false);
            let name = Folder::file_name(&path);
            self.set_rules(ui.ctx(), rules);
            self.status = tr!("Left out {name}. Tools > Folder Manager brings it back.", name = name);
        }
    }

    fn album_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tr!("Add to Album"), |ui| {
            for (token, name, _) in self.album_list() {
                if ui.button(&name).clicked() {
                    self.add_to_album(&token, &name);
                }
            }
            if ui.button(tr!("New Album…")).on_hover_text("Ctrl+N").clicked() {
                self.new_album = Some(String::new());
            }
        });
    }

    fn picture_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button(tr!("Star / Unstar")).on_hover_text("S").clicked() {
            let star = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.star));
            self.edit_targets(|meta| meta.star = star);
        }
        if ui.button(tr!("Rotate Clockwise")).on_hover_text("Ctrl+R").clicked() {
            self.rotate_targets(1);
        }
        if ui.button(tr!("Rotate Counterclockwise")).on_hover_text("Ctrl+Shift+R").clicked() {
            self.rotate_targets(-1);
        }
        ui.separator();
        for (label, id, keys) in [
            (tr!("I'm Feeling Lucky"), "enhance", "Ctrl+Shift+E"),
            (tr!("Auto Contrast"), "autolight", ""),
            (tr!("Auto Color"), "autocolor", ""),
            (tr!("Black-and-White"), "bw", "Ctrl+Shift+B"),
            (tr!("Warmify"), "warm", ""),
            (tr!("Film Grain"), "grain2", ""),
            (tr!("Sharpen"), "unsharp2", ""),
        ] {
            if ui.button(label).on_hover_text(keys).clicked() {
                self.add_effect(id);
            }
        }
        ui.separator();
        if ui.button(tr!("Hold in Tray")).on_hover_text("Ctrl+H").clicked() {
            self.hold();
        }
        self.album_menu(ui);
        if matches!(self.source, Some(Source::Album(_)))
            && ui.button(tr!("Remove from Album")).on_hover_text("Delete").clicked()
        {
            self.delete_targets();
        }
        ui.separator();
        if ui.button(tr!("Undo All Edits")).clicked() {
            self.edit_targets(|meta| {
                meta.filters = FilterStack::default();
                meta.rotate = 0;
            });
        }
        let hide = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.hidden));
        if ui.button(if hide { tr!("Hide") } else { tr!("Unhide") }).clicked() {
            self.edit_targets(|meta| meta.hidden = hide);
        }
    }

    fn folder_list(&mut self, ui: &mut egui::Ui) {
        ui.heading(tr!("Library"));
        ui.separator();
        let starred = self.folders.iter().flat_map(|f| &f.files).filter(|f| f.meta.star).count();
        if ui
            .selectable_label(self.source == Some(Source::Starred), format!("★ {}  ({starred})", tr!("Starred Photos")))
            .clicked()
        {
            self.select_source(Source::Starred);
        }
        if !self.tray.is_empty()
            && ui
                .selectable_label(
                    self.source == Some(Source::Tray),
                    format!("{}  ({})", tr!("Photo Tray"), self.tray.len()),
                )
                .clicked()
        {
            self.select_source(Source::Tray);
        }
        let albums = self.album_list();
        let mut picked_album = None;
        if !albums.is_empty() {
            let open = self.section_open("albums", true);
            let section = egui::CollapsingHeader::new(egui::RichText::new(tr!("Albums")).strong())
                .id_salt("albums")
                .open(Some(open))
                .show(ui, |ui| {
                    for (token, name, count) in &albums {
                        let open = self.source == Some(Source::Album(token.clone()));
                        let response = ui.selectable_label(open, format!("{name}  ({count})"));
                        if response.clicked() {
                            picked_album = Some(token.clone());
                        }
                        if let Some(drag) = drop_target(ui, &response) {
                            self.selection = drag.0.iter().copied().collect();
                            self.add_to_album(token, name);
                        }
                        response.context_menu(|ui| {
                            if ui.button(tr!("Rename Album…")).clicked() {
                                self.album_rename = Some((token.clone(), name.clone()));
                            }
                            if ui.button(tr!("Delete Album…")).clicked() {
                                self.confirm_album_delete = Some(token.clone());
                            }
                        });
                    }
                });
            self.section_clicked(&section.header_response, "albums", open);
        }
        if let Some(token) = picked_album {
            self.select_source(Source::Album(token));
        }
        let mut picked_source = None;
        let months = library::months(&self.folders);
        if !months.is_empty() {
            let open = self.section_open("timeline", false);
            let section =
                egui::CollapsingHeader::new(tr!("Timeline")).id_salt("timeline").open(Some(open)).show(ui, |ui| {
                    let mut years: Vec<(&str, Vec<&(String, usize)>)> = Vec::new();
                    for entry in &months {
                        match years.last_mut() {
                            Some((year, list)) if *year == &entry.0[..4] => list.push(entry),
                            _ => years.push((&entry.0[..4], vec![entry])),
                        }
                    }
                    for (year, list) in years {
                        let total: usize = list.iter().map(|(_, n)| n).sum();
                        let key = format!("timeline/{year}");
                        let year_open = self.section_open(&key, false);
                        let header = egui::CollapsingHeader::new(format!("{year}  ({total})")).id_salt(year);
                        let year_section = header.open(Some(year_open)).show(ui, |ui| {
                            for (key, count) in list {
                                let open = self.source == Some(Source::Month(key.clone()));
                                let label = format!("{}  ({count})", crate::i18n::month_label(key));
                                if ui.selectable_label(open, label).clicked() {
                                    picked_source = Some(Source::Month(key.clone()));
                                }
                            }
                        });
                        self.section_clicked(&year_section.header_response, &key, year_open);
                    }
                });
            self.section_clicked(&section.header_response, "timeline", open);
        }
        let places = library::places(&self.folders);
        if !places.is_empty() {
            let open = self.section_open("places", false);
            let section =
                egui::CollapsingHeader::new(tr!("Places")).id_salt("places").open(Some(open)).show(ui, |ui| {
                    for place in &places {
                        let open = self.source == Some(Source::Place(place.key.clone()));
                        let label = format!("{}  ({})", place.label(), place.members.len());
                        let response = ui.selectable_label(open, label);
                        if response.on_hover_text(library::place_coordinates(place.center)).clicked() {
                            picked_source = Some(Source::Place(place.key.clone()));
                        }
                    }
                });
            self.section_clicked(&section.header_response, "places", open);
        }
        if let Some(source) = picked_source {
            self.select_source(source);
        }
        ui.separator();
        let id = ui.make_persistent_id("folders");
        self.restore_state(ui.ctx(), id, "folders");
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                ui.strong(tr!("Folders"));
                ui.menu_button("…", |ui| self.folder_list_options(ui))
                    .response
                    .on_hover_text(tr!("Tree or list, and order"));
            })
            .body(|ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut rows = Vec::new();
                    if self.folder_tree {
                        for node in self.folder_nodes() {
                            self.folder_node(ui, &node, "tree", &mut rows);
                        }
                    } else if self.folder_sort == FolderSort::Date {
                        let order = self.folder_order();
                        let mut years: Vec<(String, Vec<usize>)> = Vec::new();
                        for index in order {
                            let year = self.folder_first_date(index).get(..4).unwrap_or("?").to_owned();
                            match years.last_mut() {
                                Some((y, list)) if *y == year => list.push(index),
                                _ => years.push((year, vec![index])),
                            }
                        }
                        for (year, list) in years {
                            let title = format!("{year}  ({})", plural(list.len(), "folder"));
                            let key = format!("folders/{year}");
                            let year_open = self.section_open(&key, true);
                            let year_section = egui::CollapsingHeader::new(title)
                                .id_salt(("year", &year))
                                .open(Some(year_open))
                                .show(ui, |ui| {
                                    for index in list {
                                        rows.push((index, self.folder_row(ui, index)));
                                    }
                                });
                            self.section_clicked(&year_section.header_response, &key, year_open);
                        }
                    } else {
                        for index in self.folder_order() {
                            rows.push((index, self.folder_row(ui, index)));
                        }
                    }
                    self.folder_rows_done(ui, rows);
                });
            });
        self.note_section(ui.ctx(), id, "folders");
    }

    /// Whether the section `key` is open: as last left, else `default`.
    fn section_open(&self, key: &str, default: bool) -> bool {
        self.sections.borrow().get(key).copied().unwrap_or(default)
    }

    /// Opens or closes the section `key` when its header was clicked.
    fn section_clicked(&self, header: &egui::Response, key: &str, was_open: bool) {
        if header.clicked() {
            self.sections.borrow_mut().insert(key.to_owned(), !was_open);
        }
    }

    /// Puts back the remembered state of the section with this egui `id`.
    fn restore_state(&self, ctx: &egui::Context, id: egui::Id, key: &str) {
        if !self.sections_restored.borrow_mut().insert(key.to_owned()) {
            return;
        }
        if let Some(&open) = self.sections.borrow().get(key) {
            let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ctx, id, open);
            state.set_open(open);
            state.store(ctx);
        }
    }

    /// Remembers whether the section with this egui `id` is open now.
    fn note_section(&self, ctx: &egui::Context, id: egui::Id, key: &str) {
        if let Some(state) = egui::collapsing_header::CollapsingState::load(ctx, id) {
            self.sections.borrow_mut().insert(key.to_owned(), state.is_open());
        }
    }

    /// One folder in the list: name and count, a drop target for photos.
    fn folder_row(&self, ui: &mut egui::Ui, index: usize) -> egui::Response {
        let folder = &self.folders[index];
        let label = format!("{}  ({})", folder.name, folder.files.len());
        ui.selectable_label(self.source == Some(Source::Folder(index)), label)
            .on_hover_text(folder.path.display().to_string())
    }

    /// Clicks, drops and context menus on the folder rows drawn this frame.
    fn folder_rows_done(&mut self, ui: &egui::Ui, rows: Vec<(usize, egui::Response)>) {
        for (index, response) in rows {
            if response.clicked() {
                self.select_source(Source::Folder(index));
            }
            if let Some(drag) = drop_target(ui, &response) {
                let elsewhere: Vec<FileRef> = drag.0.iter().copied().filter(|at| at.0 != index).collect();
                if !elsewhere.is_empty() {
                    self.confirm_move = Some((elsewhere, index));
                }
            }
            response.context_menu(|ui| self.folder_context_menu(ui, index));
        }
    }

    /// A folder of the tree and, open or closed as last left, what is in it;
    /// `parent` names the branch it hangs from, for remembering that.
    fn folder_node(&self, ui: &mut egui::Ui, node: &FolderNode, parent: &str, rows: &mut Vec<(usize, egui::Response)>) {
        let mut row = |ui: &mut egui::Ui| match node.folder {
            Some(index) => rows.push((index, self.folder_row(ui, index))),
            None => {
                ui.label(egui::RichText::new(&node.name).weak());
            }
        };
        if node.children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                row(ui);
            });
            return;
        }
        let id = ui.make_persistent_id(("folder tree", &node.name, node.folder));
        let key = format!("{parent}/{}", node.name);
        self.restore_state(ui.ctx(), id, &key);
        let header = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| row(ui));
        header.body(|ui| {
            for child in &node.children {
                self.folder_node(ui, child, &key, rows);
            }
        });
        self.note_section(ui.ctx(), id, &key);
    }

    fn folder_list_options(&mut self, ui: &mut egui::Ui) {
        ui.radio_value(&mut self.folder_tree, false, tr!("Flat List"));
        ui.radio_value(&mut self.folder_tree, true, tr!("Tree")).on_hover_text(tr!("Folders where they are on disk"));
        ui.separator();
        ui.add_enabled_ui(!self.folder_tree, |ui| {
            ui.radio_value(&mut self.folder_sort, FolderSort::Name, tr!("Sort by Name"));
            ui.radio_value(&mut self.folder_sort, FolderSort::Date, tr!("Sort by Date"))
                .on_hover_text(tr!("Newest first, by year"));
            ui.radio_value(&mut self.folder_sort, FolderSort::Recent, tr!("Sort by Recent Changes"));
            ui.radio_value(&mut self.folder_sort, FolderSort::Size, tr!("Sort by Size"));
        });
    }

    /// Folder indices in list order (the ribbon follows it too).
    fn folder_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.folders.len()).collect();
        if self.folder_tree {
            return order;
        }
        match self.folder_sort {
            FolderSort::Name => {}
            FolderSort::Date => order.sort_by_cached_key(|&i| std::cmp::Reverse(self.folder_first_date(i))),
            FolderSort::Recent => order.sort_by_cached_key(|&i| {
                std::cmp::Reverse(self.folders[i].files.iter().filter_map(|f| f.modified).max())
            }),
            FolderSort::Size => order
                .sort_by_cached_key(|&i| std::cmp::Reverse(self.folders[i].files.iter().map(|f| f.size).sum::<u64>())),
        }
        order
    }

    /// The folders, albums, months or places in the list around the source,
    /// in list order, each with its title and photos, for the ribbon; none
    /// while searching or for a source of its own (Starred, Duplicates).
    fn ribbon(&self) -> Option<Vec<(Source, String, Vec<FileRef>)>> {
        if !self.search.trim().is_empty() {
            return None;
        }
        Some(match self.source.as_ref()? {
            Source::Folder(_) => self
                .folder_order()
                .into_iter()
                .map(|fi| {
                    let source = Source::Folder(fi);
                    let refs = self.listing_of(Some(&source));
                    (source, self.folders[fi].name.clone(), refs)
                })
                .collect(),
            Source::Album(_) => self
                .album_list()
                .into_iter()
                .map(|(token, name, _)| {
                    let source = Source::Album(token);
                    let refs = self.listing_of(Some(&source));
                    (source, name, refs)
                })
                .collect(),
            // Months and places are sorted out of one listing, rather than
            // each going through the whole library.
            Source::Month(_) => {
                let months = library::months(&self.folders);
                let index: HashMap<&str, usize> = months.iter().enumerate().map(|(i, m)| (m.0.as_str(), i)).collect();
                let mut refs = vec![Vec::new(); months.len()];
                for at in self.dated_listing() {
                    if let Some(&i) = self.file(at).and_then(|f| index.get(&f.sort_date()[..7])) {
                        refs[i].push(at);
                    }
                }
                months
                    .iter()
                    .zip(refs)
                    .map(|((key, _), refs)| (Source::Month(key.clone()), crate::i18n::month_label(key), refs))
                    .collect()
            }
            Source::Place(_) => {
                let places = library::places(&self.folders);
                let index: HashMap<FileRef, usize> =
                    places.iter().enumerate().flat_map(|(i, p)| p.members.iter().map(move |&at| (at, i))).collect();
                let mut refs = vec![Vec::new(); places.len()];
                for at in self.dated_listing() {
                    if let Some(&i) = index.get(&at) {
                        refs[i].push(at);
                    }
                }
                places.iter().zip(refs).map(|(p, refs)| (Source::Place(p.key.clone()), p.label(), refs)).collect()
            }
            _ => return None,
        })
    }

    /// Every photo, filtered and sorted as Timeline and Places list theirs
    /// (every date starts with the empty month).
    fn dated_listing(&self) -> Vec<FileRef> {
        self.listing_of(Some(&Source::Month(String::new())))
    }

    /// When the folder's first photo was taken (or its file date).
    fn folder_first_date(&self, index: usize) -> String {
        self.folders[index].files.iter().map(|f| f.sort_date()).min().unwrap_or_default()
    }

    /// The folders as a tree under each watched folder.
    fn folder_nodes(&self) -> Vec<FolderNode> {
        let mut roots: Vec<FolderNode> = Vec::new();
        let mut watched = self.rules.watched.clone();
        watched.sort();
        for (index, folder) in self.folders.iter().enumerate() {
            // The closest watched folder holds it.
            let Some(root) =
                watched.iter().filter(|w| folder.path.starts_with(w)).max_by_key(|w| w.components().count())
            else {
                continue;
            };
            let root_name = Folder::file_name(root);
            let top = match roots.iter().position(|n| n.name == root_name) {
                Some(i) => i,
                None => {
                    roots.push(FolderNode { name: root_name, folder: None, children: Vec::new() });
                    roots.len() - 1
                }
            };
            let mut node = &mut roots[top];
            for part in folder.path.strip_prefix(root).unwrap_or(Path::new("")).components() {
                let name = part.as_os_str().to_string_lossy().into_owned();
                let i = match node.children.iter().position(|c| c.name == name) {
                    Some(i) => i,
                    None => {
                        node.children.push(FolderNode { name, folder: None, children: Vec::new() });
                        node.children.len() - 1
                    }
                };
                node = &mut node.children[i];
            }
            node.folder = Some(index);
        }
        roots
    }

    fn library(&mut self, ui: &mut egui::Ui) {
        self.hovered = None;
        let sidebar = egui::Panel::left("folders").resizable(true).default_size(self.sidebar_width).show(ui, |ui| {
            // A list, not buttons: rows as compact as egui makes them.
            ui.spacing_mut().button_padding = egui::Style::default().spacing.button_padding;
            self.folder_list(ui)
        });
        self.sidebar_width = sidebar.response.rect.width().round();
        let tray = egui::Panel::bottom("tray").show(ui, |ui| self.tray_panel(ui)).response;
        if let Some(drag) = drop_target(ui, &tray) {
            self.selection = drag.0.iter().copied().collect();
            self.hold();
        }
        egui::CentralPanel::default_margins().show(ui, |ui| {
            if self.folders.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.scan.is_some() {
                        tr!("Counting pixels…")
                    } else {
                        tr!("No photos yet. Add a folder from the File menu — Pico promises not to upload anything.")
                    });
                });
                return;
            }
            self.grid_header(ui);
            // A folder shows in the ribbon of all folders, as in Picasa; so do
            // an album, a month and a place among theirs.
            let ribbon = self.ribbon();
            // Album photos Pico cannot show get black tiles.
            let missing_of = |source: &Source, refs: &[FileRef]| match source {
                Source::Album(token) => self.album_missing(token, refs).into_iter().map(|(tile, _)| tile).collect(),
                _ => Vec::new(),
            };
            // An album's photos can be dragged into its own order.
            let albums = ribbon.is_some() && matches!(self.source, Some(Source::Album(_)));
            let parts = match ribbon {
                Some(ribbon) => ribbon
                    .into_iter()
                    .map(|(source, title, refs)| {
                        let missing = missing_of(&source, &refs);
                        (Some((source, title)), refs, missing)
                    })
                    .filter(|(_, refs, missing)| !refs.is_empty() || !missing.is_empty())
                    .collect(),
                None => vec![(None, self.listing(), Vec::new())],
            };
            if parts.iter().all(|(_, refs, missing)| refs.is_empty() && missing.is_empty()) {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.search.trim().is_empty() {
                        tr!("Nothing to show here.")
                    } else {
                        tr!("No matches. Maybe it was a dream?")
                    })
                });
                return;
            }
            // Ctrl+wheel or a pinch over the grid resizes the thumbnails.
            let zoom = ui.input(|i| i.zoom_delta());
            if zoom != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
                self.thumb_size = (self.thumb_size * zoom).clamp(MIN_THUMB, MAX_THUMB);
            }
            let folders = &self.folders;
            let mut sources = Vec::new();
            let mut sections = Vec::new();
            for (section, refs, missing) in parts {
                let items: Vec<(FileRef, &MediaFile)> =
                    refs.iter().filter_map(|&at| Some((at, folders.get(at.0)?.files.get(at.1)?))).collect();
                let header = section.as_ref().map(|(source, title)| {
                    // A folder's Picasa description goes under its title.
                    let note = match source {
                        Source::Folder(fi) => folders[*fi].ini.get("Picasa", "description"),
                        _ => None,
                    };
                    let note = note.filter(|d| !d.is_empty()).map(str::to_owned);
                    section_header(title.clone(), note, &items, missing.len())
                });
                sections.push(GridSection { header, items, missing });
                sources.push(section.map(|(source, _)| source));
            }
            let jump = self.ribbon_jump.take().and_then(|s| sources.iter().position(|p| p.as_ref() == Some(&s)));
            let home = sources.iter().position(|p| p.is_some() && *p == self.source);
            let reveal = (self.current != self.revealed).then_some(self.current).flatten();
            self.revealed = self.current;
            let view = GridView {
                sections: &sections,
                thumb_size: self.thumb_size,
                selection: &self.selection,
                current: self.current,
                captions: self.captions,
                scroll_to_section: jump,
                reveal,
                home,
                reorder: albums,
            };
            let output = grid::show(ui, view, &mut self.thumbs);
            self.hovered = output.hovered;
            if let Some(folder) = output.add_folder {
                self.status = tr!("Adding {folder}", folder = folder.display());
                self.watch_folder(ui.ctx(), folder);
            }
            // Scrolling by hand moves the highlight in the list along. A scroll the
            // grid made itself shows in the offset a frame later; skip those.
            let scrolled = (output.offset - self.ribbon_offset).abs() > 0.5;
            self.ribbon_offset = output.offset;
            if output.programmatic {
                self.ribbon_settle = 2;
            }
            let settling = self.ribbon_settle > 0;
            self.ribbon_settle = self.ribbon_settle.saturating_sub(1);
            if scrolled
                && !settling
                && let Some(Some(source)) = output.top_section.and_then(|s| sources.get(s)).cloned()
            {
                self.source = Some(source);
            }
            let (action, cells) = (output.action, output.cells);
            // Working on a photo in another folder (album, month, place) makes
            // that one current, so the viewer and arrow keys stay in it.
            if let Some(Some(source)) = output.action_section.and_then(|s| sources.get(s)).cloned() {
                self.source = Some(source);
            }
            for (at, response) in cells {
                if response.secondary_clicked() && !self.selection.contains(&at) {
                    // Right-clicking outside the selection works on that photo.
                    self.selection = BTreeSet::from([at]);
                }
                if response.secondary_clicked() {
                    self.current = Some(at);
                }
                response.context_menu(|ui| self.photo_context_menu(ui, at));
            }
            match action {
                Some(GridAction::Click { at, toggle, range }) => self.click(at, toggle, range),
                Some(GridAction::Band { hits, toggle, done }) => {
                    // With Ctrl the band adds to what was selected before it.
                    let base = self
                        .band_base
                        .get_or_insert_with(|| if toggle { self.selection.clone() } else { BTreeSet::new() });
                    self.selection = base.iter().copied().chain(hits.iter().copied()).collect();
                    if let Some(&last) = hits.last() {
                        self.current = Some(last);
                    }
                    if done {
                        self.band_base = None;
                    }
                }
                Some(GridAction::Open(at)) => {
                    self.current = Some(at);
                    self.selection = BTreeSet::from([at]);
                    self.view = View::Photo;
                }
                Some(GridAction::Reorder { section, photos, missing, to }) => {
                    if let Some(Some(Source::Album(token))) = sources.get(section) {
                        self.move_in_album(token, &photos, &missing, to);
                    }
                }
                None => {}
            }
        });
        self.library_keys(ui);
    }

    fn grid_header(&mut self, ui: &mut egui::Ui) {
        let mut album_note = None;
        ui.horizontal(|ui| {
            let searching = !self.search.trim().is_empty();
            match &self.source {
                _ if searching => {
                    ui.heading(tr!("Search results"));
                }
                Some(Source::Starred) => {
                    ui.heading(tr!("Starred Photos"));
                }
                Some(Source::Tray) => {
                    ui.heading(tr!("Photo Tray"));
                    ui.label(egui::RichText::new(tr!("Held photos from any folder")).weak());
                }
                Some(Source::Album(token)) => {
                    ui.heading(self.album_name(token).unwrap_or_default());
                    ui.label(egui::RichText::new(tr!("Album")).weak());
                    album_note = self.album_missing_note(token);
                }
                Some(Source::Month(key)) => {
                    ui.heading(crate::i18n::month_label(key));
                    ui.label(egui::RichText::new(tr!("Timeline")).weak());
                }
                Some(Source::Place(key)) => {
                    let place = library::places(&self.folders).into_iter().find(|p| &p.key == key);
                    ui.heading(place.as_ref().map_or_else(|| key.clone(), library::Place::label));
                    let at = place.map(|p| format!(" · {}", library::place_coordinates(p.center)));
                    ui.label(egui::RichText::new(format!("{}{}", tr!("Place"), at.unwrap_or_default())).weak());
                }
                Some(Source::Duplicates) => {
                    ui.heading(tr!("Duplicate Files"));
                    let text = if self.duplicate_job.is_some() {
                        tr!("Comparing files…")
                    } else {
                        tr!("Identical files, side by side")
                    };
                    ui.label(egui::RichText::new(text).weak());
                }
                Some(Source::Folder(i)) => {
                    if let Some(folder) = self.folders.get(*i) {
                        ui.heading(&folder.name);
                        if let Some(description) = folder.ini.get("Picasa", "description").filter(|d| !d.is_empty()) {
                            ui.label(egui::RichText::new(description).weak());
                        }
                    }
                }
                None => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.search).hint_text(tr!("Search (Ctrl+F)")).desired_width(220.0),
                );
                if self.focus_search {
                    search.request_focus();
                    self.focus_search = false;
                }
                if search.lost_focus() {
                    // Enter confirms the search; it should not open a photo too.
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                }
                if search.changed() {
                    self.selection.clear();
                    self.current = None;
                }
                ui.toggle_value(&mut self.videos_only, "▶").on_hover_text(tr!("Only videos"));
                ui.toggle_value(&mut self.starred_only, "★").on_hover_text(tr!("Only starred photos"));
                let count = self.selection.len();
                if count > 1 {
                    ui.label(tr!("{n} selected", n = count));
                }
            });
        });
        if let Some(note) = album_note {
            ui.label(egui::RichText::new(note).color(ui.visuals().warn_fg_color));
        }
        ui.separator();
    }

    fn library_keys(&mut self, ui: &mut egui::Ui) {
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            self.focus_search = true;
        }
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let keys = ui.input(|i| {
            let cmd = i.modifiers.command;
            let shift = i.modifiers.shift;
            Keys {
                vertical: i.key_pressed(egui::Key::ArrowDown) as isize - i.key_pressed(egui::Key::ArrowUp) as isize,
                horizontal: i.key_pressed(egui::Key::ArrowRight) as isize
                    - i.key_pressed(egui::Key::ArrowLeft) as isize,
                // Ctrl+Enter locates on disk and Alt+Enter toggles Info.
                enter: i.key_pressed(egui::Key::Enter) && !cmd && !i.modifiers.alt,
                rotate: (cmd && i.key_pressed(egui::Key::R)).then_some(if shift { -1 } else { 1 }),
                select_all: cmd && i.key_pressed(egui::Key::A),
                invert: cmd && i.key_pressed(egui::Key::I),
                clear: cmd && i.key_pressed(egui::Key::D),
                lucky: cmd && shift && i.key_pressed(egui::Key::E),
                bw: cmd && shift && i.key_pressed(egui::Key::B),
                star: !cmd && i.key_pressed(egui::Key::S),
                small: cmd && i.key_pressed(egui::Key::Num1),
                normal: cmd && i.key_pressed(egui::Key::Num2),
                rename: i.key_pressed(egui::Key::F2),
                trash: i.key_pressed(egui::Key::Delete),
                locate: cmd && i.key_pressed(egui::Key::Enter),
                open_editor: cmd && shift && i.key_pressed(egui::Key::O),
                export: cmd && shift && i.key_pressed(egui::Key::S),
                new_album: cmd && i.key_pressed(egui::Key::N),
                hold: cmd && i.key_pressed(egui::Key::H),
                page: i.key_pressed(egui::Key::PageDown) as isize - i.key_pressed(egui::Key::PageUp) as isize,
                edge: i.key_pressed(egui::Key::End) as isize - i.key_pressed(egui::Key::Home) as isize,
                open: cmd && i.key_pressed(egui::Key::Num3),
            }
        });
        if keys.hold {
            self.hold();
        }
        if keys.new_album && !self.targets().is_empty() {
            self.new_album = Some(String::new());
        }
        if keys.rename {
            self.start_rename();
        }
        if keys.trash && !self.targets().is_empty() {
            self.delete_targets();
        }
        if keys.locate {
            self.locate_on_disk();
            return;
        }
        if keys.open_editor {
            self.open_in_editor();
        }
        if keys.export && !self.targets().is_empty() {
            self.open_export();
        }
        let width = ui.available_width().max(self.thumb_size);
        let columns = grid::columns_for(width, self.thumb_size, ui.spacing().item_spacing.x) as isize;
        // A page is about the rows that fit on screen.
        let rows = (ui.ctx().content_rect().height() * 0.6 / (self.thumb_size + 16.0)).floor().max(1.0) as isize;
        self.step(keys.vertical * columns + keys.horizontal + keys.page * rows * columns);
        if keys.edge != 0 {
            let listing = self.listing();
            let end = if keys.edge > 0 { listing.last() } else { listing.first() };
            if let Some(&at) = end {
                self.current = Some(at);
                self.selection = BTreeSet::from([at]);
            }
        }
        if (keys.enter || keys.open) && self.current.is_some() {
            self.view = View::Photo;
        }
        if let Some(turns) = keys.rotate {
            self.rotate_targets(turns);
        }
        if keys.select_all {
            self.select_all();
        }
        if keys.invert {
            self.invert_selection();
        }
        if keys.clear {
            self.selection.clear();
        }
        if keys.lucky {
            self.add_effect("enhance");
        }
        if keys.bw {
            self.add_effect("bw");
        }
        if keys.star {
            let star = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.star));
            self.edit_targets(|meta| meta.star = star);
        }
        if keys.small {
            self.thumb_size = 96.0;
        }
        if keys.normal {
            self.thumb_size = 160.0;
        }
    }

    fn info_panel(&mut self, ui: &mut egui::Ui) {
        let targets = self.targets();
        let current = self.current;
        let (action, width) = {
            let file = |at: FileRef| self.folders.get(at.0).and_then(|f| f.files.get(at.1));
            let current = current.and_then(file);
            let target_files: Vec<&MediaFile> = targets.iter().filter_map(|&at| file(at)).collect();
            let panel = egui::Panel::right("info")
                .resizable(true)
                .default_size(self.info_width)
                .show(ui, |ui| self.info.show(ui, current, &target_files));
            (panel.inner, panel.response.rect.width().round())
        };
        self.info_width = width;
        match action {
            Some(TagAction::Add(tag)) => self.edit_targets(|meta| {
                if !meta.keywords.contains(&tag) {
                    meta.keywords.push(tag.clone());
                }
            }),
            Some(TagAction::Remove(tag)) => self.edit_targets(|meta| meta.keywords.retain(|k| k != &tag)),
            None => {}
        }
    }

    fn photo(&mut self, ui: &mut egui::Ui) {
        let listing = self.listing();
        let Some(file) = self.current.and_then(|at| self.folders.get(at.0)?.files.get(at.1)) else {
            // The photo went away (trashed, renamed, rescanned).
            self.set_full_screen(ui.ctx(), false);
            self.view = View::Library;
            return;
        };
        let at = self.current.unwrap_or_default();
        let position = listing.iter().position(|&r| r == at).unwrap_or(0);
        if self.show_filmstrip && !self.viewer.is_full_screen() {
            let items: Vec<(FileRef, &MediaFile)> = listing
                .iter()
                .filter_map(|&r| Some((r, self.folders.get(r.0).and_then(|f| f.files.get(r.1))?)))
                .collect();
            let picked = egui::Panel::top("filmstrip")
                .show(ui, |ui| filmstrip::show(ui, &items, Some(at), &[], &mut self.thumbs, str::to_owned, None))
                .inner;
            if let Some(filmstrip::Pick::Clicked(other) | filmstrip::Pick::DoubleClicked(other)) = picked {
                self.current = Some(other);
                self.selection = BTreeSet::from([other]);
                return;
            }
        }
        match self.viewer.show(ui, file, (position, listing.len().max(1))) {
            Some(ViewerAction::Close) => {
                self.set_full_screen(ui.ctx(), false);
                self.view = View::Library;
            }
            Some(ViewerAction::Previous) => self.step(-1),
            Some(ViewerAction::Next) => self.step(1),
            Some(ViewerAction::First) => self.step(-(listing.len() as isize)),
            Some(ViewerAction::Last) => self.step(listing.len() as isize),
            Some(ViewerAction::FullScreen(on)) => self.set_full_screen(ui.ctx(), on),
            Some(ViewerAction::ToggleStar) => self.edit_files(&[at], |meta| meta.star = !meta.star),
            Some(ViewerAction::Rotate(turns)) => {
                self.edit_files(&[at], |meta| meta.rotate = (meta.rotate as i8 + turns).rem_euclid(4) as u8)
            }
            Some(ViewerAction::SetCaption(caption)) => self.edit_files(&[at], |meta| meta.caption = caption.clone()),
            Some(ViewerAction::SetFilters(filters)) => self.edit_files(&[at], |meta| meta.filters = filters.clone()),
            Some(ViewerAction::Slideshow) => self.start_slideshow(ui.ctx()),
            Some(ViewerAction::Status(message)) => self.status = message,
            Some(ViewerAction::FileAdded(path)) => self.add_file(path),
            None => {}
        }
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.scan.is_some() {
                ui.spinner();
                let found = self.scan_found.load(std::sync::atomic::Ordering::Relaxed);
                ui.label(tr!("{status}  ({n} found so far)", status = self.status, n = found));
                // The count changes on its own; keep it fresh.
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            } else {
                ui.label(&self.status);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if video::ffmpeg().is_none() {
                    ui.label(egui::RichText::new(tr!("ffmpeg not found: videos will stay mysterious")).weak());
                    ui.separator();
                }
                if self.view == View::Library {
                    ui.label(egui::RichText::new(self.status_details()).weak());
                }
            });
        });
    }

    /// The photo under the pointer, else the selection: name, date and size.
    fn status_details(&self) -> String {
        let describe = |file: &MediaFile| {
            let date = file.taken.as_deref().map(crate::i18n::date_label);
            let mut parts = vec![file.name()];
            parts.extend(date);
            parts.push(crate::info::human_size(file.size));
            parts.join("  ·  ")
        };
        if let Some(file) = self.hovered.and_then(|at| self.file(at)) {
            return describe(file);
        }
        match self.selection.len() {
            0 => String::new(),
            1 => self.selection.first().and_then(|&at| self.file(at)).map(describe).unwrap_or_default(),
            n => {
                let bytes: u64 = self.selection.iter().filter_map(|&at| self.file(at)).map(|f| f.size).sum();
                format!("{}  ·  {}", tr!("{n} selected", n = n), crate::info::human_size(bytes))
            }
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.export_job.is_some() {
            let (done, total) = self.export_count;
            let fraction = (total > 0).then(|| done as f32 / total as f32);
            progress_window(
                ctx,
                tr!("Exporting"),
                &tr!("Exporting {done} of {total}…", done = done, total = total),
                fraction,
            );
        }
        if self.movie_job.is_some() {
            let (done, total) = self.movie_frames;
            if done < total {
                let text = tr!("Preparing photo {done} of {total}…", done = done + 1, total = total);
                progress_window(ctx, tr!("Making the Movie"), &text, Some(done as f32 / total as f32));
            } else {
                progress_window(ctx, tr!("Making the Movie"), tr!("Encoding the movie…"), None);
            }
        }
        if self.duplicate_job.is_some() {
            progress_window(ctx, tr!("Finding Duplicates"), tr!("Comparing files…"), None);
        }
        if let Some(mut dialog) = self.export.take() {
            let mut open = true;
            let mut go = false;
            let mut browse = false;
            let count = self.targets().len();
            dialog_window(ctx, tr!("Export to Folder")).open(&mut open).show_dialog(ctx, |ui| {
                ui.label(tr!("Folder:"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dialog.dir).desired_width(300.0));
                    browse = ui.button(tr!("Browse…")).clicked();
                });
                ui.label(tr!("Image size:"));
                for (i, (label, _)) in EXPORT_SIZES.iter().enumerate() {
                    ui.radio_value(&mut dialog.size, i, crate::i18n::tr(label));
                }
                if EXPORT_SIZES[dialog.size].1 == Some(0) {
                    ui.add(egui::Slider::new(&mut dialog.custom, 100..=6000).text(tr!("longest side, px")));
                }
                ui.add(egui::Slider::new(&mut dialog.quality, 40..=100).text(tr!("JPEG quality")));
                ui.label(egui::RichText::new(tr!("Edits are applied; the originals stay untouched.")).weak());
                go = ui.button(tr!("Export {pictures}", pictures = plural(count, "picture"))).clicked();
            });
            if let Some(dir) = self.picked_export_dir.take() {
                dialog.dir = dir.display().to_string();
            }
            if browse {
                self.pick_folder(ctx, FolderPurpose::ExportTo);
            }
            if go && !dialog.dir.trim().is_empty() {
                self.start_export(ctx, &dialog);
            } else if open {
                self.export = Some(dialog);
            }
        }

        if let Some((at, mut name)) = self.rename.take() {
            let mut open = true;
            let mut submit = false;
            dialog_window(ctx, tr!("Rename")).open(&mut open).show_dialog(ctx, |ui| {
                let edit = ui.text_edit_singleline(&mut name);
                edit.request_focus();
                submit = ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.button(tr!("Rename")).clicked();
            });
            if submit {
                match self.folders.get_mut(at.0).map(|f| fileops::rename(f, at.1, &name)) {
                    Some(Ok(())) => {
                        self.status = tr!("Renamed to {name}", name = name);
                        self.albums_dirty = true;
                    }
                    Some(Err(e)) => self.status = tr!("Could not rename: {e}", e = e),
                    None => {}
                }
            } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.rename = Some((at, name));
            }
        }

        if let Some(mut name) = self.new_album.take() {
            let mut open = true;
            let mut create = false;
            dialog_window(ctx, tr!("New Album")).open(&mut open).show_dialog(ctx, |ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("Album name")));
                field.request_focus();
                create = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    || ui.button(tr!("Create")).clicked();
            });
            if create && !name.trim().is_empty() {
                let token = library::new_album_token();
                self.add_to_album(&token, name.trim());
            } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.new_album = Some(name);
            }
        }

        if let Some((token, mut name)) = self.album_rename.take() {
            let mut open = true;
            let mut rename = false;
            dialog_window(ctx, tr!("Rename Album")).open(&mut open).show_dialog(ctx, |ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("Album name")));
                field.request_focus();
                rename = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    || ui.button(tr!("Rename")).clicked();
            });
            if rename && !name.trim().is_empty() {
                self.rename_album(&token, name.trim());
            } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.album_rename = Some((token, name));
            }
        }

        if let Some(token) = self.confirm_album_delete.take() {
            let mut open = true;
            let mut confirmed = false;
            let mut keep = false;
            let name = self.album_name(&token).unwrap_or_default();
            dialog_window(ctx, tr!("Delete Album")).open(&mut open).show_dialog(ctx, |ui| {
                ui.label(tr!("Delete the album \"{name}\"?", name = name));
                ui.label(egui::RichText::new(tr!("The photos stay where they are.")).weak());
                ui.horizontal(|ui| {
                    confirmed = ui.button(tr!("Delete Album")).clicked();
                    keep = ui.button(tr!("Keep")).clicked();
                });
            });
            if confirmed {
                self.delete_album(&token);
            } else if open && !keep && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.confirm_album_delete = Some(token);
            }
        }

        if let Some(files) = self.confirm_trash.take() {
            let mut open = true;
            let mut confirmed = false;
            let mut keep = false;
            dialog_window(ctx, tr!("Move to Trash")).open(&mut open).show_dialog(ctx, |ui| {
                ui.label(tr!("Move {files} to the trash?", files = plural(files.len(), "file")));
                ui.horizontal(|ui| {
                    confirmed = ui.button(tr!("Move to Trash")).clicked();
                    keep = ui.button(tr!("Keep")).clicked();
                });
            });
            if confirmed {
                self.trash(files);
            } else if open && !keep && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.confirm_trash = Some(files);
            }
        }

        if let Some((files, dest)) = self.confirm_move.take() {
            let mut open = true;
            let mut confirmed = false;
            let mut keep = false;
            let name = self.folders.get(dest).map(|f| f.name.clone()).unwrap_or_default();
            dialog_window(ctx, tr!("Move Photos")).open(&mut open).show_dialog(ctx, |ui| {
                ui.label(tr!("Move {files} to \"{name}\"?", files = plural(files.len(), "file"), name = name));
                ui.label(egui::RichText::new(tr!("Captions, stars and edits go along.")).weak());
                ui.horizontal(|ui| {
                    confirmed = ui.button(tr!("Move")).clicked();
                    keep = ui.button(tr!("Cancel")).clicked();
                });
            });
            if confirmed {
                self.move_files(ctx, &files, dest);
            } else if open && !keep && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.confirm_move = Some((files, dest));
            }
        }

        if let Some(mut dialog) = self.movie.take() {
            let mut open = true;
            let mut create = false;
            let count = self
                .targets()
                .iter()
                .filter(|&&at| self.file(at).is_some_and(|f| f.kind != pico_core::media::MediaKind::Video))
                .count();
            dialog_window(ctx, tr!("Create Movie")).open(&mut open).show_dialog(ctx, |ui| {
                ui.label(tr!("{photos} in the order shown.", photos = plural(count, "photo")));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut dialog.full_hd, false, "1280 × 720");
                    ui.radio_value(&mut dialog.full_hd, true, "1920 × 1080");
                });
                ui.add(egui::Slider::new(&mut dialog.seconds, 1.0..=15.0).text(tr!("seconds per photo")));
                ui.add(egui::Slider::new(&mut dialog.fade, 0.0..=3.0).text(tr!("crossfade, seconds")));
                ui.label(tr!("Music (optional):"));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dialog.audio).desired_width(260.0));
                    if ui.button(tr!("Browse…")).clicked() && dialog.pick_audio.is_none() {
                        let (tx, rx) = channel();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(rfd::FileDialog::new().set_title(tr!("Music for the movie")).pick_file());
                            ctx.request_repaint();
                        });
                        dialog.pick_audio = Some(rx);
                    }
                });
                ui.label(tr!("Save as:"));
                ui.add(egui::TextEdit::singleline(&mut dialog.out).desired_width(330.0));
                let busy = self.movie_job.is_some();
                create = ui.add_enabled(count > 0 && !busy, egui::Button::new(tr!("Make the Movie"))).clicked();
            });
            if let Some(rx) = &dialog.pick_audio
                && let Ok(answer) = rx.try_recv()
            {
                if let Some(path) = answer {
                    dialog.audio = path.display().to_string();
                }
                dialog.pick_audio = None;
            }
            if create && !dialog.out.trim().is_empty() {
                self.start_movie(ctx, &dialog);
            } else if open {
                self.movie = Some(dialog);
            }
        }

        if let Some(manager) = &mut self.folder_manager {
            match manager.show(ctx) {
                Some(FolderManagerAction::Apply(rules)) => {
                    self.folder_manager = None;
                    if let Some(rules) = rules {
                        self.set_rules(ctx, rules);
                    }
                }
                Some(FolderManagerAction::Cancel) => self.folder_manager = None,
                Some(FolderManagerAction::Browse) => self.pick_folder(ctx, FolderPurpose::Watch),
                None => {}
            }
        }

        if self.show_options {
            let mut open = true;
            dialog_window(ctx, tr!("Options")).open(&mut open).show_dialog(ctx, |ui| {
                ui.heading(tr!("Look"));
                ui.horizontal(|ui| {
                    let before = self.theme;
                    ui.radio_value(&mut self.theme, egui::ThemePreference::System, tr!("As the system"));
                    ui.radio_value(&mut self.theme, egui::ThemePreference::Dark, tr!("Dark"));
                    ui.radio_value(&mut self.theme, egui::ThemePreference::Light, tr!("Light"));
                    if self.theme != before {
                        self.apply_theme(ctx);
                    }
                });
                ui.label(egui::RichText::new(tr!("Photos keep their dark backdrop either way.")).weak());
                ui.separator();
                ui.heading(tr!("Thumbnails"));
                let dir = config::cache_dir().map(|d| d.join("thumbs"));
                let size = dir.as_deref().map_or(0, dir_size);
                ui.label(tr!("Cache: {size} MB", size = format!("{:.1}", size as f64 / (1u64 << 20) as f64)));
                if let Some(dir) = &dir
                    && ui.button(tr!("Clear Thumbnail Cache")).clicked()
                {
                    let _ = std::fs::remove_dir_all(dir);
                    self.status = tr!("Thumbnail cache cleared; thumbnails will be redrawn as needed.").into();
                }
                ui.separator();
                ui.heading(tr!("Video tools"));
                for (name, found) in [
                    ("ffmpeg", video::ffmpeg()),
                    ("ffprobe", video::ffprobe()),
                    (tr!("ffplay (sound)"), video::ffplay()),
                ] {
                    let text = match found {
                        Some(path) => format!("{name}: {}", path.display()),
                        None => tr!("{name}: not found", name = name),
                    };
                    ui.label(text);
                }
            });
            self.show_options = open && !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        }

        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.show_shortcuts = !self.show_shortcuts;
        }
        if self.show_shortcuts {
            let mut open = true;
            dialog_window(ctx, tr!("Keyboard Shortcuts")).open(&mut open).show_dialog(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    ui.set_min_width(460.0);
                    for (group, keys) in SHORTCUTS {
                        ui.heading(crate::i18n::tr(group));
                        egui::Grid::new(*group).num_columns(2).striped(true).show(ui, |ui| {
                            for (key, what) in *keys {
                                ui.strong(crate::i18n::tr(key));
                                ui.label(crate::i18n::tr(what));
                                ui.end_row();
                            }
                        });
                        ui.add_space(8.0);
                    }
                });
            });
            self.show_shortcuts = open && !ctx.input(|i| i.key_pressed(egui::Key::Escape));
        }

        if self.show_about {
            let logo = self.logo.get_or_insert_with(|| crate::logo::with_title(ctx));
            let logo_size = logo.size_vec2();
            let logo = logo.id();
            // The logo band reaches past the window's margin to its edges.
            let margin = ctx.global_style().spacing.window_margin;
            dialog_window(ctx, tr!("About Pico")).open(&mut self.show_about).resizable(false).show_dialog(ctx, |ui| {
                let width = 560.0;
                ui.set_width(width);
                // Off-white, a touch darker than paper, in light and dark themes alike.
                let band = egui::Frame::new()
                    .fill(egui::Color32::from_gray(236))
                    .inner_margin(egui::Margin::symmetric(12 + margin.left, 12))
                    .outer_margin(egui::Margin {
                        left: -margin.left,
                        right: -margin.right,
                        top: -margin.top,
                        bottom: 0,
                    });
                band.show(ui, |ui| {
                    let inner = width - 24.0;
                    ui.image((logo, egui::vec2(inner, inner * logo_size.y / logo_size.x)));
                });
                egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 10)).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.strong(tr!("Version {version}", version = env!("PICO_VERSION")));
                        ui.label(tr!("No cloud. No uploads. Your photos stay exactly where you left them."));
                        ui.label(egui::RichText::new(pico_core::geo::CREDITS).weak());
                    });
                });
                egui::CollapsingHeader::new(tr!("Open-source licenses")).id_salt("licenses").show(ui, |ui| {
                    crate::logo::licenses(ui);
                });
            });
        }
    }
}

/// A dialog window, first shown in the middle of the app window so it does
/// not cover the menus.
pub fn dialog_window<'a>(ctx: &egui::Context, title: &'a str) -> egui::Window<'a> {
    egui::Window::new(title)
        .collapsible(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
}

/// Lists are not buttons: their rows stay as compact as egui makes them, while
/// buttons get the roomier padding of [`roomier`].
pub fn list_rows(ui: &mut egui::Ui) {
    ui.spacing_mut().button_padding = egui::Style::default().spacing.button_padding;
}

/// A small window for a long operation: what is going on and how far along
/// it is; `None` when that is not known.
pub fn progress_window(ctx: &egui::Context, title: &str, text: &str, fraction: Option<f32>) {
    dialog_window(ctx, title).resizable(false).show_dialog(ctx, |ui| {
        ui.set_width(320.0);
        ui.label(text);
        let bar = match fraction {
            Some(f) => egui::ProgressBar::new(f).show_percentage(),
            None => egui::ProgressBar::new(0.0).animate(true),
        };
        ui.add(bar);
    });
}

/// Shows a dialog with a title bar like the menu bar: the title in body text
/// and the bar as tall as the menu bar. egui sizes title bars by the heading
/// style, so headings are body-sized while the frame is laid out, and the
/// contents get the usual style back.
pub trait ShowDialog {
    fn show_dialog<R>(
        self,
        ctx: &egui::Context,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> Option<egui::InnerResponse<Option<R>>>;
}

impl ShowDialog for egui::Window<'_> {
    fn show_dialog<R>(
        self,
        ctx: &egui::Context,
        add_contents: impl FnOnce(&mut egui::Ui) -> R,
    ) -> Option<egui::InnerResponse<Option<R>>> {
        let usual = ctx.global_style();
        let heading = |style: &egui::Style| style.text_styles[&egui::TextStyle::Heading].clone();
        let (usual_heading, body) = (heading(&usual), usual.text_styles[&egui::TextStyle::Body].clone());
        ctx.all_styles_mut(|style| {
            style.text_styles.insert(egui::TextStyle::Heading, body.clone());
        });
        let response = self.show(ctx, |ui| {
            ui.set_style(usual.clone());
            add_contents(ui)
        });
        ctx.all_styles_mut(|style| {
            style.text_styles.insert(egui::TextStyle::Heading, usual_heading.clone());
        });
        response
    }
}

/// egui's buttons are tight for a desktop app; give their labels more room.
/// Window title bars, on the other hand, are thinner than egui's.
fn roomier(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.button_padding = egui::vec2(16.0, 7.0);
        style.spacing.window_margin = egui::Margin::symmetric(6, 4);
    });
}

struct Keys {
    vertical: isize,
    horizontal: isize,
    enter: bool,
    rotate: Option<i8>,
    select_all: bool,
    invert: bool,
    clear: bool,
    lucky: bool,
    bw: bool,
    star: bool,
    small: bool,
    normal: bool,
    rename: bool,
    trash: bool,
    locate: bool,
    open_editor: bool,
    export: bool,
    new_album: bool,
    hold: bool,
    /// Page Down minus Page Up.
    page: isize,
    /// End minus Home.
    edge: isize,
    /// Ctrl+3: open the photo, Picasa's Edit View.
    open: bool,
}

struct MovieDialog {
    out: String,
    full_hd: bool,
    seconds: f32,
    fade: f32,
    audio: String,
    pick_audio: Option<Receiver<Option<PathBuf>>>,
}

/// A choice from a tray thumbnail's menu or a double click on it.
enum TrayChoice {
    View(FileRef),
    Hold(FileRef),
    Unhold(FileRef),
}

enum MovieProgress {
    Frames(usize, usize),
    Finished(Result<PathBuf, String>),
}

#[derive(Clone, Copy)]
enum FolderPurpose {
    Watch,
    ExportTo,
}

struct ExportDialog {
    dir: String,
    /// Index into [`EXPORT_SIZES`].
    size: usize,
    custom: u32,
    quality: u8,
}

const EXPORT_SIZES: [(&str, Option<u32>); 6] = [
    (tr_mark!("Original size"), None),
    (tr_mark!("800 pixels"), Some(800)),
    (tr_mark!("1024 pixels"), Some(1024)),
    (tr_mark!("1600 pixels"), Some(1600)),
    (tr_mark!("2048 pixels"), Some(2048)),
    (tr_mark!("Custom"), Some(0)),
];

impl ExportDialog {
    fn max_side(&self) -> Option<u32> {
        match EXPORT_SIZES[self.size].1 {
            Some(0) => Some(self.custom.max(16)),
            other => other,
        }
    }
}

enum ExportProgress {
    Done { done: usize, total: usize, failed: usize, dir: PathBuf },
    Failed(String),
}

/// Search matches file and folder names, captions and keywords.
fn matches_query(folder: &Folder, file: &MediaFile, query: &str) -> bool {
    file.name().to_lowercase().contains(query)
        || folder.name.to_lowercase().contains(query)
        || file.meta.caption.as_ref().is_some_and(|c| c.to_lowercase().contains(query))
        || file.meta.keywords.iter().any(|k| k.to_lowercase().contains(query))
}

/// Read-only probes and shortcuts for UI tests.
#[cfg(test)]
impl PicoApp {
    pub fn folder_count(&self) -> usize {
        self.folders.len()
    }

    /// Index of the current photo within its folder.
    pub fn selected_file(&self) -> Option<usize> {
        self.current.map(|(_, i)| i)
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    pub fn listing_names(&self) -> Vec<String> {
        self.listing().into_iter().filter_map(|at| self.file(at)).map(|f| f.name()).collect()
    }

    pub fn is_viewing(&self) -> bool {
        self.view == View::Photo
    }

    pub fn player_state(&self) -> (bool, std::time::Duration, bool) {
        self.viewer.player_state()
    }

    pub fn file_names(&self) -> Vec<String> {
        match self.source {
            Some(Source::Folder(i)) => self.folders[i].files.iter().map(|f| f.name()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn slideshow_index(&self) -> Option<usize> {
        self.slideshow.as_ref().filter(|_| matches!(self.view, View::Slideshow(_))).map(|s| s.index)
    }

    pub fn slideshow_video_playing(&self) -> bool {
        self.slideshow.as_ref().is_some_and(|s| s.video_playing())
    }

    pub fn set_slide_seconds(&mut self, seconds: f64) {
        if let Some(show) = &mut self.slideshow {
            show.seconds = seconds;
        }
    }

    pub fn set_filmstrip(&mut self, on: bool) {
        self.show_filmstrip = on;
    }

    pub fn clipboard_files(&self) -> Vec<String> {
        self.clipboard_files.iter().filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).collect()
    }

    pub fn star_flying(&self) -> bool {
        self.viewer.star_flying()
    }

    pub fn star_puffing(&self) -> bool {
        self.viewer.star_puffing()
    }

    pub fn viewer_loaded(&self) -> bool {
        self.current.and_then(|at| self.file(at)).is_some_and(|file| self.viewer.is_loaded(file))
    }

    pub fn open_file_named(&mut self, name: &str) {
        let found = self
            .folders
            .iter()
            .enumerate()
            .find_map(|(fi, f)| f.files.iter().position(|m| m.name() == name).map(|i| (fi, i)));
        self.current = found;
        self.view = View::Photo;
    }

    pub fn showing_original(&self) -> (bool, bool) {
        self.viewer.showing_original()
    }

    pub fn is_full_screen(&self) -> bool {
        self.viewer.is_full_screen()
    }

    pub fn thumbnail_size(&self) -> f32 {
        self.thumb_size
    }

    /// `sort`: "name", "date", "recent" or "size".
    pub fn set_folder_list(&mut self, tree: bool, sort: &str) {
        self.folder_tree = tree;
        self.folder_sort = match sort {
            "date" => FolderSort::Date,
            "recent" => FolderSort::Recent,
            "size" => FolderSort::Size,
            _ => FolderSort::Name,
        };
    }

    /// The folder highlighted in the list, by name.
    pub fn current_folder_name(&self) -> Option<String> {
        match self.source {
            Some(Source::Folder(i)) => self.folders.get(i).map(|f| f.name.clone()),
            _ => None,
        }
    }

    pub fn status_details_text(&self) -> String {
        self.status_details()
    }

    pub fn settings_for_test(&self, ctx: &egui::Context) -> config::Settings {
        self.settings(ctx)
    }

    pub fn apply_settings_for_test(&mut self, settings: &config::Settings) {
        self.apply_settings(settings);
    }

    /// As if the desktop had said light (`false`) or dark (`true`).
    pub fn set_system_dark(&mut self, ctx: &egui::Context, dark: bool) {
        let (tx, rx) = channel();
        let _ = tx.send(dark);
        self.system_theme = Some(rx);
        self.poll_system_theme(ctx);
    }

    pub fn confirming_move(&self) -> bool {
        self.confirm_move.is_some()
    }

    pub fn tray_len(&self) -> usize {
        self.tray.len()
    }

    pub fn zoom_state(&self) -> (Option<f32>, egui::Vec2, bool) {
        self.viewer.zoom_state()
    }

    pub fn open_edit_panel(&mut self) {
        self.viewer.open_edit_panel();
    }

    pub fn tool_state(&self) -> String {
        self.viewer.tool_state()
    }

    pub fn folder_names(&self) -> Vec<String> {
        self.folders.iter().map(|f| f.name.clone()).collect()
    }

    pub fn select_folder(&mut self, index: usize) {
        self.select_source(Source::Folder(index));
        self.view = View::Library;
    }

    pub fn thumbnails_pending(&self) -> usize {
        self.thumbs.pending()
    }

    /// Edit > Copy All Effects, Select All, Edit > Paste All Effects.
    pub fn copy_and_paste_to_all(&mut self) {
        self.copy_effects();
        self.select_all();
        let current = self.current;
        // Pasting onto the source photo too would apply its effects twice.
        if let Some(current) = current {
            self.selection.remove(&current);
        }
        self.paste_effects();
    }

    pub fn hide_current(&mut self) {
        self.selection = self.current.into_iter().collect();
        self.edit_targets(|meta| meta.hidden = true);
        self.selection.clear();
    }

    pub fn keywords_of(&self, name: &str) -> Vec<String> {
        self.folders
            .iter()
            .flat_map(|f| &f.files)
            .find(|f| f.name() == name)
            .map(|f| f.meta.keywords.clone())
            .unwrap_or_default()
    }

    pub fn sort(&mut self, by: &str, reverse: bool) {
        self.sort_by = match by {
            "date" => SortBy::Date,
            "size" => SortBy::Size,
            "custom" => SortBy::Custom,
            _ => SortBy::Name,
        };
        self.reverse = reverse;
    }

    pub fn is_reversed(&self) -> bool {
        self.reverse
    }

    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_owned();
    }

    pub fn open_month(&mut self, key: &str) {
        self.select_source(Source::Month(key.to_owned()));
    }

    pub fn place_keys(&self) -> Vec<String> {
        library::places(&self.folders).into_iter().map(|p| p.key).collect()
    }

    pub fn album_names(&self) -> Vec<String> {
        self.album_list().into_iter().map(|(_, name, count)| format!("{name} ({count})")).collect()
    }

    /// Gives the album an order, as if read from Pico's config.
    pub fn set_album_order(&mut self, token: &str, name: &str, paths: &[PathBuf]) {
        let files = paths.iter().map(|p| Entry::of(p)).collect();
        let album = Album { token: token.to_owned(), name: name.to_owned(), files, ..Album::default() };
        self.album_files.borrow_mut().insert(token.to_owned(), album);
    }

    /// The album's own file, as Pico would save it.
    pub fn album_file_text(&self, token: &str) -> String {
        album_file::to_text(&self.album_file(token))
    }

    pub fn album_note(&self, token: &str) -> Option<String> {
        self.album_missing_note(token)
    }

    pub fn open_album(&mut self, name: &str) {
        if let Some((token, ..)) = self.album_list().into_iter().find(|a| a.1 == name) {
            self.select_source(Source::Album(token));
        }
    }

    pub fn finding_duplicates(&self) -> bool {
        self.duplicate_job.is_some()
    }

    pub fn set_movie_output(&mut self, path: &std::path::Path) {
        if let Some(dialog) = &mut self.movie {
            dialog.out = path.display().to_string();
            dialog.seconds = 1.0;
            dialog.fade = 0.0;
        }
    }

    pub fn making_movie(&self) -> bool {
        self.movie_job.is_some()
    }

    pub fn collage_ready(&self) -> bool {
        self.collage.as_ref().is_some_and(|c| c.is_ready())
    }

    pub fn collage_layout(&mut self, layout: pico_core::collage::Layout) {
        if let Some(view) = &mut self.collage {
            view.set_layout(layout);
        }
    }

    pub fn in_collage(&self) -> bool {
        self.view == View::Collage
    }

    pub fn status_text(&self) -> &str {
        &self.status
    }

    pub fn watched_count(&self) -> usize {
        self.rules.watched.len()
    }

    pub fn show_starred(&mut self) {
        self.select_source(Source::Starred);
    }

    pub fn set_export_dir(&mut self, dir: &std::path::Path) {
        if let Some(dialog) = &mut self.export {
            dialog.dir = dir.display().to_string();
        }
    }

    pub fn exporting(&self) -> bool {
        self.export_job.is_some()
    }
}

/// Total size of the files in a directory tree, in bytes.
fn dir_size(dir: &std::path::Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            _ => e.metadata().map_or(0, |m| m.len()),
        })
        .sum()
}

/// A ribbon header: the title, count (with `missing` photos Pico cannot show)
/// and the months the photos span, and a note under it.
fn section_header(
    title: String,
    note: Option<String>,
    items: &[(FileRef, &MediaFile)],
    missing: usize,
) -> SectionHeader {
    let months: BTreeSet<String> = items.iter().map(|(_, f)| f.sort_date()[..7].to_owned()).collect();
    let span = match (months.first(), months.last()) {
        (Some(first), Some(last)) if first != last => {
            tr!("{first} to {last}", first = crate::i18n::month_label(first), last = crate::i18n::month_label(last))
        }
        (Some(only), _) => crate::i18n::month_label(only),
        _ => String::new(),
    };
    let videos = items.iter().filter(|(_, f)| f.kind == pico_core::media::MediaKind::Video).count();
    let photos = items.len() - videos;
    let mut count = match (photos, videos) {
        (_, 0) => plural(photos, "photo"),
        (0, _) => plural(videos, "video"),
        _ => format!("{}, {}", plural(photos, "photo"), plural(videos, "video")),
    };
    if missing > 0 {
        count = tr!("{count}, {missing} not in Pico", count = count, missing = missing);
    }
    // A month's own name is the span already.
    let detail = if span.is_empty() || span == title { count } else { format!("{count}  ·  {span}") };
    SectionHeader { title, detail, note }
}

/// A section name in the settings, where `|` and `=` separate them.
fn escape_key(key: &str) -> String {
    key.replace('%', "%25").replace('|', "%7C").replace('=', "%3D")
}

fn unescape_key(key: &str) -> String {
    key.replace("%3D", "=").replace("%7C", "|").replace("%25", "%")
}

/// Highlights `response` while photos are dragged over it, and returns them
/// when they are dropped on it.
fn drop_target(ui: &egui::Ui, response: &egui::Response) -> Option<std::sync::Arc<PhotoDrag>> {
    // Missing photos' tiles only arrange their album.
    if response.dnd_hover_payload::<PhotoDrag>().is_some_and(|drag| !drag.0.is_empty()) {
        let stroke = egui::Stroke::new(2.0, ui.visuals().selection.stroke.color);
        ui.painter().rect_stroke(response.rect, 4.0, stroke, egui::StrokeKind::Inside);
    }
    response.dnd_release_payload::<PhotoDrag>().filter(|drag| !drag.0.is_empty())
}

/// The Help > Keyboard Shortcuts list, by where the keys work.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        tr_mark!("Library"),
        &[
            (tr_mark!("Arrows"), tr_mark!("Move between photos")),
            ("Home / End", tr_mark!("First / last photo")),
            ("Page Up / Page Down", tr_mark!("A screenful up / down")),
            ("Enter, Ctrl+3", tr_mark!("Open the photo")),
            ("Ctrl+A / Ctrl+I / Ctrl+D", tr_mark!("Select all / invert / clear")),
            ("Ctrl+H", tr_mark!("Hold in the Photo Tray")),
            ("S", tr_mark!("Star or unstar")),
            ("Ctrl+N", tr_mark!("New album from the selection")),
            ("F2", tr_mark!("Rename")),
            ("Delete", tr_mark!("Move to the trash")),
            ("Ctrl+Enter", tr_mark!("Locate on disk")),
            ("Ctrl+Shift+O", tr_mark!("Open in another program")),
            ("Ctrl+Shift+S", tr_mark!("Export to a folder")),
            (tr_mark!("Ctrl+1 / Ctrl+2, Ctrl+wheel"), tr_mark!("Thumbnail size")),
            ("Ctrl+F", tr_mark!("Search")),
        ],
    ),
    (
        tr_mark!("Photo"),
        &[
            (tr_mark!("Left / Right, wheel"), tr_mark!("Previous / next photo")),
            ("Home / End", tr_mark!("First / last photo")),
            (tr_mark!("Ctrl+wheel, pinch"), tr_mark!("Zoom")),
            (tr_mark!("Double-click, Esc"), tr_mark!("Back to the library")),
            ("F11", tr_mark!("Full screen")),
            ("Ctrl+Z / Ctrl+Y", tr_mark!("Undo / redo an edit")),
            (tr_mark!("\\ (hold)"), tr_mark!("Show the original")),
        ],
    ),
    (
        tr_mark!("Everywhere"),
        &[
            ("Ctrl+R / Ctrl+Shift+R", tr_mark!("Rotate clockwise / counterclockwise")),
            ("Ctrl+Shift+E / Ctrl+Shift+B", tr_mark!("I'm Feeling Lucky / black and white")),
            ("Ctrl+C", tr_mark!("Copy the files, to paste into a chat or a folder")),
            ("Alt+Enter / Ctrl+K", tr_mark!("Properties and tags / add a tag")),
            ("Ctrl+4", tr_mark!("Slideshow (arrows, Space, -/+ and Esc in it)")),
            ("F1", tr_mark!("This list")),
        ],
    ),
];

/// "1 photo", "5 photos" in the language in use.
fn plural(count: usize, noun: &str) -> String {
    match noun {
        "photo" => trn!("{n} photo", "{n} photos", count),
        "video" => trn!("{n} video", "{n} videos", count),
        "picture" => trn!("{n} picture", "{n} pictures", count),
        "file" => trn!("{n} file", "{n} files", count),
        "folder" => trn!("{n} folder", "{n} folders", count),
        "effect" => trn!("{n} effect", "{n} effects", count),
        "group" => trn!("{n} group", "{n} groups", count),
        _ => format!("{count} {noun}"),
    }
}

impl eframe::App for PicoApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_export();
        self.poll_duplicates();
        self.poll_folder_picker(ctx);
        self.poll_movie();
        self.thumbs.poll(ctx);
        self.take_dropped_files(ctx);
        self.poll_system_theme(ctx);
        self.save_settings_when_settled(ctx);
        if std::mem::take(&mut self.albums_dirty) {
            self.sync_album_files();
        }
        if self.view != View::Photo {
            self.viewer.stop_video();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let View::Slideshow(back) = self.view {
            egui::CentralPanel::no_frame().show(ui, |ui| self.slideshow(ui, back));
            return;
        }
        if self.view == View::Collage {
            egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
            self.collage(ui);
            return;
        }
        if self.view == View::Photo && self.viewer.is_full_screen() {
            self.photo(ui);
            return;
        }
        egui::Panel::top("menu").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Num4)) {
            self.start_slideshow(ui.ctx());
        }
        let (info_key, tags_key) = ui.input(|i| {
            (i.modifiers.alt && i.key_pressed(egui::Key::Enter), i.modifiers.command && i.key_pressed(egui::Key::K))
        });
        if info_key {
            self.show_info = !self.show_info;
        }
        if tags_key {
            self.show_info = true;
            self.info.focus_tag_field = true;
        }
        // Text fields copy their own text.
        let copy = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if copy && matches!(self.view, View::Library | View::Photo) && !ui.ctx().egui_wants_keyboard_input() {
            self.copy_files();
        }
        if self.show_info {
            self.info_panel(ui);
        }
        match self.view {
            View::Library => self.library(ui),
            View::Photo => self.photo(ui),
            View::Slideshow(_) | View::Collage => {}
        }
        self.dialogs(&ui.ctx().clone());
        self.drag_feedback(ui.ctx());
    }
}
