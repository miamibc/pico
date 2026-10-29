#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod app;
mod collage_view;
mod filmstrip;
mod folder_manager;
mod grid;
mod info;
mod logo;
mod player;
mod slideshow;
mod star;
mod system_theme;
#[cfg(test)]
mod tests;
mod thumbs;
mod tools;
mod viewer;

use std::path::PathBuf;

fn main() -> eframe::Result {
    // Folders given on the command line are watched in addition to saved ones.
    let extra_folders: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    // The window comes back at the size it was closed at.
    let settings = pico_core::config::load_settings();
    // The interface speaks the chosen language, else the system's.
    i18n::set_language(settings.get("language").map(String::as_str).filter(|l| *l != "auto"));
    let number = |key: &str, default: f32| settings.get(key).and_then(|v| v.parse::<f32>().ok()).unwrap_or(default);
    let size = [number("window_width", 1200.0).max(640.0), number("window_height", 800.0).max(400.0)];
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Pico")
            .with_icon(logo::icon())
            .with_inner_size(size)
            .with_min_inner_size([640.0, 400.0])
            .with_maximized(settings.get("maximized").is_some_and(|v| v == "true")),
        ..Default::default()
    };
    eframe::run_native(
        "Pico",
        options,
        Box::new(|cc| {
            let watched = app::PicoApp::startup_folders(extra_folders);
            Ok(Box::new(app::PicoApp::new(&cc.egui_ctx, watched, true)))
        }),
    )
}
