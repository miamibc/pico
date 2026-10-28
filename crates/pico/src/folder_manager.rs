//! Folder Manager: a tree of the disk where each folder is put into the
//! library or left out of it. Changes apply only on OK.

use crate::app::ShowDialog;
use eframe::egui::{self, Color32, RichText};
use egui::collapsing_header::CollapsingState;
use pico_core::config;
use pico_core::folders::{FolderRules, FolderState};
use pico_core::library::natural_cmp;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub enum FolderManagerAction {
    /// The edited rules, if they differ from the ones the dialog opened with.
    Apply(Option<FolderRules>),
    Cancel,
    /// Open the system folder dialog; its answer goes to [`FolderManager::pick`].
    Browse,
}

pub struct FolderManager {
    original: FolderRules,
    rules: FolderRules,
    roots: Vec<(String, PathBuf)>,
    selected: Option<PathBuf>,
    /// The path box; Enter or "Go" selects what is typed.
    path_text: String,
    /// Subfolders of every folder listed so far, read once per dialog.
    children: HashMap<PathBuf, Vec<PathBuf>>,
    /// A folder to open the tree to and scroll into view next frame.
    reveal: Option<PathBuf>,
}

impl FolderManager {
    pub fn new(rules: FolderRules) -> Self {
        let selected = rules.watched.first().cloned();
        let mut manager = Self {
            original: rules.clone(),
            rules,
            roots: roots(),
            path_text: String::new(),
            selected: None,
            children: HashMap::new(),
            reveal: None,
        };
        if let Some(path) = selected {
            manager.select(path);
        }
        manager
    }

    /// Takes a folder chosen elsewhere (the system dialog) into the library.
    pub fn pick(&mut self, path: PathBuf) {
        self.rules.set_in_library(&path, true);
        self.select(path);
    }

