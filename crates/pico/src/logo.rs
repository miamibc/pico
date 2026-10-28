//! What Help → About shows: the Pico logo (`docs/logo/pico.svg` and, with
//! the name and slogan, `docs/logo/pico-logo-with-title.svg`, drawn into PNGs
//! by `make logo`) and the licenses of the free software Pico is built on
//! (`assets/licenses.txt`, written by `make licenses`).

use eframe::egui;

const ICON: &[u8] = include_bytes!("../assets/pico-256.png");
const WITH_TITLE: &[u8] = include_bytes!("../assets/pico-logo-with-title.png");
const LICENSES: &str = include_str!("../assets/licenses.txt");

fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let image = pico_core::image::load_from_memory(png).expect("the logo is a PNG").into_rgba8();
    (image.width(), image.height(), image.into_raw())
}

/// For the window and the taskbar.
pub fn icon() -> egui::IconData {
    let (width, height, rgba) = decode(ICON);
    egui::IconData { rgba, width, height }
}

/// The logo with the name, for Help → About.
pub fn with_title(ctx: &egui::Context) -> egui::TextureHandle {
    let (width, height, rgba) = decode(WITH_TITLE);
    let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
    ctx.load_texture("pico-logo-with-title", image, egui::TextureOptions::LINEAR)
}

/// The license list and texts, scrolled; only the rows in view are laid out.
pub fn licenses(ui: &mut egui::Ui) {
    static LINES: std::sync::OnceLock<Vec<&str>> = std::sync::OnceLock::new();
    let lines = LINES.get_or_init(|| LICENSES.lines().collect());
    // Small enough for the 72-column license texts to fit the window.
    let font = egui::FontId::monospace(11.0);
    let row_height = ui.fonts_mut(|f| f.row_height(&font));
    egui::ScrollArea::both().max_height(220.0).min_scrolled_height(220.0).auto_shrink([false, true]).show_rows(
        ui,
        row_height,
        lines.len(),
        |ui, rows| {
            for line in &lines[rows] {
                ui.add(egui::Label::new(egui::RichText::new(*line).font(font.clone())).extend());
            }
        },
    );
}
