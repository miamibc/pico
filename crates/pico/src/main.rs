#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod collage_view;
mod filmstrip;
mod folder_manager;
mod grid;
mod info;
mod player;
mod slideshow;
#[cfg(test)]
mod tests;
mod thumbs;
mod tools;
mod viewer;

use std::path::PathBuf;

fn main() -> eframe::Result {
    // Folders given on the command line are watched in addition to saved ones.
    let extra_folders: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Pico")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
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