    fn select(&mut self, path: PathBuf) {
        self.path_text = path.display().to_string();
        self.reveal = Some(path.clone());
        self.selected = Some(path);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<FolderManagerAction> {
        let mut action = None;
        let mut open = true;
        crate::app::dialog_window(ctx, "Folder Manager").open(&mut open).resizable(false).show_dialog(ctx, |ui| {
            ui.label("Pick folders on the left and tell Pico whether to look inside them.");
            ui.separator();
            let size = egui::vec2(720.0, 340.0);
            ui.allocate_ui_with_layout(size, egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.set_min_size(size);
                ui.vertical(|ui| {
                    ui.set_width(400.0);
                    egui::ScrollArea::both().id_salt("folder tree").auto_shrink(false).show(ui, |ui| {
                        crate::app::list_rows(ui);
                        // The same folder can sit under several roots;
                        // open the tree to it under the first only.
                        let target = self.reveal.take();
                        let first = target.as_ref().and_then(|t| self.roots.iter().position(|(_, r)| t.starts_with(r)));
                        for (i, (label, path)) in self.roots.clone().into_iter().enumerate() {
                            self.reveal = if Some(i) == first { target.clone() } else { None };
                            self.node(ui, &path, label);
                        }
                        self.reveal = None;
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    if self.details(ui) {
                        action = Some(FolderManagerAction::Browse);
                    }
                });
            });
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(720.0, 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Watching {}, leaving out {}.",
                            count(self.rules.watched.len()),
                            count(self.rules.excluded.len())
                        ))
                        .weak(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            action = Some(FolderManagerAction::Cancel);
                        }
                        if ui.button("OK").clicked() {
                            let changed = self.rules != self.original;
                            action = Some(FolderManagerAction::Apply(changed.then(|| self.rules.clone())));
                        }
                    });
                },
            );
        });
        if !open {
            action = Some(FolderManagerAction::Cancel);
        }
        action
    }

    /// The right side: what happens to the selected folder. Returns whether
    /// "Browse…" was clicked.
    fn details(&mut self, ui: &mut egui::Ui) -> bool {
        ui.label("Folder path:");
        let mut go = false;
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut self.path_text).desired_width(250.0));
            go = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            go |= ui.button("Go").clicked();
        });
        let browse = ui.button("Browse…").on_hover_text("Choose with the system folder dialog").clicked();
        if go {
            let path = PathBuf::from(self.path_text.trim());
            if path.is_dir() {
                self.select(path);
            } else {
                self.selected = None;
            }
        }
        ui.add_space(12.0);

        let Some(path) = self.selected.clone() else {
            ui.label(RichText::new("Pick a folder on the left, or type its path.").weak());
            return browse;
        };
        ui.strong(display_name(&path));
        let state = self.rules.state(&path);
        let mut in_library = state.in_library();
        ui.radio_value(&mut in_library, true, "Watch for photos")
            .on_hover_text("Pico shows this folder and everything inside it, and notices new photos");
        ui.radio_value(&mut in_library, false, "Leave out of Pico")
            .on_hover_text("Pico ignores this folder; no files are touched");
        if in_library != state.in_library() {
            self.rules.set_in_library(&path, in_library);
        }
        let note = match self.rules.state(&path) {
            FolderState::Watched => "Watched, with everything inside it.",
            FolderState::InsideWatched => "Inside a watched folder, so it's in.",
            FolderState::Excluded => "Left out, along with everything inside it.",
            FolderState::InsideExcluded => "Inside a folder that's left out.",
            FolderState::Outside => "Not in Pico. Photos here are safe from being admired.",
        };
        ui.label(RichText::new(note).weak());
        browse
    }

    fn node(&mut self, ui: &mut egui::Ui, path: &Path, label: String) {
        let opens = self.reveal.as_deref().is_some_and(|r| r != path && r.starts_with(path));
        let reveal_here = self.reveal.as_deref() == Some(path);
        let header = |this: &mut Self, ui: &mut egui::Ui| {
            let (icon, color) = match this.rules.state(path) {
                FolderState::Watched => ("👁", Color32::from_rgb(90, 170, 90)),
                FolderState::InsideWatched => ("✔", Color32::from_rgb(90, 170, 90)),
                FolderState::Excluded | FolderState::InsideExcluded => ("✖", Color32::from_rgb(200, 90, 80)),
                FolderState::Outside => ("🗀", ui.visuals().weak_text_color()),
            };
            let selected = this.selected.as_deref() == Some(path);
            let response = ui
                .horizontal(|ui| {
                    ui.label(RichText::new(icon).color(color));
                    ui.selectable_label(selected, &label)
                })
                .inner;
            if response.clicked() {
                this.select(path.to_path_buf());
                this.reveal = None;
            }
            if reveal_here {
                response.scroll_to_me(Some(egui::Align::Center));
            }
        };
        let children = self.children_of(path);
        if children.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(ui.spacing().indent);
                header(self, ui);
            });
            return;
        }
        let id = ui.make_persistent_id(("folder-manager", path));
        let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, false);
        if opens {
            state.set_open(true);
        }
        state.show_header(ui, |ui| header(self, ui)).body(|ui| {
            for child in children {
                self.node(ui, &child, display_name(&child));
            }
        });
    }

    /// Subfolders worth listing: hidden ones only on the way to a folder the
    /// user picked or has a rule for.
    fn children_of(&mut self, path: &Path) -> Vec<PathBuf> {
        let all = self.children.entry(path.to_path_buf()).or_insert_with(|| list_dirs(path));
        let rules = &self.rules;
        let selected = self.selected.as_deref();
        all.iter()
            .filter(|child| {
                !display_name(child).starts_with('.')
                    || rules
                        .watched
                        .iter()
                        .chain(&rules.excluded)
                        .map(PathBuf::as_path)
                        .chain(selected)
                        .any(|p| p.starts_with(child))
            })
            .cloned()
            .collect()
    }
}

fn list_dirs(path: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort_by(|a, b| natural_cmp(&display_name(a), &display_name(b)));
    dirs
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

fn count(n: usize) -> String {
    if n == 1 { "1 folder".into() } else { format!("{n} folders") }
}

/// Where the tree starts: the user's folders first, then whole disks.
fn roots() -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();
    if let Some(pictures) = config::default_pictures_dir() {
        roots.push(("Pictures".to_owned(), pictures));
    }
    if let Some(home) = config::home_dir().filter(|h| h.is_dir()) {
        roots.push(("Home".to_owned(), home));
    }
    if cfg!(windows) {
        for letter in 'A'..='Z' {
            let drive = PathBuf::from(format!("{letter}:\\"));
            if drive.is_dir() {
                roots.push((format!("Drive {letter}:"), drive));
            }
        }
    } else {
        let mut disks = vec![PathBuf::from("/Volumes")];
        if let Ok(user) = std::env::var("USER") {
            disks.extend([Path::new("/media").join(&user), Path::new("/run/media").join(&user)]);
        }
        for dir in disks.into_iter().filter(|d| d.is_dir()) {
            roots.push(("Drives".to_owned(), dir));
        }
        roots.push(("Computer".to_owned(), PathBuf::from("/")));
    }
    roots
}
