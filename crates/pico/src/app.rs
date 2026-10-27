use crate::collage_view::{CollageAction, CollageView};
use crate::filmstrip;
use crate::folder_manager::{FolderManager, FolderManagerAction};
use crate::grid::{self, CaptionMode, FileRef, GridAction, GridView, MAX_THUMB, MIN_THUMB};
use crate::info::{InfoPanel, TagAction};
use crate::slideshow::{Slideshow, SlideshowAction};
use crate::thumbs::ThumbCache;
use crate::viewer::{Viewer, ViewerAction};
use eframe::egui;
use pico_core::export::{self, ExportOptions};
use pico_core::filters::{Filter, FilterStack};
use pico_core::folders::FolderRules;
use pico_core::library::{self, Folder, MediaFile};
use pico_core::meta::PhotoMeta;
use pico_core::movie::{self, MovieOptions};
use pico_core::{config, fileops, video};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
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
    search: String,
    focus_search: bool,
    starred_only: bool,
    show_hidden: bool,
    captions: CaptionMode,
    sort_by: SortBy,
    reverse: bool,
    show_info: bool,
    /// Picasa's strip of neighbouring photos above the viewer.
    show_filmstrip: bool,
    info: InfoPanel,
    /// "Copy All Effects" clipboard.
    copied_filters: Option<FilterStack>,
    view: View,
    viewer: Viewer,
    slideshow: Option<Slideshow>,
    collage: Option<CollageView>,
    scan: Option<Receiver<Vec<Folder>>>,
    thumbs: ThumbCache,
    thumb_size: f32,
    export: Option<ExportDialog>,
    export_job: Option<Receiver<ExportProgress>>,
    rename: Option<(FileRef, String)>,
    new_album: Option<String>,
    /// Duplicate files, group after group, once a search finished.
    duplicates: Vec<FileRef>,
    duplicate_job: Option<Receiver<Vec<Vec<PathBuf>>>>,
    folder_manager: Option<FolderManager>,
    movie: Option<MovieDialog>,
    movie_job: Option<Receiver<MovieProgress>>,
    /// A running system folder dialog and what its answer is for.
    folder_picker: Option<(FolderPurpose, Receiver<Option<PathBuf>>)>,
    picked_export_dir: Option<PathBuf>,
    show_options: bool,
    confirm_trash: Option<Vec<FileRef>>,
    show_about: bool,
    status: String,
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
            search: String::new(),
            focus_search: false,
            starred_only: false,
            show_hidden: false,
            captions: CaptionMode::None,
            sort_by: SortBy::Name,
            reverse: false,
            show_info: false,
            show_filmstrip: true,
            info: InfoPanel::default(),
            copied_filters: None,
            view: View::Library,
            viewer: Viewer::default(),
            slideshow: None,
            collage: None,
            scan: None,
            thumbs: ThumbCache::new(ctx, persist_settings),
            thumb_size: 160.0,
            export: None,
            export_job: None,
            rename: None,
            new_album: None,
            duplicates: Vec::new(),
            duplicate_job: None,
            folder_manager: None,
            movie: None,
            movie_job: None,
            folder_picker: None,
            picked_export_dir: None,
            show_options: false,
            confirm_trash: None,
            show_about: false,
            status: String::new(),
            persist_settings,
        };
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
        std::thread::spawn(move || {
            let mut folders: Vec<Folder> =
                rules.watched.iter().flat_map(|root| library::scan_excluding(root, &rules.excluded)).collect();
            folders.sort_by(|a, b| a.path.cmp(&b.path));
            folders.dedup_by(|a, b| a.path == b.path);
            library::sort_folders(&mut folders);
            let _ = tx.send(folders);
            ctx.request_repaint();
        });
        self.scan = Some(rx);
        self.status = "Looking for photos… (they're usually hiding in Downloads)".into();
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan else { return };
        let Ok(folders) = rx.try_recv() else { return };
        let total: usize = folders.iter().map(|f| f.files.len()).sum();
        self.status = format!("{}, {}", plural(folders.len(), "folder"), plural(total, "file"));
        let previous = match self.source {
            Some(Source::Folder(i)) => self.folders.get(i).map(|f| f.path.clone()),
            _ => None,
        };
        self.folders = folders;
        self.source = match previous.and_then(|p| self.folders.iter().position(|f| f.path == p)) {
            Some(i) => Some(Source::Folder(i)),
            None if matches!(self.source, Some(Source::Starred | Source::Album(_))) => self.source.take(),
            None if self.source == Some(Source::Duplicates) => Some(Source::Starred),
            None if self.source == Some(Source::Tray) => Some(Source::Tray),
            None => (!self.folders.is_empty()).then_some(Source::Folder(0)),
        };
        self.current = None;
        self.selection.clear();
        self.view = View::Library;
        self.scan = None;
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
                self.status = format!("Could not save the folder list: {e}");
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

    /// Picks the photos tray actions work on: the held tray, else the
    /// selection. A held tray becomes the listing and the selection, so the
    /// existing commands (slideshow, export, collage, movie) see it.
    fn take_tray(&mut self) -> bool {
        if self.tray.is_empty() {
            return !self.targets().is_empty();
        }
        let refs = self.tray_refs();
        self.select_source(Source::Tray);
        self.selection = refs.iter().copied().collect();
        self.current = refs.first().copied();
        !refs.is_empty()
    }

    fn tray_panel(&mut self, ui: &mut egui::Ui) {
        let held = !self.tray.is_empty();
        let refs = if held { self.tray_refs() } else { self.selection.iter().copied().collect() };
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                ui.strong("Photo Tray");
                let note = if held {
                    format!("{} held", plural(refs.len(), "photo"))
                } else if refs.is_empty() {
                    "Select photos, then Hold".to_owned()
                } else {
                    format!("{} selected", plural(refs.len(), "photo"))
                };
                ui.label(egui::RichText::new(note).weak());
                ui.horizontal(|ui| {
                    let any = !self.targets().is_empty();
                    let hold = ui.add_enabled(any, egui::Button::new("Hold"));
                    if hold.on_hover_text("Keep the selection in the tray (Ctrl+H)").clicked() {
                        self.hold();
                    }
                    let clear = ui.add_enabled(held, egui::Button::new("Clear"));
                    if clear.on_hover_text("Empty the tray; the photos stay where they are").clicked() {
                        self.tray.clear();
                        if self.source == Some(Source::Tray) {
                            self.source = None;
                        }
                    }
                });
            });
            ui.separator();
            let items: Vec<(FileRef, &MediaFile)> =
                refs.iter().filter_map(|&at| Some((at, self.folders.get(at.0)?.files.get(at.1)?))).collect();
            let strip_width = (ui.available_width() - 480.0).max(100.0);
            let picked = ui
                .allocate_ui(egui::vec2(strip_width, filmstrip::CELL), |ui| {
                    filmstrip::show(ui, &items, None, &mut self.thumbs, |name| format!("Tray: {name}"))
                })
                .inner;
            if let Some(at) = picked {
                self.current = Some(at);
            }
            ui.separator();
            ui.add_enabled_ui(!refs.is_empty(), |ui| {
                egui::Grid::new("tray actions").num_columns(2).show(ui, |ui| {
                    if ui.button("▶ Slideshow").clicked() && self.take_tray() {
                        self.start_slideshow(ui.ctx());
                    }
                    if ui.button("Export…").clicked() && self.take_tray() {
                        self.open_export();
                    }
                    ui.end_row();
                    if ui.button("Make a Collage…").clicked() && self.take_tray() {
                        self.open_collage(ui.ctx());
                    }
                    if ui.button("Make a Movie…").clicked() && self.take_tray() {
                        self.open_movie_dialog();
                    }
                    ui.end_row();
                    if held && ui.button("Show Tray").clicked() {
                        self.select_source(Source::Tray);
                    }
                    let remove = held && {
                        let button = ui.button("Remove");
                        button.on_hover_text("Take the selected photos out of the tray").clicked()
                    };
                    if remove {
                        let paths: Vec<PathBuf> =
                            self.selection.iter().filter_map(|&at| self.file(at).map(|f| f.path.clone())).collect();
                        self.tray.retain(|p| !paths.contains(p));
                    }
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let slider = egui::Slider::new(&mut self.thumb_size, MIN_THUMB..=MAX_THUMB).show_value(false);
                ui.add_sized([110.0, 18.0], slider).on_hover_text("Thumbnail size (Ctrl+wheel over the photos)");
                ui.label(egui::RichText::new("🗖").weak());
            });
        });
    }

    fn file(&self, at: FileRef) -> Option<&MediaFile> {
        self.folders.get(at.0)?.files.get(at.1)
    }

    /// The files the grid shows, in order: the source folder (or all folders
    /// while searching or showing starred photos), minus hidden photos and,
    /// optionally, unstarred ones.
    fn listing(&self) -> Vec<FileRef> {
        let query = self.search.trim().to_lowercase();
        let album = match &self.source {
            Some(Source::Album(token)) if query.is_empty() => Some(token.as_str()),
            _ => None,
        };
        if self.source == Some(Source::Duplicates) && query.is_empty() {
            return self.duplicates.iter().copied().filter(|&at| self.file(at).is_some()).collect();
        }
        if self.source == Some(Source::Tray) && query.is_empty() {
            return self.tray_refs();
        }
        let month = match &self.source {
            Some(Source::Month(key)) if query.is_empty() => Some(key.as_str()),
            _ => None,
        };
        let place = match &self.source {
            Some(Source::Place(key)) if query.is_empty() => Some(key.as_str()),
            _ => None,
        };
        let by_date = month.is_some() || place.is_some();
        let all_folders = !query.is_empty()
            || matches!(self.source, Some(Source::Starred | Source::Album(_) | Source::Month(_) | Source::Place(_)));
        let folders: Vec<usize> = match self.source {
            _ if all_folders => (0..self.folders.len()).collect(),
            Some(Source::Folder(i)) if i < self.folders.len() => vec![i],
            _ => Vec::new(),
        };
        let starred = self.starred_only || self.source == Some(Source::Starred);
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
                        && (!starred || f.meta.star);
                    let matches = query.is_empty() || matches_query(folder, f, query);
                    (visible && matches).then_some((fi, i))
                })
            })
            .collect();
        // Folders are already in name order; other orders (and names across
        // several folders) need sorting.
        if self.sort_by != SortBy::Name || all_folders {
            let file = |at: &FileRef| &self.folders[at.0].files[at.1];
            // Timeline and Places read best in the order the photos were taken.
            let sort_by = if by_date && self.sort_by == SortBy::Name { SortBy::Date } else { self.sort_by };
            match sort_by {
                SortBy::Name => listing.sort_by(|a, b| library::natural_cmp(&file(a).name(), &file(b).name())),
                SortBy::Date => listing.sort_by_cached_key(|at| file(at).sort_date()),
                SortBy::Size => listing.sort_by_key(|at| file(at).size),
            }
        }
        if self.reverse {
            listing.reverse();
        }
        listing
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
                    saved += changed.len();
                    for (i, old) in &before {
                        let file = &folder.files[*i];
                        if file.meta.rotate != old.rotate || file.meta.filters != old.filters {
                            self.thumbs.invalidate(&file.path);
                        }
                    }
                    self.status = format!("Saved to {}", folder.ini_path.display());
                }
                Err(e) => {
                    // Keep the screen honest: undo changes that did not reach the disk.
                    for (i, old) in before {
                        folder.files[i].meta = old;
                    }
                    self.status = format!("Could not write {}: {e}", folder.ini_path.display());
                    return;
                }
            }
        }
        if saved > 1 {
            self.status = format!("Updated {saved} photos");
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

    fn copy_effects(&mut self) {
        if let Some(filters) = self.current.and_then(|at| self.file(at)).map(|f| f.meta.filters.clone()) {
            self.status = format!("Copied {}", plural(filters.0.len(), "effect"));
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
            0 => "No duplicates. Every photo is one of a kind.".into(),
            n => format!("{} of identical files", plural(n, "group")),
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
                FolderPurpose::Watch => "Add a folder to Pico",
                FolderPurpose::ExportTo => "Export to folder",
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
                self.status = format!("Collage saved to {}", path.display());
                if wallpaper {
                    match pico_core::fileops::set_wallpaper(&path) {
                        Ok(()) => self.status += " and set as the desktop background",
                        Err(e) => self.status = format!("Saved, but could not set the wallpaper: {e}"),
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
        self.status = "Making a movie… popcorn recommended.".into();
    }

    fn poll_movie(&mut self) {
        let Some(rx) = &self.movie_job else { return };
        let mut finished = None;
        while let Ok(progress) = rx.try_recv() {
            match progress {
                MovieProgress::Frames(done, total) => self.status = format!("Preparing photo {done} of {total}…"),
                MovieProgress::Finished(result) => finished = Some(result),
            }
        }
        if let Some(result) = finished {
            self.movie_job = None;
            match result {
                Ok(path) => {
                    self.status = format!("Movie saved to {}", path.display());
                    self.add_file(path);
                }
                Err(e) => self.status = format!("Could not make the movie: {e}"),
            }
        }
    }

    fn album_name(&self, token: &str) -> Option<String> {
        library::albums(&self.folders).into_iter().find(|a| a.token == token).map(|a| a.name)
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
        self.status = format!("Added {} to \"{name}\"", plural(targets.len(), "photo"));
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
        self.status = format!("Saved {}", path.display());
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
        self.source = Some(source);
        self.current = None;
        self.selection.clear();
        self.search.clear();
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Add Folder to Pico…").clicked() {
                    self.open_folder_manager();
                }
                if ui.button("Rescan Folders").clicked() {
                    self.rescan(ui.ctx());
                }
                ui.separator();
                let any = !self.targets().is_empty();
                ui.add_enabled_ui(any, |ui| self.file_menu(ui));
                ui.separator();
                if ui.button("Exit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Select All").on_hover_text("Ctrl+A").clicked() {
                    self.select_all();
                }
                if ui.button("Invert Selection").on_hover_text("Ctrl+I").clicked() {
                    self.invert_selection();
                }
                if ui.button("Clear Selection").on_hover_text("Ctrl+D").clicked() {
                    self.selection.clear();
                }
                ui.separator();
                if ui.add_enabled(self.current.is_some(), egui::Button::new("Copy All Effects")).clicked() {
                    self.copy_effects();
                }
                if ui.add_enabled(self.copied_filters.is_some(), egui::Button::new("Paste All Effects")).clicked() {
                    self.paste_effects();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Slideshow").on_hover_text("Ctrl+4").clicked() {
                    self.start_slideshow(ui.ctx());
                }
                ui.add(egui::Slider::new(&mut self.thumb_size, MIN_THUMB..=MAX_THUMB).text("Thumbnail size"));
                ui.menu_button("Thumbnail Captions", |ui| {
                    for (mode, label) in CaptionMode::ALL {
                        ui.radio_value(&mut self.captions, mode, label);
                    }
                });
                ui.menu_button("Sort", |ui| {
                    ui.radio_value(&mut self.sort_by, SortBy::Name, "By Name");
                    ui.radio_value(&mut self.sort_by, SortBy::Date, "By Date Taken");
                    ui.radio_value(&mut self.sort_by, SortBy::Size, "By Size");
                    ui.separator();
                    ui.checkbox(&mut self.reverse, "Reverse Order");
                });
                ui.checkbox(&mut self.starred_only, "Only Starred Photos");
                ui.checkbox(&mut self.show_info, "Properties and Tags").on_hover_text("Alt+Enter, Ctrl+K");
                ui.checkbox(&mut self.show_filmstrip, "Filmstrip")
                    .on_hover_text("Neighbouring photos above the open one");
                ui.checkbox(&mut self.show_hidden, "Hidden Pictures");
            });
            ui.menu_button("Picture", |ui| {
                let any = !self.targets().is_empty();
                ui.add_enabled_ui(any, |ui| self.picture_menu(ui));
            });
            ui.menu_button("Create", |ui| {
                let photos = self.targets().len();
                if ui
                    .add_enabled(photos > 0, egui::Button::new("Movie…"))
                    .on_hover_text("From the selected photos")
                    .clicked()
                {
                    self.open_movie_dialog();
                }
                if ui.button("Picture Collage…").on_hover_text("From the selected photos, or all shown").clicked() {
                    self.open_collage(ui.ctx());
                }
            });
            ui.menu_button("Tools", |ui| {
                if ui.button("Show Duplicate Files").clicked() {
                    self.find_duplicates(ui.ctx());
                }
                if ui.button("Folder Manager…").clicked() {
                    self.open_folder_manager();
                }
                if ui.button("Options…").clicked() {
                    self.show_options = true;
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About Pico").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Export Picture to Folder…").on_hover_text("Ctrl+Shift+S").clicked() {
            self.open_export();
        }
        if ui.button("Save").on_hover_text("Write the edits into the file; the original is kept").clicked() {
            self.save_targets();
        }
        if ui.button("Save a Copy").clicked() {
            self.save_copies();
        }
        if ui.button("Revert").on_hover_text("Bring back the original saved by Save").clicked() {
            self.revert_targets();
        }
        ui.separator();
        if ui.button("Rename…").on_hover_text("F2").clicked() {
            self.start_rename();
        }
        if ui.button("Locate on Disk").on_hover_text("Ctrl+Enter").clicked() {
            self.locate_on_disk();
        }
        if ui.button("Open in Editor").on_hover_text("Ctrl+Shift+O").clicked() {
            self.open_in_editor();
        }
        if ui.button("Move to Trash…").on_hover_text("Delete").clicked() {
            self.confirm_trash = Some(self.targets());
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
    }

    fn poll_export(&mut self) {
        let Some(rx) = &self.export_job else { return };
        while let Ok(progress) = rx.try_recv() {
            match progress {
                ExportProgress::Failed(message) => self.status = message,
                ExportProgress::Done { done, total, failed, dir } => {
                    self.status = if done < total {
                        format!("Exporting {done} of {total}…")
                    } else if failed == 0 {
                        format!("Exported {} to {}", plural(total, "picture"), dir.display())
                    } else {
                        format!("Exported {} of {total}; {failed} failed", total - failed)
                    };
                    if done == total {
                        self.export_job = None;
                        return;
                    }
                }
            }
        }
    }

    /// Runs a file operation on every target, reporting the first failure.
    fn for_each_target(&mut self, what: &str, op: impl Fn(&mut Folder, usize) -> std::io::Result<()>) -> usize {
        let mut done = 0;
        for (fi, i) in self.targets() {
            let Some(folder) = self.folders.get_mut(fi) else { continue };
            let path = folder.files[i].path.clone();
            match op(folder, i) {
                Ok(()) => {
                    done += 1;
                    self.thumbs.invalidate(&path);
                }
                Err(e) => {
                    self.status = format!("Could not {what} {}: {e}", path.display());
                    return done;
                }
            }
        }
        done
    }

    fn save_targets(&mut self) {
        let done = self.for_each_target("save", export::save_in_place);
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
                Err(e) => self.status = format!("Could not save a copy of {}: {e}", file.name()),
            }
        }
        for path in added {
            self.add_file(path);
        }
    }

    fn revert_targets(&mut self) {
        let done = self.for_each_target("revert", |folder, i| {
            if export::has_backup(folder, i) { export::revert(folder, i) } else { Ok(()) }
        });
        if done > 0 {
            self.status = "Reverted. The edits are back in the edit stack, where Undo can reach them.".into();
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
            self.status = format!("Could not open the file manager: {e}");
        }
    }

    fn open_in_editor(&mut self) {
        if let Some(file) = self.current.and_then(|at| self.file(at))
            && let Err(e) = fileops::open_external(&file.path)
        {
            self.status = format!("Could not open {}: {e}", file.name());
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
                Ok(_) => moved += 1,
                Err(e) => {
                    self.status = format!("Could not move to the trash: {e}");
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
            self.status = format!("Moved {} to the trash. It's not gone gone.", plural(moved, "file"));
        }
    }

    /// Right-click on a thumbnail; works on the selection, which holds `at`.
    fn photo_context_menu(&mut self, ui: &mut egui::Ui, at: FileRef) {
        let many = self.selection.len() > 1;
        if !many && ui.button("View and Edit").clicked() {
            self.current = Some(at);
            self.view = View::Photo;
        }
        let star = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.star));
        if ui.button(if star { "Star" } else { "Unstar" }).on_hover_text("S").clicked() {
            self.edit_targets(|meta| meta.star = star);
        }
        if ui.button("Rotate Clockwise").on_hover_text("Ctrl+R").clicked() {
            self.rotate_targets(1);
        }
        if ui.button("Rotate Counterclockwise").on_hover_text("Ctrl+Shift+R").clicked() {
            self.rotate_targets(-1);
        }
        self.album_menu(ui);
        if ui.button("Hold in Tray").on_hover_text("Ctrl+H").clicked() {
            self.hold();
        }
        let hide = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.hidden));
        if ui.button(if hide { "Hide" } else { "Unhide" }).clicked() {
            self.edit_targets(|meta| meta.hidden = hide);
        }
        ui.separator();
        if ui.button("Export to Folder…").on_hover_text("Ctrl+Shift+S").clicked() {
            self.open_export();
        }
        if !many {
            if ui.button("Locate on Disk").on_hover_text("Ctrl+Enter").clicked() {
                self.locate_on_disk();
            }
            if ui.button("Open in Editor").on_hover_text("Ctrl+Shift+O").clicked() {
                self.open_in_editor();
            }
            if ui.button("Rename…").on_hover_text("F2").clicked() {
                self.start_rename();
            }
            if ui.button("Copy Full Path").clicked()
                && let Some(file) = self.file(at)
            {
                ui.ctx().copy_text(file.path.display().to_string());
            }
        }
        if ui.button("Move to Trash…").on_hover_text("Delete").clicked() {
            self.confirm_trash = Some(self.targets());
        }
    }

    /// Right-click on a folder in the list.
    fn folder_context_menu(&mut self, ui: &mut egui::Ui, index: usize) {
        let Some(path) = self.folders.get(index).map(|f| f.path.clone()) else { return };
        if ui.button("View Slideshow").clicked() {
            self.select_source(Source::Folder(index));
            self.start_slideshow(ui.ctx());
        }
        if ui.button("Locate on Disk").clicked()
            && let Err(e) = fileops::open_external(&path)
        {
            self.status = format!("Could not open the file manager: {e}");
        }
        if ui.button("Copy Full Path").clicked() {
            ui.ctx().copy_text(path.display().to_string());
        }
        ui.separator();
        if ui.button("Rescan Folders").clicked() {
            self.rescan(ui.ctx());
        }
        if ui.button("Remove from Pico").on_hover_text("Leave this folder out; no files are touched").clicked() {
            let mut rules = self.rules.clone();
            rules.set_in_library(&path, false);
            let name = Folder::file_name(&path);
            self.set_rules(ui.ctx(), rules);
            self.status = format!("Left out {name}. Tools > Folder Manager brings it back.");
        }
    }

    fn album_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Add to Album", |ui| {
            for album in library::albums(&self.folders) {
                if ui.button(&album.name).clicked() {
                    self.add_to_album(&album.token, &album.name);
                }
            }
            if ui.button("New Album…").on_hover_text("Ctrl+N").clicked() {
                self.new_album = Some(String::new());
            }
        });
    }

    fn picture_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Star / Unstar").on_hover_text("S").clicked() {
            let star = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.star));
            self.edit_targets(|meta| meta.star = star);
        }
        if ui.button("Rotate Clockwise").on_hover_text("Ctrl+R").clicked() {
            self.rotate_targets(1);
        }
        if ui.button("Rotate Counterclockwise").on_hover_text("Ctrl+Shift+R").clicked() {
            self.rotate_targets(-1);
        }
        ui.separator();
        for (label, id, keys) in [
            ("I'm Feeling Lucky", "enhance", "Ctrl+Shift+E"),
            ("Auto Contrast", "autolight", ""),
            ("Auto Color", "autocolor", ""),
            ("Black-and-White", "bw", "Ctrl+Shift+B"),
            ("Warmify", "warm", ""),
            ("Film Grain", "grain2", ""),
            ("Sharpen", "unsharp2", ""),
        ] {
            if ui.button(label).on_hover_text(keys).clicked() {
                self.add_effect(id);
            }
        }
        ui.separator();
        if ui.button("Hold in Tray").on_hover_text("Ctrl+H").clicked() {
            self.hold();
        }
        self.album_menu(ui);
        if let Some(Source::Album(token)) = self.source.clone()
            && ui.button("Remove from Album").clicked()
        {
            self.edit_targets(|meta| meta.albums.retain(|t| t != &token));
            self.selection.clear();
        }
        ui.separator();
        if ui.button("Undo All Edits").clicked() {
            self.edit_targets(|meta| {
                meta.filters = FilterStack::default();
                meta.rotate = 0;
            });
        }
        let hide = !self.targets().iter().all(|&at| self.file(at).is_some_and(|f| f.meta.hidden));
        if ui.button(if hide { "Hide" } else { "Unhide" }).clicked() {
            self.edit_targets(|meta| meta.hidden = hide);
        }
    }

    fn folder_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("Library");
        ui.separator();
        let starred = self.folders.iter().flat_map(|f| &f.files).filter(|f| f.meta.star).count();
        if ui.selectable_label(self.source == Some(Source::Starred), format!("★ Starred Photos  ({starred})")).clicked()
        {
            self.select_source(Source::Starred);
        }
        if !self.tray.is_empty()
            && ui
                .selectable_label(self.source == Some(Source::Tray), format!("Photo Tray  ({})", self.tray.len()))
                .clicked()
        {
            self.select_source(Source::Tray);
        }
        let albums = library::albums(&self.folders);
        let mut picked_album = None;
        if !albums.is_empty() {
            ui.label(egui::RichText::new("Albums").strong());
            for album in &albums {
                let open = self.source == Some(Source::Album(album.token.clone()));
                if ui.selectable_label(open, format!("{}  ({})", album.name, album.members.len())).clicked() {
                    picked_album = Some(album.token.clone());
                }
            }
        }
        if let Some(token) = picked_album {
            self.select_source(Source::Album(token));
        }
        let mut picked_source = None;
        let months = library::months(&self.folders);
        if !months.is_empty() {
            egui::CollapsingHeader::new("Timeline").id_salt("timeline").show(ui, |ui| {
                let mut years: Vec<(&str, Vec<&(String, usize)>)> = Vec::new();
                for entry in &months {
                    match years.last_mut() {
                        Some((year, list)) if *year == &entry.0[..4] => list.push(entry),
                        _ => years.push((&entry.0[..4], vec![entry])),
                    }
                }
                for (year, list) in years {
                    let total: usize = list.iter().map(|(_, n)| n).sum();
                    egui::CollapsingHeader::new(format!("{year}  ({total})")).id_salt(year).show(ui, |ui| {
                        for (key, count) in list {
                            let open = self.source == Some(Source::Month(key.clone()));
                            let label = format!("{}  ({count})", library::month_label(key));
                            if ui.selectable_label(open, label).clicked() {
                                picked_source = Some(Source::Month(key.clone()));
                            }
                        }
                    });
                }
            });
        }
        let places = library::places(&self.folders);
        if !places.is_empty() {
            egui::CollapsingHeader::new("Places").id_salt("places").show(ui, |ui| {
                for place in &places {
                    let open = self.source == Some(Source::Place(place.key.clone()));
                    let label = format!("{}  ({})", library::place_label(place.center), place.members.len());
                    if ui.selectable_label(open, label).clicked() {
                        picked_source = Some(Source::Place(place.key.clone()));
                    }
                }
            });
        }
        if let Some(source) = picked_source {
            self.select_source(source);
        }
        ui.separator();
        ui.label(egui::RichText::new("Folders").strong());
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut picked = None;
            let mut menus = Vec::new();
            for (index, folder) in self.folders.iter().enumerate() {
                let label = format!("{}  ({})", folder.name, folder.files.len());
                let response = ui
                    .selectable_label(self.source == Some(Source::Folder(index)), label)
                    .on_hover_text(folder.path.display().to_string());
                if response.clicked() {
                    picked = Some(index);
                }
                menus.push((index, response));
            }
            if let Some(index) = picked {
                self.select_source(Source::Folder(index));
            }
            for (index, response) in menus {
                response.context_menu(|ui| self.folder_context_menu(ui, index));
            }
        });
    }

    fn library(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("folders").resizable(true).default_size(260.0).show(ui, |ui| self.folder_list(ui));
        egui::Panel::bottom("tray").show(ui, |ui| self.tray_panel(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| {
            if self.folders.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.scan.is_some() {
                        "Counting pixels…"
                    } else {
                        "No photos yet. Add a folder from the File menu — Pico promises not to upload anything."
                    });
                });
                return;
            }
            self.grid_header(ui);
            let listing = self.listing();
            if listing.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.search.trim().is_empty() {
                        "Nothing to show here."
                    } else {
                        "No matches. Maybe it was a dream?"
                    })
                });
                return;
            }
            // Ctrl+wheel or a pinch over the grid resizes the thumbnails.
            let zoom = ui.input(|i| i.zoom_delta());
            if zoom != 1.0 && ui.rect_contains_pointer(ui.max_rect()) {
                self.thumb_size = (self.thumb_size * zoom).clamp(MIN_THUMB, MAX_THUMB);
            }
            let items: Vec<(FileRef, &MediaFile)> =
                listing.iter().filter_map(|&at| Some((at, self.folders.get(at.0)?.files.get(at.1)?))).collect();
            let view = GridView {
                items: &items,
                thumb_size: self.thumb_size,
                selection: &self.selection,
                current: self.current,
                captions: self.captions,
            };
            let (action, cells) = grid::show(ui, view, &mut self.thumbs);
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
                Some(GridAction::Open(at)) => {
                    self.current = Some(at);
                    self.selection = BTreeSet::from([at]);
                    self.view = View::Photo;
                }
                None => {}
            }
        });
        self.library_keys(ui);
    }

    fn grid_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let searching = !self.search.trim().is_empty();
            match &self.source {
                _ if searching => {
                    ui.heading("Search results");
                }
                Some(Source::Starred) => {
                    ui.heading("Starred Photos");
                }
                Some(Source::Tray) => {
                    ui.heading("Photo Tray");
                    ui.label(egui::RichText::new("Held photos from any folder").weak());
                }
                Some(Source::Album(token)) => {
                    ui.heading(self.album_name(token).unwrap_or_default());
                    ui.label(egui::RichText::new("Album").weak());
                }
                Some(Source::Month(key)) => {
                    ui.heading(library::month_label(key));
                    ui.label(egui::RichText::new("Timeline").weak());
                }
                Some(Source::Place(key)) => {
                    let place = library::places(&self.folders).into_iter().find(|p| &p.key == key);
                    ui.heading(place.map_or_else(|| key.clone(), |p| library::place_label(p.center)));
                    ui.label(egui::RichText::new("Place").weak());
                }
                Some(Source::Duplicates) => {
                    ui.heading("Duplicate Files");
                    let text = if self.duplicate_job.is_some() {
                        "Comparing files…"
                    } else {
                        "Identical files, side by side"
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
                    egui::TextEdit::singleline(&mut self.search).hint_text("Search (Ctrl+F)").desired_width(220.0),
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
                ui.toggle_value(&mut self.starred_only, "★").on_hover_text("Only starred photos");
                let count = self.selection.len();
                if count > 1 {
                    ui.label(format!("{count} selected"));
                }
            });
        });
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
            self.confirm_trash = Some(self.targets());
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
        let columns = grid::columns_for(ui.available_width().max(self.thumb_size), self.thumb_size) as isize;
        self.step(keys.vertical * columns + keys.horizontal);
        if keys.enter && self.current.is_some() {
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
        let action = {
            let file = |at: FileRef| self.folders.get(at.0).and_then(|f| f.files.get(at.1));
            let current = current.and_then(file);
            let target_files: Vec<&MediaFile> = targets.iter().filter_map(|&at| file(at)).collect();
            egui::Panel::right("info")
                .resizable(true)
                .default_size(280.0)
                .show(ui, |ui| self.info.show(ui, current, &target_files))
                .inner
        };
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
                .show(ui, |ui| filmstrip::show(ui, &items, Some(at), &mut self.thumbs, str::to_owned))
                .inner;
            if let Some(other) = picked {
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
            ui.label(&self.status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if video::ffmpeg().is_none() {
                    ui.label(egui::RichText::new("ffmpeg not found: videos will stay mysterious").weak());
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(mut dialog) = self.export.take() {
            let mut open = true;
            let mut go = false;
            let mut browse = false;
            let count = self.targets().len();
            egui::Window::new("Export to Folder").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.label("Folder:");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dialog.dir).desired_width(300.0));
                    browse = ui.button("Browse…").clicked();
                });
                ui.label("Image size:");
                for (i, (label, _)) in EXPORT_SIZES.iter().enumerate() {
                    ui.radio_value(&mut dialog.size, i, *label);
                }
                if EXPORT_SIZES[dialog.size].1 == Some(0) {
                    ui.add(egui::Slider::new(&mut dialog.custom, 100..=6000).text("longest side, px"));
                }
                ui.add(egui::Slider::new(&mut dialog.quality, 40..=100).text("JPEG quality"));
                ui.label(egui::RichText::new("Edits are applied; the originals stay untouched.").weak());
                go = ui.button(format!("Export {}", plural(count, "picture"))).clicked();
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
            egui::Window::new("Rename").open(&mut open).collapsible(false).show(ctx, |ui| {
                let edit = ui.text_edit_singleline(&mut name);
                edit.request_focus();
                submit = ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.button("Rename").clicked();
            });
            if submit {
                match self.folders.get_mut(at.0).map(|f| fileops::rename(f, at.1, &name)) {
                    Some(Ok(())) => self.status = format!("Renamed to {name}"),
                    Some(Err(e)) => self.status = format!("Could not rename: {e}"),
                    None => {}
                }
            } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.rename = Some((at, name));
            }
        }

        if let Some(mut name) = self.new_album.take() {
            let mut open = true;
            let mut create = false;
            egui::Window::new("New Album").open(&mut open).collapsible(false).show(ctx, |ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut name).hint_text("Album name"));
                field.request_focus();
                create = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    || ui.button("Create").clicked();
            });
            if create && !name.trim().is_empty() {
                let token = library::new_album_token();
                self.add_to_album(&token, name.trim());
            } else if open && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.new_album = Some(name);
            }
        }

        if let Some(files) = self.confirm_trash.take() {
            let mut open = true;
            let mut confirmed = false;
            let mut keep = false;
            egui::Window::new("Move to Trash").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.label(format!("Move {} to the trash?", plural(files.len(), "file")));
                ui.horizontal(|ui| {
                    confirmed = ui.button("Move to Trash").clicked();
                    keep = ui.button("Keep").clicked();
                });
            });
            if confirmed {
                self.trash(files);
            } else if open && !keep && !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.confirm_trash = Some(files);
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
            egui::Window::new("Create Movie").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.label(format!("{} in the order shown.", plural(count, "photo")));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut dialog.full_hd, false, "1280 × 720");
                    ui.radio_value(&mut dialog.full_hd, true, "1920 × 1080");
                });
                ui.add(egui::Slider::new(&mut dialog.seconds, 1.0..=15.0).text("seconds per photo"));
                ui.add(egui::Slider::new(&mut dialog.fade, 0.0..=3.0).text("crossfade, seconds"));
                ui.label("Music (optional):");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut dialog.audio).desired_width(260.0));
                    if ui.button("Browse…").clicked() && dialog.pick_audio.is_none() {
                        let (tx, rx) = channel();
                        let ctx = ui.ctx().clone();
                        std::thread::spawn(move || {
                            let _ = tx.send(rfd::FileDialog::new().set_title("Music for the movie").pick_file());
                            ctx.request_repaint();
                        });
                        dialog.pick_audio = Some(rx);
                    }
                });
                ui.label("Save as:");
                ui.add(egui::TextEdit::singleline(&mut dialog.out).desired_width(330.0));
                let busy = self.movie_job.is_some();
                create = ui.add_enabled(count > 0 && !busy, egui::Button::new("Make the Movie")).clicked();
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
            egui::Window::new("Options").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.heading("Thumbnails");
                let dir = config::cache_dir().map(|d| d.join("thumbs"));
                let size = dir.as_deref().map_or(0, dir_size);
                ui.label(format!("Cache: {:.1} MB", size as f64 / (1u64 << 20) as f64));
                if let Some(dir) = &dir
                    && ui.button("Clear Thumbnail Cache").clicked()
                {
                    let _ = std::fs::remove_dir_all(dir);
                    self.status = "Thumbnail cache cleared; thumbnails will be redrawn as needed.".into();
                }
                ui.separator();
                ui.heading("Video tools");
                for (name, found) in
                    [("ffmpeg", video::ffmpeg()), ("ffprobe", video::ffprobe()), ("ffplay (sound)", video::ffplay())]
                {
                    let text = match found {
                        Some(path) => format!("{name}: {}", path.display()),
                        None => format!("{name}: not found"),
                    };
                    ui.label(text);
                }
            });
            self.show_options = open;
        }

        if self.show_about {
            egui::Window::new("About Pico").open(&mut self.show_about).collapsible(false).show(ctx, |ui| {
                ui.heading(format!("Pico {}", env!("CARGO_PKG_VERSION")));
                ui.label("A small photo organizer that remembers how Picasa did things.");
                ui.label("No cloud. No uploads. Your photos stay exactly where you left them.");
            });
        }
    }
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
}

