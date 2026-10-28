//! The Info side panel: file properties with EXIF, and the tag (keyword) editor.

use eframe::egui;
use pico_core::exif::{self, Exif};
use pico_core::library::MediaFile;
use pico_core::media::MediaKind;
use pico_core::metadata;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

/// A file's EXIF (if any), pixel size (if known), XMP properties and other
/// metadata by kind.
type Details = (Option<Exif>, Option<(u32, u32)>, Vec<(String, String)>, Vec<(&'static str, metadata::Rows)>);

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
    /// The Properties sections open (EXIF, XMP…); the rest are closed.
    /// Remembered between runs.
    pub open_sections: BTreeSet<String>,
    /// The Copy button was clicked; the properties copy themselves.
    copy: bool,
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
        // The heading and Copy stay put above the scrolling list.
        ui.horizontal(|ui| {
            ui.heading("Properties");
            if current.is_some() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // A plain icon that lights up as a button under the pointer.
                    let button = ui
                        .add(egui::Button::new("📋").frame_when_inactive(false))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Copy"));
                    let hint = "Copy: all the metadata as tab-separated values, to paste into a spreadsheet";
                    self.copy = button.on_hover_text(hint).clicked();
                });
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
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
        let (exif, dimensions, xmp, others) = self.details.entry(file.path.clone()).or_insert_with(|| {
            let exif = exif::read(&file.path);
            let dimensions =
                if file.kind == MediaKind::Photo { pico_core::image::image_dimensions(&file.path).ok() } else { None };
            (exif, dimensions, pico_core::xmp::read(&file.path), metadata::read(&file.path, file.kind))
        });
        let mut summary = vec![("Name".to_owned(), file.name())];
        if let Some(folder) = file.path.parent() {
            summary.push(("Folder".into(), folder.display().to_string()));
        }
        summary.push(("File size".into(), human_size(file.size)));
        if let Some((w, h)) = dimensions {
            summary.push(("Dimensions".into(), format!("{w} × {h}")));
        }
        if let Some(created) = file.created_text() {
            summary.push(("Created".into(), created));
        }
        summary.push(("Modified".into(), file.sort_date_if_modified()));
        summary.extend(
            exif.iter().flat_map(|e| &e.fields).filter(|(l, _)| *l != "GPS").map(|(l, v)| (l.to_string(), v.clone())),
        );
        // The position from wherever the file has one: EXIF, XMP, a video's
        // ISO 6709 tag. The first found is shown, linked to the map; where
        // they disagree, every one of them gets a warning mark.
        let mut positions: Vec<(&str, (f64, f64), String)> = Vec::new();
        if let Some(at) = exif.as_ref().and_then(|e| e.gps) {
            positions.push(("EXIF", at, String::new()));
        }
        if let Some(at) = pico_core::xmp::gps(xmp) {
            positions.push(("XMP", at, String::new()));
        }
        for (title, rows) in others.iter() {
            for (label, value) in rows {
                if let Some(at) = metadata::iso6709(value) {
                    positions.push((title, at, label.clone()));
                }
            }
        }
        let first = positions.first().map(|p| p.1);
        let far = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).abs() > 1e-4 || (a.1 - b.1).abs() > 1e-4;
        let disagree = first.is_some_and(|f| positions.iter().any(|p| far(p.1, f)));
        let where_from = disagree.then(|| {
            let each: Vec<String> =
                positions.iter().map(|(from, (lat, lon), _)| format!("{from}: {lat:.6}, {lon:.6}")).collect();
            format!("The positions differ:\n{}", each.join("\n"))
        });
        if let Some((lat, lon)) = first {
            summary.push(("GPS".into(), format!("{lat:.6}, {lon:.6}")));
        }
        let map = first.map(|(lat, lon)| {
            format!("https://www.openstreetmap.org/?mlat={lat:.6}&mlon={lon:.6}#map=15/{lat:.6}/{lon:.6}")
        });
        let is_coordinate = |label: &str| label == "GPS Latitude" || label == "GPS Longitude";
        table(ui, "properties", &summary, |label| Extra {
            link: map.clone().filter(|_| label == "GPS"),
            warning: where_from.clone().filter(|_| label == "GPS"),
        });
        let all_exif = exif.as_ref().map_or(&[][..], |e| e.all.as_slice());
        // Fields that hold the same fact in different kinds of metadata and
        // disagree; and, separately, positions that disagree.
        let mut sections: Vec<(&str, &[(String, String)])> = vec![("EXIF", all_exif), ("XMP", xmp.as_slice())];
        sections.extend(others.iter().map(|(title, rows)| (*title, rows.as_slice())));
        let differ = pico_core::agree::disagreements(&sections);
        let warning = |section: &str, label: &str| {
            let positioned = match section {
                "EXIF" | "XMP" => is_coordinate(label),
                _ => positions.iter().any(|(from, _, l)| *from == section && l == label),
            };
            differ
                .iter()
                .find(|(s, l, _)| s == section && l == label)
                .map(|(_, _, note)| note.clone())
                .or_else(|| where_from.clone().filter(|_| positioned))
        };
        for (title, rows) in &sections {
            if rows.is_empty() {
                continue;
            }
            // Open or closed as the user left it, also after a restart.
            let open = self.open_sections.contains(*title);
            let section = egui::CollapsingHeader::new(*title).open(Some(open)).show(ui, |ui| {
                table(ui, title, rows, |label| Extra { warning: warning(title, label), ..Extra::default() });
            });
            if section.header_response.clicked() {
                if open {
                    self.open_sections.remove(*title);
                } else {
                    self.open_sections.insert(title.to_string());
                }
            }
        }
        if std::mem::take(&mut self.copy) {
            let mut all = vec![("File", summary.as_slice())];
            all.extend(sections.iter().copied());
            ui.ctx().copy_text(tsv(&all));
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

/// What a row of a table gets besides its value, by its label.
#[derive(Default)]
struct Extra {
    /// A warning mark, saying this.
    warning: Option<String>,
    /// The value is a link to this address.
    link: Option<String>,
}

/// Label and value rows that wrap long text within the panel's width rather
/// than widen the panel, with the marks and buttons `extra` asks for.
fn table(ui: &mut egui::Ui, id: &str, rows: &[(String, String)], extra: impl Fn(&str) -> Extra) {
    let width = ui.available_width();
    let spacing = ui.spacing().item_spacing.x;
    let label_width = 120.0_f32.min(width * 0.45);
    egui::Grid::new(id)
        .num_columns(2)
        .striped(true)
        .min_col_width(label_width)
        .max_col_width((width - label_width - spacing).max(label_width))
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.add(egui::Label::new(egui::RichText::new(label).weak()).wrap());
                let Extra { warning, link } = extra(label);
                ui.horizontal_top(|ui| {
                    if let Some(note) = warning {
                        warning_mark(ui).on_hover_text(note);
                    }
                    match link {
                        Some(url) => {
                            // A link that wraps like the other values.
                            let text = egui::RichText::new(value).color(ui.visuals().hyperlink_color);
                            let link = ui.add(egui::Label::new(text).wrap().sense(egui::Sense::click()));
                            link.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, value));
                            let link = link.on_hover_cursor(egui::CursorIcon::PointingHand);
                            if link.on_hover_text("Show on OpenStreetMap").clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                            }
                        }
                        None => {
                            ui.add(egui::Label::new(value).wrap());
                        }
                    }
                });
                ui.end_row();
            }
        });
}

