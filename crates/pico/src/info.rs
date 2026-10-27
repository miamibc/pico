//! The Info side panel: file properties with EXIF, and the tag (keyword) editor.

use eframe::egui;
use pico_core::exif::{self, Exif};
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// A file's EXIF (if any) and pixel size (if known).
type Details = (Option<Exif>, Option<(u32, u32)>);

pub enum TagAction {
    Add(String),
    Remove(String),
}

#[derive(Default)]
pub struct InfoPanel {
    /// EXIF and pixel size per file, read on first display.
    details: HashMap<PathBuf, Details>,
    new_tag: String,
    pub focus_tag_field: bool,
}

impl InfoPanel {
    /// `current` gets the properties; `targets` share the tag editor.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        current: Option<&MediaFile>,
        targets: &[&MediaFile],
    ) -> Option<TagAction> {
        let mut action = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Properties");
            match current {
                Some(file) => self.properties(ui, file),
                None => {
                    ui.label(egui::RichText::new("Pick a photo to see its secrets.").weak());
                }
            }
            ui.separator();
            ui.heading("Tags");
            action = self.tags(ui, targets);
        });
        action
    }

    fn properties(&mut self, ui: &mut egui::Ui, file: &MediaFile) {
        let (exif, dimensions) = self.details.entry(file.path.clone()).or_insert_with(|| {
            let exif = exif::read(&file.path);
            let dimensions =
                if file.kind == MediaKind::Photo { pico_core::image::image_dimensions(&file.path).ok() } else { None };
            (exif, dimensions)
        });
        egui::Grid::new("properties").num_columns(2).striped(true).show(ui, |ui| {
            let mut row = |label: &str, value: String| {
                ui.label(egui::RichText::new(label).weak());
                ui.label(value);
                ui.end_row();
            };
            row("Name", file.name());
            if let Some(folder) = file.path.parent() {
                row("Folder", folder.display().to_string());
            }
            row("File size", human_size(file.size));
            if let Some((w, h)) = dimensions {
                row("Dimensions", format!("{w} × {h}"));
            }
            row("Modified", file.sort_date_if_modified());
            for (label, value) in exif.iter().flat_map(|e| &e.fields) {
                row(label, value.clone());
            }
        });
        if let Some((lat, lon)) = exif.as_ref().and_then(|e| e.gps)
            && ui.button("Show on OpenStreetMap").on_hover_text("Opens your web browser").clicked()
        {
            let url = format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=15/{lat:.6}/{lon:.6}");
            let _ = pico_core::fileops::open_external(std::path::Path::new(&url));
        }
        if exif.is_none() && file.kind == MediaKind::Photo {
            ui.label(egui::RichText::new("No camera details in this file.").weak());
        }
    }

    fn tags(&mut self, ui: &mut egui::Ui, targets: &[&MediaFile]) -> Option<TagAction> {
        if targets.is_empty() {
            ui.label(egui::RichText::new("Select photos to tag them.").weak());
            return None;
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for file in targets {
            for tag in &file.meta.keywords {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }
        let mut action = None;
        ui.horizontal(|ui| {
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.new_tag)
                    .hint_text(if targets.len() > 1 { "Tag all selected…" } else { "Add a tag…" })
                    .desired_width(160.0),
            );
            if self.focus_tag_field {
                field.request_focus();
                self.focus_tag_field = false;
            }
            // Take the Enter so it does not also open the photo behind the panel.
            let enter = field.lost_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            if (ui.button("Add").clicked() || enter) && !self.new_tag.trim().is_empty() {
                // Picasa stores keywords comma-separated, so commas cannot be part of one.
                action = Some(TagAction::Add(self.new_tag.trim().replace(',', " ")));
                self.new_tag.clear();
            }
        });
        if counts.is_empty() {
            ui.label(egui::RichText::new("No tags yet.").weak());
        }
        ui.horizontal_wrapped(|ui| {
            for (tag, count) in &counts {
                let label = if targets.len() > 1 {
                    format!("{tag} ({count}/{}) ×", targets.len())
                } else {
                    format!("{tag} ×")
                };
                if ui.button(label).on_hover_text("Remove this tag").clicked() {
                    action = Some(TagAction::Remove(tag.clone()));
                }
            }
        });
        action
    }
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 => format!("{:.1} GB", b as f64 / (1u64 << 30) as f64),
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} bytes"),
    }
}

trait ModifiedText {
    fn sort_date_if_modified(&self) -> String;
}

impl ModifiedText for MediaFile {
    /// The file's modification time in the same layout as EXIF dates.
    fn sort_date_if_modified(&self) -> String {
        let secs = self.modified.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        pico_core::fileops::exif_style_date(secs) + " UTC"
    }
}