struct MovieDialog {
    out: String,
    full_hd: bool,
    seconds: f32,
    fade: f32,
    audio: String,
    pick_audio: Option<Receiver<Option<PathBuf>>>,
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
    ("Original size", None),
    ("800 pixels", Some(800)),
    ("1024 pixels", Some(1024)),
    ("1600 pixels", Some(1600)),
    ("2048 pixels", Some(2048)),
    ("Custom", Some(0)),
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

    pub fn set_slide_seconds(&mut self, seconds: f64) {
        if let Some(show) = &mut self.slideshow {
            show.seconds = seconds;
        }
    }

    pub fn set_filmstrip(&mut self, on: bool) {
        self.show_filmstrip = on;
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
            _ => SortBy::Name,
        };
        self.reverse = reverse;
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
        library::albums(&self.folders).into_iter().map(|a| format!("{} ({})", a.name, a.members.len())).collect()
    }

    pub fn open_album(&mut self, name: &str) {
        if let Some(album) = library::albums(&self.folders).into_iter().find(|a| a.name == name) {
            self.select_source(Source::Album(album.token));
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

fn plural(count: usize, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

impl eframe::App for PicoApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_export();
        self.poll_duplicates();
        self.poll_folder_picker(ctx);
        self.poll_movie();
        self.thumbs.poll(ctx);
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
        if self.show_info {
            self.info_panel(ui);
        }
        match self.view {
            View::Library => self.library(ui),
            View::Photo => self.photo(ui),
            View::Slideshow(_) | View::Collage => {}
        }
        self.dialogs(&ui.ctx().clone());
    }
}