/// Section, field and value columns under a header row; tabs and line
/// breaks inside values become spaces.
fn tsv(sections: &[(&str, &[(String, String)])]) -> String {
    let clean = |s: &str| s.replace(['\t', '\r', '\n'], " ");
    let mut out = String::from("Section\tField\tValue\n");
    for (section, rows) in sections {
        for (label, value) in *rows {
            out.push_str(&format!("{section}\t{}\t{}\n", clean(label), clean(value)));
        }
    }
    out
}

/// An exclamation mark in a circle.
fn warning_mark(ui: &mut egui::Ui) -> egui::Response {
    let size = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let color = ui.visuals().warn_fg_color;
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size * 0.45, color);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "!",
        egui::FontId::proportional(size * 0.8),
        ui.visuals().panel_fill,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Differs"));
    response
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
    /// When the file was created, where the system keeps that.
    fn created_text(&self) -> Option<String>;
}

impl ModifiedText for MediaFile {
    /// The file's modification time in the same layout as EXIF dates.
    fn sort_date_if_modified(&self) -> String {
        let secs = self.modified.and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        pico_core::fileops::exif_style_date(secs) + " UTC"
    }

    fn created_text(&self) -> Option<String> {
        let created = std::fs::metadata(&self.path).ok()?.created().ok()?;
        let secs = created.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        Some(pico_core::fileops::exif_style_date(secs) + " UTC")
    }
}
