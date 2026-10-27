//! UI tests: drive the real app headlessly with egui_kittest and check what
//! ends up in `.picasa.ini` on disk.

use crate::app::PicoApp;
use eframe::egui::{self, Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use pico_core::image::{Rgb, RgbImage};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// UI tests render with wgpu; several GPU devices in one process at once
/// crash some drivers, so UI tests take turns.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_turn() -> std::sync::MutexGuard<'static, ()> {
    // A failed test poisons the lock; the next one may still run.
    GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Fixture {
    dir: PathBuf,
    _gpu: std::sync::MutexGuard<'static, ()>,
}

impl Fixture {
    /// A folder with three small photos and an INI holding a key Pico must keep.
    fn new(name: &str) -> Self {
        let gpu = gpu_turn();
        let dir = std::env::temp_dir().join(format!("pico-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (file, color) in [("a.png", [200, 40, 40]), ("b.png", [40, 200, 40]), ("c.png", [40, 40, 200])] {
            RgbImage::from_pixel(120, 80, Rgb(color)).save(dir.join(file)).unwrap();
        }
        std::fs::write(dir.join(".picasa.ini"), "[Picasa]\r\nname=Holiday\r\n[b.png]\r\nbackuphash=4242\r\n").unwrap();
        Self { dir, _gpu: gpu }
    }

    fn ini(&self) -> String {
        std::fs::read_to_string(self.dir.join(".picasa.ini")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn harness(dir: &Path) -> Harness<'static, PicoApp> {
    let dir = dir.to_path_buf();
    Harness::builder().with_size([1000.0, 800.0]).build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false))
}

/// Steps the UI until background work (scanning, decoding) satisfies `done`.
fn wait_until(harness: &mut Harness<'_, PicoApp>, what: &str, done: impl Fn(&PicoApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done(harness.state()) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    harness.step();
}

/// Saves a screenshot for eyeballing; skipped where no GPU adapter exists.
fn screenshot(harness: &mut Harness<'_, PicoApp>, name: &str) {
    let Ok(image) = harness.render() else { return };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
fn star_rotate_and_caption_are_saved_to_ini() {
    let fixture = Fixture::new("viewer");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    assert!(harness.state().selected_file().is_none(), "nothing is selected on startup");
    screenshot(&mut harness, "library");

    harness.get_by_label("b.png").click();
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(1));

    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());

    harness.key_press(Key::S);
    harness.step();
    assert!(fixture.ini().contains("[b.png]\r\nbackuphash=4242\r\nstar=yes\r\n"), "{}", fixture.ini());

    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    harness.step();
    assert!(fixture.ini().contains("rotate=rotate(1)\r\n"), "{}", fixture.ini());
    wait_until(&mut harness, "the rotated photo", |app| app.viewer_loaded());

    let caption = harness.get_by_role(egui::Role::TextInput);
    caption.focus();
    caption.type_text("Sunny day");
    harness.step();
    harness.key_press(Key::Enter);
    harness.step();
    assert!(fixture.ini().contains("caption=Sunny day\r\n"), "{}", fixture.ini());
    screenshot(&mut harness, "viewer");

    // Ctrl+Shift+R undoes the rotation; an unrotated photo has no rotate key.
    harness.get_by_label("⟲").click();
    harness.step();
    assert!(!fixture.ini().contains("rotate="), "{}", fixture.ini());

    harness.key_press(Key::Escape);
    harness.step();
    assert!(!harness.state().is_viewing());
    assert!(fixture.ini().starts_with("[Picasa]\r\nname=Holiday\r\n"), "{}", fixture.ini());
}

#[test]
fn double_click_opens_and_closes_the_photo() {
    let fixture = Fixture::new("double-click");
    // Each queued event takes a frame; at the default 0.25 s a frame, two
    // clicks are too slow to count as a double click.
    let dir = fixture.dir.clone();
    let mut harness = Harness::builder()
        .with_size([1000.0, 800.0])
        .with_step_dt(0.05)
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    double_click(&mut harness, "b.png");
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());
    double_click(&mut harness, "Photo");
    harness.step();
    assert!(!harness.state().is_viewing(), "double-clicking the photo goes back to the library");

    // Beside the photo, on the dark backdrop, works the same.
    double_click(&mut harness, "b.png");
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());
    let photo = harness.get_by_label("Photo").rect();
    let beside = egui::pos2(photo.min.x - 20.0, photo.center().y);
    double_click_at(&mut harness, beside);
    harness.step();
    assert!(!harness.state().is_viewing());

    // While a tool is open, double-clicks belong to the tool.
    double_click(&mut harness, "b.png");
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());
    harness.state_mut().open_edit_panel();
    harness.step();
    harness.get_by_label("Crop").click();
    harness.step();
    double_click(&mut harness, "Photo");
    harness.step();
    assert!(harness.state().is_viewing());
}

fn double_click(harness: &mut Harness<'_, PicoApp>, label: &str) {
    let pos = harness.get_by_label(label).rect().center();
    double_click_at(harness, pos);
}

fn double_click_at(harness: &mut Harness<'_, PicoApp>, pos: egui::Pos2) {
    // Pause first, or egui takes it for a triple click after the last one.
    for _ in 0..10 {
        harness.step();
    }
    harness.hover_at(pos);
    harness.step();
    for pressed in [true, false, true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        });
    }
    harness.step(); // one frame per event
}

#[test]
fn filmstrip_switches_photos() {
    let fixture = Fixture::new("filmstrip");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    harness.get_by_label("a.png").click();
    harness.step();
    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());
    // The strip lists the folder; only the open photo is marked.
    assert!(marked(&harness, "a.png"));
    assert!(!marked(&harness, "c.png"));
    wait_until(&mut harness, "the strip's thumbnails", |app| app.thumbnails_pending() == 0);
    screenshot(&mut harness, "filmstrip");

    harness.get_by_label("c.png").click();
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(2));
    assert!(harness.state().is_viewing(), "the strip switches photos, it does not close the viewer");
    harness.step();
    assert!(marked(&harness, "c.png"));

    harness.state_mut().set_filmstrip(false);
    harness.step();
    assert!(harness.query_by_label("a.png").is_none(), "no strip, no thumbnails");
}

/// Whether a thumbnail button shows as the current one.
fn marked(harness: &Harness<'_, PicoApp>, label: &str) -> bool {
    harness.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn zoom_pan_and_wheel_in_viewer() {
    let fixture = Fixture::new("zoom");
    // Wider than the working copy, so 1:1 needs the full-resolution picture.
    RgbImage::from_fn(3000, 2000, |x, y| Rgb([(x % 256) as u8, (y % 256) as u8, 90]))
        .save(fixture.dir.join("big.png"))
        .unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("a.png").click();
    harness.step();
    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());

    // Pinch or Ctrl+wheel zooms; a small photo fits at 100%, so zoom in.
    let photo = harness.get_by_label("Photo").rect();
    harness.hover_at(photo.center());
    harness.event(egui::Event::Zoom(2.0));
    harness.step();
    assert_eq!(harness.state().zoom_state().0, Some(200.0));
    harness.event(egui::Event::Zoom(10.0));
    harness.step();
    assert_eq!(harness.state().zoom_state().0, Some(800.0), "zoom stops at 800%");
    // Dragging looks around the zoomed photo.
    let start = harness.get_by_label("Photo").rect().center();
    harness.drag_at(start);
    harness.step();
    harness.hover_at(start + egui::vec2(30.0, 20.0));
    harness.step();
    harness.drop_at(start + egui::vec2(30.0, 20.0));
    harness.step();
    assert_ne!(harness.state().zoom_state().1, egui::Vec2::ZERO, "the photo moved");
    harness.get_by_label("Fit").click();
    harness.step();
    assert_eq!(harness.state().zoom_state(), (None, egui::Vec2::ZERO, false));

    // The plain wheel turns pages, like in Picasa.
    harness.hover_at(photo.center());
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, -3.0),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    });
    for _ in 0..10 {
        harness.step();
    }
    assert_eq!(harness.state().selected_file(), Some(1), "one notch down, one photo on");

    // At 1:1 a big photo is shown from its full-resolution picture.
    harness.state_mut().open_file_named("big.png");
    wait_until(&mut harness, "the big photo", |app| app.viewer_loaded());
    harness.get_by_label("1:1").click();
    harness.step();
    assert_eq!(harness.state().zoom_state().0, Some(100.0));
    wait_until(&mut harness, "the full-resolution picture", |app| app.zoom_state().2);
    screenshot(&mut harness, "zoom");
}

#[test]
fn info_line_above_the_photo() {
    let fixture = Fixture::new("info-line");
    let ini = fixture.ini() + "[c.png]\r\nfilters=crop64=1,7fffffff;\r\n";
    std::fs::write(fixture.dir.join(".picasa.ini"), ini).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("a.png").click();
    harness.step();
    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());
    harness.step();
    let line = info_line(&harness, "a.png");
    assert!(line.contains("120 × 80"), "{line}");
    assert!(line.contains("file date"), "no EXIF, so the file's own date: {line}");
    assert!(line.ends_with(" bytes") || line.ends_with(" KB"), "{line}");

    // Turned sideways the photo is 80 wide; that is not a new size.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    harness.step(); // the key press starts the new render
    wait_until(&mut harness, "the rotated photo", |app| app.viewer_loaded());
    harness.step();
    let line = info_line(&harness, "a.png");
    assert!(line.contains("80 × 120") && !line.contains("original"), "{line}");
    screenshot(&mut harness, "info-line");

    // c.png was cropped to its left half in Picasa.
    harness.key_press(Key::ArrowRight);
    harness.key_press(Key::ArrowRight);
    wait_until(&mut harness, "the cropped photo", |app| app.selected_file() == Some(2) && app.viewer_loaded());
    harness.step();
    let line = info_line(&harness, "c.png");
    assert!(line.contains("60 × 80 (original 120 × 80)"), "{line}");
}

fn info_line(harness: &Harness<'_, PicoApp>, name: &str) -> String {
    let prefix = format!("{name}   ·");
    harness
        .get_by(|n| n.role() == egui::accesskit::Role::Label && n.value().is_some_and(|v| v.starts_with(&prefix)))
        .accesskit_node()
        .value()
        .unwrap_or_default()
}

#[test]
fn hold_original_to_compare() {
    let fixture = Fixture::new("original");
    let ini = fixture.ini() + "[c.png]\r\nfilters=bw=1;\r\n";
    std::fs::write(fixture.dir.join(".picasa.ini"), ini).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    harness.state_mut().open_file_named("a.png");
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());
    assert!(harness.get_by_label("Original").accesskit_node().is_disabled(), "nothing to compare without edits");

    harness.state_mut().open_file_named("c.png");
    wait_until(&mut harness, "the edited photo and its original", |app| {
        app.viewer_loaded() && app.showing_original().0
    });
    let button = harness.get_by_label("Original").rect().center();
    harness.drag_at(button);
    harness.step();
    harness.step();
    assert_eq!(harness.state().showing_original(), (true, true), "held down");
    screenshot(&mut harness, "original");
    harness.drop_at(button);
    harness.step();
    harness.step();
    assert_eq!(harness.state().showing_original(), (true, false), "let go");

    harness.key_down(Key::Backslash);
    harness.step();
    assert!(harness.state().showing_original().1, "the backslash key works too");
    harness.key_up(Key::Backslash);
    harness.step();
    assert!(!harness.state().showing_original().1);
}

#[test]
fn histogram_and_camera_over_the_photo() {
    let fixture = Fixture::new("histogram");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.state_mut().open_file_named("a.png");
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());
    harness.step();
    assert!(harness.query_by_label("Histogram").is_none(), "hidden until asked for");
    harness.get_by_label("Info").click();
    harness.step();
    harness.step();
    harness.get_by_label("Histogram");
    harness.get_by_label("No camera details. A screenshot, perhaps?");
    screenshot(&mut harness, "histogram");
}

#[test]
fn undo_and_redo_edits() {
    let fixture = Fixture::new("undo-redo");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.state_mut().open_file_named("a.png");
    harness.state_mut().open_edit_panel();
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());
    let filters = |fixture: &Fixture| {
        let ini = fixture.ini();
        ini.split("[a.png]").nth(1).and_then(|s| s.lines().find(|l| l.starts_with("filters=")).map(str::to_owned))
    };
    harness.get_by_label("B&W").click();
    harness.step();
    harness.get_by_label("Sepia").click();
    harness.step();
    assert_eq!(filters(&fixture).as_deref(), Some("filters=bw=1;sepia=1;"));

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    harness.step();
    assert_eq!(filters(&fixture).as_deref(), Some("filters=bw=1;"));
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    harness.step();
    assert_eq!(filters(&fixture).as_deref(), Some("filters=bw=1;sepia=1;"));

    harness.get_by_label("Undo All Edits").click();
    harness.step();
    assert_eq!(filters(&fixture), None);
    harness.step(); // Redo is enabled from the next frame on
    harness.get_by_label("Redo").click();
    harness.step();
    assert_eq!(filters(&fixture).as_deref(), Some("filters=bw=1;"), "redo brings edits back oldest first");

    // A new edit ends the redo history.
    harness.get_by_label("Warmify").click();
    harness.step();
    harness.step();
    assert!(harness.get_by_label("Redo").accesskit_node().is_disabled());
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    harness.step();
    assert_eq!(filters(&fixture).as_deref(), Some("filters=bw=1;warm=1;"));
}

#[test]
fn home_end_and_full_screen_in_viewer() {
    let fixture = Fixture::new("full-screen");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.state_mut().open_file_named("b.png");
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());

    harness.key_press(Key::End);
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(2));
    harness.key_press(Key::Home);
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(0));

    // F11: only the photo; Esc leaves full screen but keeps the photo open.
    harness.key_press(Key::F11);
    harness.step();
    harness.step();
    assert!(harness.state().is_full_screen());
    assert!(harness.query_by_label("File").is_none(), "no menu bar");
    assert!(harness.query_by_label("⬅ Back to Library").is_none(), "no toolbar");
    wait_until(&mut harness, "the first photo", |app| app.viewer_loaded());
    harness.get_by_label("Photo");
    screenshot(&mut harness, "full-screen");
    harness.key_press(Key::ArrowRight);
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(1), "arrows still work");
    harness.key_press(Key::Escape);
    harness.step();
    harness.step();
    assert!(!harness.state().is_full_screen());
    assert!(harness.state().is_viewing());
    harness.get_by_label("File");

    harness.get_by_label("⛶").click();
    harness.step();
    harness.step();
    assert!(harness.state().is_full_screen());
    harness.key_press(Key::F11);
    harness.step();
    assert!(!harness.state().is_full_screen(), "F11 toggles");
}

#[test]
fn a_star_flies_onto_a_photo_when_starred() {
    let fixture = Fixture::new("star-flight");
    let dir = fixture.dir.clone();
    let mut harness = Harness::builder()
        .with_size([1000.0, 800.0])
        .with_step_dt(0.05)
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    double_click(&mut harness, "b.png");
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());

    harness.get_by_label("☆ Star").click();
    for frame in 1..=30 {
        harness.step();
        if [4, 6, 8, 14].contains(&frame) {
            assert!(harness.state().star_flying(), "frame {frame}");
            screenshot(&mut harness, &format!("star-flight-{frame:02}"));
        }
    }
    assert!(!harness.state().star_flying(), "the star has landed");
    assert!(fixture.ini().contains("star=yes"), "{}", fixture.ini());

    // Unstarring makes the badge vanish in smoke, without a flight.
    harness.get_by_label("★ Starred").click();
    harness.step();
    assert!(!harness.state().star_flying());
    assert!(harness.state().star_puffing());
    harness.step();
    harness.step();
    screenshot(&mut harness, "star-puff");
    for _ in 0..20 {
        harness.step();
    }
    assert!(!harness.state().star_puffing(), "the smoke has cleared");

    // Moving to another photo mid-flight drops the flight.
    harness.get_by_label("☆ Star").click();
    harness.step();
    assert!(harness.state().star_flying());
    harness.key_press(Key::ArrowRight);
    harness.step();
    assert!(!harness.state().star_flying(), "the flight belongs to the photo before");
}

#[test]
fn context_menus_on_photos_and_folders() {
    let fixture = Fixture::new("context-menu");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    // Right-click on an unselected photo works on that photo alone.
    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("b.png").click_secondary();
    harness.step();
    harness.step();
    harness.get_by_label("View and Edit");
    harness.get_by_label("Star").click();
    harness.step();
    assert!(fixture.ini().contains("[b.png]\r\nbackuphash=4242\r\nstar=yes\r\n"), "{}", fixture.ini());
    assert!(!fixture.ini().contains("[a.png]"), "{}", fixture.ini());

    // On a selection of two it works on both, without single-photo commands.
    harness.get_by_label("a.png").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.get_by_label("b.png").click_secondary();
    harness.step();
    harness.step();
    assert!(harness.query_by_label("View and Edit").is_none());
    screenshot(&mut harness, "context-menu");
    harness.get_by_label("Star").click();
    harness.step();
    assert!(fixture.ini().contains("[a.png]\r\nstar=yes\r\n"), "{}", fixture.ini());

    harness.get_by_label("Holiday  (3)").click_secondary();
    harness.step();
    harness.step();
    harness.get_by_label("Remove from Pico").click();
    harness.step();
    assert_eq!(harness.state().watched_count(), 0);
    wait_until(&mut harness, "the rescan", |app| app.folder_count() == 0);
    assert!(fixture.dir.join("a.png").is_file());
}

#[test]
fn photo_tray_holds_photos_across_folders() {
    let fixture = Fixture::new("tray");
    std::fs::create_dir_all(fixture.dir.join("more")).unwrap();
    RgbImage::from_pixel(60, 40, Rgb([200, 200, 0])).save(fixture.dir.join("more/d.png")).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 2);
    harness.get_by_label("Holiday  (3)").click();
    harness.step();

    harness.get_by_label("a.png").click();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::H);
    harness.step();
    assert_eq!(harness.state().tray_len(), 1);
    harness.get_by_label("more  (1)").click();
    harness.step();
    harness.get_by_label("d.png").click();
    harness.step();
    harness.step(); // the tray, drawn before the grid, sees the selection next frame
    harness.get_by_label("Hold").click();
    harness.step();
    assert_eq!(harness.state().tray_len(), 2, "held photos stay when the folder changes");
    harness.step();
    harness.get_by_label("Unhold").click();
    harness.step();
    assert_eq!(harness.state().tray_len(), 1, "the selected photo leaves the tray");
    harness.step();
    harness.get_by_label("Hold").click();
    harness.step();
    assert_eq!(harness.state().tray_len(), 2);
    harness.step();

    harness.get_by_label("Photo Tray  (2)").click();
    harness.step();
    assert_eq!(harness.state().listing_names(), ["a.png", "d.png"]);
    wait_until(&mut harness, "the thumbnails", |app| app.thumbnails_pending() == 0);
    screenshot(&mut harness, "tray");

    harness.get_by_label("Export…").click();
    harness.step();
    harness.step();
    harness.get_by_label("Export 2 pictures");
    harness.key_press(Key::Escape);
    harness.step();

    harness.get_by_label("Clear").click();
    harness.step();
    assert_eq!(harness.state().tray_len(), 0);
    assert!(fixture.dir.join("more/d.png").is_file());
}

#[test]
fn tray_thumbnails_open_the_viewer_and_unhold() {
    let fixture = Fixture::new("tray-menu");
    // Short frames, so two clicks count as a double click.
    let dir = fixture.dir.clone();
    let mut harness = Harness::builder()
        .with_size([1000.0, 800.0])
        .with_step_dt(0.05)
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("c.png").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::H);
    harness.step();
    assert_eq!(harness.state().tray_len(), 2);

    double_click(&mut harness, "Tray: c.png");
    wait_until(&mut harness, "the photo to load", |app| app.is_viewing() && app.viewer_loaded());
    assert_eq!(harness.state().selected_file(), Some(2), "c.png");
    assert_eq!(harness.state().listing_names(), ["a.png", "c.png"], "the viewer steps through the tray");
    double_click(&mut harness, "Photo");
    harness.step();
    assert!(!harness.state().is_viewing());

    // A photo picked in the tray can be unheld. Pause first, or the click
    // counts as a double click with the last one.
    for _ in 0..10 {
        harness.step();
    }
    harness.get_by_label("Tray: a.png").click();
    harness.step();
    harness.get_by_label("Unhold").click();
    harness.step();
    assert_eq!(harness.state().tray_len(), 1);
    harness.step();
    assert!(harness.query_by_label("Tray: a.png").is_none(), "a.png left the tray");

    // The selection not held yet joins the tray on the right; its menu holds it.
    harness.get_by_label("Holiday  (3)").click();
    harness.step();
    harness.get_by_label("b.png").click();
    harness.step();
    harness.step();
    harness.get_by_label("Tray: c.png");
    wait_until(&mut harness, "the thumbnails", |app| app.thumbnails_pending() == 0);
    screenshot(&mut harness, "tray-held");
    // The tray's buttons work on all of it, held or not.
    harness.get_by_label("Export…").click();
    harness.step();
    harness.step();
    harness.get_by_label("Export 2 pictures");
    harness.key_press(Key::Escape);
    harness.step();
    harness.step();
    harness.get_by_label("Tray: b.png").click_secondary();
    harness.step();
    harness.step();
    // The menu item, drawn after the tray's own Hold button.
    harness.get_all_by_label("Hold").last().unwrap().click();
    harness.step();
    assert_eq!(harness.state().tray_len(), 2, "b.png is held now");
}

#[test]
fn thumbnail_size_by_slider_and_ctrl_wheel() {
    let fixture = Fixture::new("thumb-size");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    assert_eq!(harness.state().thumbnail_size(), 160.0);
    harness.hover_at(harness.get_by_label("b.png").rect().center());
    harness.event(egui::Event::Zoom(1.25));
    harness.step();
    assert_eq!(harness.state().thumbnail_size(), 200.0);
    harness.event(egui::Event::Zoom(4.0));
    harness.step();
    assert_eq!(harness.state().thumbnail_size(), 256.0, "no bigger than the largest size");
    harness.get_by_label("Thumbnail size (Ctrl+wheel over the photos)");
    screenshot(&mut harness, "thumb-size");
}

#[test]
fn drag_a_rectangle_to_select() {
    let fixture = Fixture::new("band");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    let (b, c) = (harness.get_by_label("b.png").rect(), harness.get_by_label("c.png").rect());

    // From the empty space below c, up and left into b.
    let start = c.right_bottom() + egui::vec2(10.0, 30.0);
    harness.drag_at(start);
    harness.step();
    harness.hover_at(start - egui::vec2(20.0, 20.0));
    harness.step();
    harness.hover_at(b.center());
    harness.step();
    screenshot(&mut harness, "band");
    harness.drop_at(b.center());
    harness.step();
    assert_eq!(harness.state().selected_count(), 2);

    // Ctrl adds the next band to the selection.
    let a = harness.get_by_label("a.png").rect();
    let start = a.left_bottom() + egui::vec2(-2.0, 30.0);
    harness.event(egui::Event::ModifiersChanged(Modifiers::COMMAND));
    harness.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::COMMAND,
    });
    harness.step();
    harness.event(egui::Event::PointerMoved(start + egui::vec2(10.0, -10.0)));
    harness.step();
    harness.event(egui::Event::PointerMoved(a.center()));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: a.center(),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::COMMAND,
    });
    harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    harness.step();
    assert_eq!(harness.state().selected_count(), 3);
}

#[test]
fn drag_photos_to_folders_and_the_tray_and_drop_folders_in() {
    let fixture = Fixture::new("drag-drop");
    std::fs::create_dir_all(fixture.dir.join("more")).unwrap();
    RgbImage::from_pixel(60, 40, Rgb([200, 200, 0])).save(fixture.dir.join("more/d.png")).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 2);
    harness.get_by_label("Holiday  (3)").click();
    harness.step();

    let drag = |harness: &mut Harness<'_, PicoApp>, from: &str, to: egui::Pos2| {
        let start = harness.get_by_label(from).rect().center();
        harness.drag_at(start);
        harness.step();
        harness.hover_at(start + egui::vec2(15.0, 15.0));
        harness.step();
        harness.hover_at(to);
        harness.step();
        screenshot(harness, "drag");
        harness.drop_at(to);
        harness.step();
        harness.step();
    };
    let folder = harness.get_by_label("more  (1)").rect().center();
    drag(&mut harness, "b.png", folder);
    assert!(harness.state().confirming_move());
    harness.get_by_label("Move").click();
    harness.step();
    wait_until(&mut harness, "the rescan", |app| !app.confirming_move() && app.folder_count() == 2);
    assert!(fixture.dir.join("more/b.png").is_file() && !fixture.dir.join("b.png").exists());
    let moved_ini = std::fs::read_to_string(fixture.dir.join("more/.picasa.ini")).unwrap();
    assert!(moved_ini.contains("[b.png]\r\nbackuphash=4242"), "the metadata went along: {moved_ini}");

    harness.get_by_label("Holiday  (2)").click();
    harness.step();
    let tray = harness.get_by_label("Photo Tray").rect().center();
    drag(&mut harness, "c.png", tray);
    assert_eq!(harness.state().tray_len(), 1);

    // A folder dropped from the file manager joins the library.
    let outside = fixture.dir.with_file_name(format!("pico-ui-drop-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    RgbImage::from_pixel(30, 30, Rgb([9, 9, 9])).save(outside.join("e.png")).unwrap();
    harness.input_mut().dropped_files.push(std::sync::Arc::new(TestDrop(outside.clone())));
    harness.step();
    assert_eq!(harness.state().watched_count(), 2);
    wait_until(&mut harness, "the rescan", |app| app.folder_count() == 3);
    std::fs::remove_dir_all(&outside).unwrap();
}

#[derive(Debug)]
struct TestDrop(PathBuf);

impl egui::DroppedFile for TestDrop {
    fn path(&self) -> &Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

#[test]
fn ribbon_of_folders_follows_the_list_and_the_scroll() {
    let fixture = Fixture::new("ribbon");
    for n in 1..=5 {
        let dir = fixture.dir.join(format!("trip {n}"));
        std::fs::create_dir_all(&dir).unwrap();
        for k in 0..4 {
            RgbImage::from_pixel(60, 40, Rgb([40 * n, 30 * k, 90])).save(dir.join(format!("t{n}-{k}.png"))).unwrap();
        }
    }
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 6);
    harness.get_by_label("Holiday  (3)").click();
    harness.step();
    // Every folder is in the grid, each under its header.
    harness.get_by_label("Holiday, 3 photos  ·  October 2026");
    harness.get_by_label("t1-0.png");

    harness.get_by_label("trip 2  (4)").click();
    harness.step();
    harness.step();
    let header = harness.get_by(|n| n.label().is_some_and(|l| l.starts_with("trip 2, "))).rect();
    let first = harness.get_by_label("t2-0.png").rect();
    assert!(first.top() > header.top() && first.top() < header.top() + 80.0, "{header:?} {first:?}");
    assert!(header.top() < 120.0, "scrolled to the top: {header:?}");
    assert_eq!(harness.state().current_folder_name().as_deref(), Some("trip 2"));
    screenshot(&mut harness, "ribbon");

    // Scrolling up by hand moves the highlight to the folder at the top.
    harness.hover_at(first.center());
    for _ in 0..3 {
        harness.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 400.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::NONE,
        });
        for _ in 0..6 {
            harness.step();
        }
    }
    let name = harness.state().current_folder_name();
    assert!(name.as_deref() != Some("trip 2"), "{name:?}");
}

#[test]
fn folder_list_as_tree_by_year_and_by_size() {
    let fixture = Fixture::new("folder-list");
    let at = |secs: u64| std::time::UNIX_EPOCH + Duration::from_secs(secs);
    for (dir, when, count) in [("2019/summer", at(1_561_896_000), 1), ("2021", at(1_623_844_800), 4)] {
        let dir = fixture.dir.join(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for k in 0..count {
            let path = dir.join(format!("p{k}.png"));
            RgbImage::from_fn(300, 200, |x, y| Rgb([x as u8, y as u8, 40 * k as u8])).save(&path).unwrap();
            std::fs::File::options().write(true).open(&path).unwrap().set_modified(when).unwrap();
        }
    }
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 3);
    let top = |harness: &Harness<'_, PicoApp>, label: &str| harness.get_by_label(label).rect().top();

    harness.state_mut().set_folder_list(false, "date");
    harness.step();
    harness.step();
    // Newest first, under year headers.
    assert!(top(&harness, "2026  (1 folder)") < top(&harness, "2021  (1 folder)"));
    assert!(top(&harness, "2021  (1 folder)") < top(&harness, "2019  (1 folder)"));
    assert!(top(&harness, "2021  (4)") < top(&harness, "summer  (1)"));

    harness.state_mut().set_folder_list(false, "size");
    harness.step();
    harness.step();
    assert!(top(&harness, "2021  (4)") < top(&harness, "Holiday  (3)"), "biggest first");

    // In the tree, 2019 has no photos of its own and holds summer.
    harness.state_mut().set_folder_list(true, "name");
    harness.step();
    harness.step();
    let year = harness.get_by_label("2019");
    assert!(year.rect().top() < top(&harness, "summer  (1)"));
    assert!(year.rect().left() < harness.get_by_label("summer  (1)").rect().left(), "summer is indented");
    screenshot(&mut harness, "folder-tree");
    harness.get_by_label("summer  (1)").click();
    harness.step();
    assert_eq!(harness.state().current_folder_name().as_deref(), Some("summer"));
}

#[test]
fn only_videos_filter() {
    let fixture = Fixture::new("videos-only");
    // Only the name matters for the filter; ffmpeg need not understand it.
    std::fs::write(fixture.dir.join("clip.mp4"), b"not really a video").unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.state_mut().select_folder(0);
    harness.step();
    assert_eq!(harness.state().listing_names().len(), 4);
    harness.get_by_label("▶").click();
    harness.step();
    harness.step();
    assert_eq!(harness.state().listing_names(), ["clip.mp4"]);
    harness.get_by_label("Holiday, 1 video  ·  October 2026");
    assert!(harness.query_by_label("a.png").is_none(), "photos are hidden from the grid too");
    harness.get_by_label("▶").click();
    harness.step();
    assert_eq!(harness.state().listing_names().len(), 4);
}

#[test]
fn edited_photos_are_marked() {
    let fixture = Fixture::new("badges");
    let ini = fixture.ini() + "[c.png]\r\nfilters=bw=1;\r\n";
    std::fs::write(fixture.dir.join(".picasa.ini"), ini).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    wait_until(&mut harness, "the thumbnails", |app| app.thumbnails_pending() == 0);
    let tooltip = |harness: &mut Harness<'_, PicoApp>, name: &str| {
        harness.hover_at(harness.get_by_label(name).rect().center());
        // Tooltips wait for the pointer to rest.
        for _ in 0..4 {
            harness.step();
        }
        harness
            .query_all(egui_kittest::kittest::by().role(egui::accesskit::Role::Label))
            .filter_map(|n| n.accesskit_node().value())
            // The tooltip: the name alone, or the name and more lines.
            .find(|v| v == name || v.starts_with(&format!("{name}\n")))
            .unwrap_or_default()
    };
    assert_eq!(tooltip(&mut harness, "c.png"), "c.png\nEdited");
    screenshot(&mut harness, "badges");
    assert_eq!(tooltip(&mut harness, "a.png"), "a.png");
}

#[test]
fn status_bar_tells_about_the_photo_and_the_selection() {
    let fixture = Fixture::new("status-bar");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    assert_eq!(harness.state().status_details_text(), "");
    harness.hover_at(harness.get_by_label("b.png").rect().center());
    harness.step();
    let text = harness.state().status_details_text();
    assert!(text.starts_with("b.png  ·  ") && text.ends_with(" bytes"), "{text}");

    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("c.png").click_modifiers(Modifiers::COMMAND);
    harness.step();
    harness.remove_cursor();
    harness.step();
    harness.step();
    let text = harness.state().status_details_text();
    assert!(text.starts_with("2 selected  ·  ") && text.ends_with(" bytes"), "{text}");
    screenshot(&mut harness, "status-bar");
}

#[test]
fn home_end_pages_and_ctrl_3_in_the_grid() {
    let fixture = Fixture::new("grid-keys");
    for k in 0..30 {
        RgbImage::from_pixel(40, 30, Rgb([k * 8, 50, 50])).save(fixture.dir.join(format!("z{k:02}.png"))).unwrap();
    }
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.state_mut().select_folder(0);
    harness.step();
    let name = |harness: &Harness<'_, PicoApp>| {
        let at = harness.state().selected_file().unwrap();
        harness.state().listing_names()[at].clone()
    };
    harness.key_press(Key::End);
    harness.step();
    assert_eq!(name(&harness), "z29.png");
    harness.key_press(Key::Home);
    harness.step();
    assert_eq!(name(&harness), "a.png");
    harness.key_press(Key::PageDown);
    harness.step();
    let paged = harness.state().selected_file().unwrap();
    assert!(paged >= 5, "a page is more than a row: {paged}");
    harness.key_press(Key::PageUp);
    harness.step();
    assert_eq!(name(&harness), "a.png");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num3);
    harness.step();
    assert!(harness.state().is_viewing(), "Ctrl+3 opens the photo");
}

#[test]
fn settings_come_back_next_time() {
    let fixture = Fixture::new("settings");
    std::fs::create_dir_all(fixture.dir.join("more")).unwrap();
    RgbImage::from_pixel(60, 40, Rgb([200, 200, 0])).save(fixture.dir.join("more/d.png")).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 2);
    harness.get_by_label("more  (1)").click();
    harness.step();
    harness.hover_at(harness.get_by_label("d.png").rect().center());
    harness.event(egui::Event::Zoom(1.25));
    harness.step();
    harness.state_mut().set_folder_list(true, "size");
    harness.step();
    let ctx = harness.ctx.clone();
    let settings = harness.state().settings_for_test(&ctx);
    assert_eq!(settings.get("thumb_size").map(String::as_str), Some("200"));
    assert_eq!(settings.get("folder_tree").map(String::as_str), Some("true"));
    assert!(settings.get("last_folder").is_some_and(|p| p.ends_with("more")), "{settings:?}");
    // (The window size comes from eframe, which the test harness does not report.)
    assert_eq!(settings.get("sidebar_width").map(String::as_str), Some("260"));
    drop(harness);

    // A new window with those settings opens where the last one was.
    let dir = fixture.dir.clone();
    let mut harness = Harness::builder().with_size([1000.0, 800.0]).build_eframe(move |cc| {
        let mut app = PicoApp::new(&cc.egui_ctx, vec![dir], false);
        app.apply_settings_for_test(&settings);
        app
    });
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 2);
    assert_eq!(harness.state().current_folder_name().as_deref(), Some("more"));
    assert_eq!(harness.state().thumbnail_size(), 200.0);
}

#[test]
fn keyboard_shortcuts_window() {
    let fixture = Fixture::new("shortcuts");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("Help").click();
    harness.step();
    harness.step();
    harness.get_by_label("Keyboard Shortcuts").click();
    harness.step();
    harness.step();
    harness.get_by_label("Hold in the Photo Tray");
    screenshot(&mut harness, "shortcuts");
    harness.key_press(Key::Escape);
    harness.step();
    assert!(harness.query_by_label("Hold in the Photo Tray").is_none());
    harness.key_press(Key::F1);
    harness.step();
    harness.step();
    harness.get_by_label("Undo / redo an edit");
}

#[test]
fn about_window_shows_the_logo() {
    let fixture = Fixture::new("about");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("Help").click();
    harness.step();
    harness.step();
    harness.get_by_label("About Pico").click();
    harness.step();
    harness.step();
    harness.get_by_label(pico_core::geo::CREDITS);
    harness.get_by_label("Open-source licenses").click();
    for _ in 0..4 {
        harness.step();
    }
    harness.get_by_label("Pico is built on free software and open data. Thank you to their authors.");
    screenshot(&mut harness, "about");
}

#[test]
fn light_theme_in_options() {
    let fixture = Fixture::new("theme");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("Tools").click();
    harness.step();
    harness.step();
    harness.get_by_label("Options…").click();
    harness.step();
    harness.step();
    harness.get_by_label("Dark").click();
    harness.step();
    harness.step();
    assert!(harness.ctx.global_style().visuals.dark_mode);
    harness.get_by_label("Light").click();
    harness.step();
    harness.step();
    assert!(!harness.ctx.global_style().visuals.dark_mode, "light visuals");
    screenshot(&mut harness, "light-theme");
    let ctx = harness.ctx.clone();
    assert_eq!(harness.state().settings_for_test(&ctx).get("theme").map(String::as_str), Some("light"));

    // "As the system" follows the desktop, both ways.
    harness.get_by_label("As the system").click();
    harness.step();
    let ctx = harness.ctx.clone();
    harness.state_mut().set_system_dark(&ctx, false);
    harness.step();
    assert!(!harness.ctx.global_style().visuals.dark_mode, "the desktop says light");
    harness.state_mut().set_system_dark(&ctx, true);
    harness.step();
    assert!(harness.ctx.global_style().visuals.dark_mode, "the desktop says dark");
    harness.get_by_label("Light").click();
    harness.step();

    harness.key_press(Key::Escape);
    harness.step();
    assert!(harness.query_by_label("Look").is_none(), "Esc closes Options");
    harness.state_mut().open_file_named("b.png");
    harness.state_mut().open_edit_panel();
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());
    screenshot(&mut harness, "light-viewer");
}

#[test]
fn arrows_move_through_photos_in_viewer() {
    let fixture = Fixture::new("arrows");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    harness.key_press(Key::ArrowRight);
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(0), "the first arrow press selects the first photo");
    harness.key_press(Key::Enter);
    harness.step();
    harness.key_press(Key::ArrowRight);
    harness.step();
    harness.key_press(Key::ArrowRight);
    harness.step();
    harness.key_press(Key::ArrowRight);
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(2), "stops at the last photo");
    harness.key_press(Key::ArrowLeft);
    harness.step();
    assert_eq!(harness.state().selected_file(), Some(1));
}

/// Every non-ASCII character in the app's source (icons, typographic
/// punctuation) must exist in the bundled fonts; otherwise it renders as an
/// empty box. Scanning the source keeps this honest as the UI grows.
#[test]
fn every_non_ascii_character_has_a_glyph() {
    let _gpu = gpu_turn();
    let sources = [
        include_str!("app.rs"),
        include_str!("grid.rs"),
        include_str!("main.rs"),
        include_str!("slideshow.rs"),
        include_str!("thumbs.rs"),
        include_str!("tools.rs"),
        include_str!("viewer.rs"),
    ];
    let mut chars: Vec<char> = sources.iter().flat_map(|s| s.chars()).filter(|c| !c.is_ascii()).collect();
    chars.sort_unstable();
    chars.dedup();
    let mut missing = Vec::new();
    let mut harness = Harness::new_ui(|ui| {
        for &ch in &chars {
            if !ui.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), &ch.to_string())) {
                missing.push(ch);
            }
        }
    });
    harness.step();
    drop(harness);
    assert!(missing.is_empty(), "no glyph for {missing:?}");
}

/// Counts screenshot pixels of exactly this color; `None` without a GPU.
fn count_color(harness: &mut Harness<'_, PicoApp>, rgb: [u8; 3]) -> Option<usize> {
    let image = harness.render().ok()?;
    Some(image.pixels().filter(|p| p.0[..3] == rgb).count())
}

#[test]
fn edit_panel_writes_filters_and_renders_them() {
    let fixture = Fixture::new("editor");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("b.png").click();
    harness.step();
    harness.key_press(Key::Enter);
    harness.state_mut().open_edit_panel();
    wait_until(&mut harness, "the photo to load", |app| app.viewer_loaded());
    let green = [40, 200, 40];
    let gray = [134, 134, 134]; // Rec. 601 luma of the fixture green
    if let Some(n) = count_color(&mut harness, green) {
        assert!(n > 1000, "the green photo is on screen");
    }

    harness.get_by_label("B&W").click();
    harness.step();
    assert!(fixture.ini().contains("filters=bw=1;\r\n"), "{}", fixture.ini());
    wait_until(&mut harness, "the B&W render", |app| app.viewer_loaded());
    if let (Some(green_left), Some(gray_now)) = (count_color(&mut harness, green), count_color(&mut harness, gray)) {
        assert_eq!(green_left, 0, "no green left after B&W");
        assert!(gray_now > 1000, "the photo turned gray");
    }
    screenshot(&mut harness, "editor");

    // Slider changes from the keyboard or accessibility are saved at once and
    // keep editing the same Tuning entry while it is on top of the stack.
    harness.get_by_label("Fill Light").set_value_accesskit(0.5);
    harness.step();
    harness.get_by_label("Shadows").set_value_accesskit(0.1);
    harness.step();
    assert!(
        fixture.ini().contains("filters=bw=1;finetune2=1,0.500000,0.000000,0.100000,00000000,0.000000;\r\n"),
        "{}",
        fixture.ini()
    );

    harness.get_by_label("Undo").click();
    harness.step();
    assert!(fixture.ini().contains("filters=bw=1;\r\n"), "{}", fixture.ini());
    harness.get_by_label("Undo All Edits").click();
    harness.step();
    assert!(!fixture.ini().contains("filters="), "{}", fixture.ini());
    assert!(fixture.ini().contains("backuphash=4242"), "{}", fixture.ini());
}

/// Opens every folder of `testdata/library` and saves a screenshot of each to
/// `target/ui-shots/tour-*.png`. Run explicitly after `testdata/make_testdata.py`.
#[test]
#[ignore = "needs testdata/library; run with --ignored"]
fn testdata_tour() {
    let _gpu = gpu_turn();
    let library = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/library");
    assert!(library.is_dir(), "run testdata/make_testdata.py first");
    let mut harness = Harness::builder()
        .with_size([1400.0, 900.0])
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![library], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() > 0);
    let names = harness.state().folder_names();
    for (index, name) in names.iter().enumerate() {
        harness.state_mut().select_folder(index);
        harness.step();
        wait_until(&mut harness, "thumbnails", |app| app.thumbnails_pending() == 0);
        screenshot(&mut harness, &format!("tour-{index:02}"));
        println!("tour-{index:02}: {name}");
    }
}

/// Reads the last `crop64` of `file` from the INI as unit `[left, top, right, bottom]`.
fn ini_crop(fixture: &Fixture, file: &str) -> [f32; 4] {
    let ini = pico_core::ini::PicasaIni::parse(&fixture.ini());
    let meta = pico_core::meta::PhotoMeta::from_section(ini.section(file).expect("section"));
    meta.filters.crop().expect("a crop").to_unit()
}

#[test]
fn crop_tool_drag_on_rotated_photo_and_straighten() {
    let fixture = Fixture::new("tools");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("b.png").click();
    harness.step();
    harness.key_press(Key::Enter);
    harness.state_mut().open_edit_panel();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    harness.step();
    wait_until(&mut harness, "the rotated photo", |app| app.viewer_loaded());

    harness.get_by_label("Crop").click();
    harness.step();
    let photo = harness.get_by_label("Photo").rect();
    // Select the top-left quarter of the photo as shown (rotated clockwise).
    harness.drag_at(photo.min + egui::vec2(1.0, 1.0));
    harness.step();
    harness.hover_at(photo.center() - egui::vec2(10.0, 10.0));
    harness.step();
    harness.hover_at(photo.center());
    harness.step();
    harness.drop_at(photo.center());
    harness.step();
    harness.get_by_label("Apply").click();
    harness.step();

    // The shown top-left of a clockwise-rotated photo is its bottom-left.
    let [left, top, right, bottom] = ini_crop(&fixture, "b.png");
    let near = |a: f32, b: f32| (a - b).abs() < 0.03;
    assert!(
        near(left, 0.0) && near(top, 0.5) && near(right, 0.5) && near(bottom, 1.0),
        "{:?}",
        [left, top, right, bottom]
    );
    assert!(fixture.ini().contains("crop=rect64("), "{}", fixture.ini());
    assert!(fixture.ini().contains("rotate=rotate(1)"), "{}", fixture.ini());
    wait_until(&mut harness, "the cropped photo", |app| app.viewer_loaded());
    screenshot(&mut harness, "crop");

    harness.get_by_label("Straighten").click();
    // One frame handles the click, the next draws the tool's panel.
    harness.step();
    harness.step();
    // Drag the angle slider from its middle (0°) to three quarters (+6°).
    let bar = harness.get_by_role(egui::Role::Slider).rect();
    harness.drag_at(bar.center());
    harness.step();
    harness.hover_at(bar.lerp_inside(egui::vec2(0.6, 0.5)));
    harness.step();
    harness.drop_at(bar.lerp_inside(egui::vec2(0.75, 0.5)));
    harness.step();
    assert!(harness.state().tool_state().starts_with("straighten 0.5"), "{}", harness.state().tool_state());
    harness.key_press(Key::Enter);
    harness.step();
    let ini = fixture.ini();
    let tilt = ini.split("tilt=1,").nth(1).and_then(|t| t.split(',').next()).and_then(|v| v.parse::<f32>().ok());
    assert!(tilt.is_some_and(|t| (t - 0.5).abs() < 0.1), "{ini}");

    // Esc closes an open tool without leaving the photo.
    harness.get_by_label("Crop").click();
    harness.step();
    harness.key_press(Key::Escape);
    harness.step();
    assert!(harness.state().is_viewing());
}

#[test]
fn effect_tool_places_center_on_rotated_photo() {
    let fixture = Fixture::new("effect");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("b.png").click();
    harness.step();
    harness.key_press(Key::Enter);
    harness.state_mut().open_edit_panel();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    harness.step();
    wait_until(&mut harness, "the rotated photo", |app| app.viewer_loaded());

    // One-click effects are added at once.
    harness.get_by_label("Sepia").click();
    harness.step();
    assert!(fixture.ini().contains("filters=sepia=1;\r\n"), "{}", fixture.ini());

    harness.get_by_label("Soft Focus").click();
    harness.step();
    harness.step();
    assert!(harness.state().tool_state().starts_with("effect radblur"), "{}", harness.state().tool_state());
    // Click near the top-right corner of the clockwise-rotated photo: that is
    // near the top-left corner of the photo itself. The tool panel may widen
    // in its first frame, so let the layout settle first.
    harness.step();
    let photo = harness.get_by_label("Photo").rect();
    harness.get_by_label("Photo").hover();
    harness.step();
    harness.event(egui::Event::PointerMoved(photo.right_top() + egui::vec2(-2.0, 2.0)));
    harness.event(egui::Event::PointerButton {
        pos: photo.right_top() + egui::vec2(-2.0, 2.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: photo.right_top() + egui::vec2(-2.0, 2.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    harness.step();
    wait_until(&mut harness, "the preview", |app| !app.tool_state().is_empty());
    screenshot(&mut harness, "effect");
    harness.key_press(Key::Enter);
    harness.step();

    let ini = fixture.ini();
    let params = ini.split("radblur=1,").nth(1).expect("radblur saved").split(';').next().unwrap();
    let values: Vec<f32> = params.split(',').map(|v| v.parse().unwrap()).collect();
    assert!(values[0] < 0.1 && values[1] < 0.1, "center near the source top-left: {params}");
    assert!(ini.contains("filters=sepia=1;radblur=1,"), "{ini}");
}

#[test]
fn slideshow_advances_pauses_and_returns() {
    let fixture = Fixture::new("slideshow");
    let dir = fixture.dir.clone();
    let mut harness = Harness::builder()
        .with_size([800.0, 600.0])
        .with_step_dt(0.1)
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num4);
    harness.step();
    assert_eq!(harness.state().slideshow_index(), Some(0));
    harness.state_mut().set_slide_seconds(0.5);
    wait_until(&mut harness, "the next slide", |app| app.slideshow_index() == Some(1));
    screenshot(&mut harness, "slideshow");

    harness.key_press(Key::Space); // pause
    harness.step();
    for _ in 0..20 {
        harness.step();
    }
    assert_eq!(harness.state().slideshow_index(), Some(1), "paused slideshows stay put");
    harness.key_press(Key::ArrowRight);
    harness.step();
    harness.key_press(Key::ArrowRight);
    harness.step();
    assert_eq!(harness.state().slideshow_index(), Some(0), "wraps around after the last photo");
    harness.key_press(Key::ArrowLeft);
    harness.step();
    harness.key_press(Key::Escape);
    harness.step();
    assert_eq!(harness.state().slideshow_index(), None);
    assert_eq!(harness.state().selected_file(), Some(2), "the library selects where the slideshow stopped");
}

#[test]
fn video_plays_saves_a_frame_and_plays_in_slideshows() {
    let Some(ffmpeg) = pico_core::video::ffmpeg() else { return };
    let fixture = Fixture::new("video");
    let made = std::process::Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=10:duration=4",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(fixture.dir.join("clip.mp4"))
        .status()
        .unwrap();
    assert!(made.success());
    let dir = fixture.dir.clone();
    let mut harness = Harness::builder()
        .with_size([900.0, 650.0])
        .with_step_dt(0.1)
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("clip.mp4").click();
    harness.step();
    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the first frame", |app| app.player_state().2);
    assert!(!harness.state().player_state().0, "videos open paused");

    harness.key_press(Key::Slash);
    wait_until(&mut harness, "playback", |app| app.player_state().1 >= Duration::from_millis(800));
    screenshot(&mut harness, "video");
    harness.key_press(Key::Slash);
    harness.step();
    let paused_at = harness.state().player_state().1;
    assert!(!harness.state().player_state().0);

    harness.get_by_label("Save Frame as Photo").click();
    harness.step();
    let saved = format!("clip.{}.jpg", paused_at.as_millis());
    assert!(fixture.dir.join(&saved).is_file(), "{saved} written");
    assert!(harness.state().file_names().contains(&saved), "{:?}", harness.state().file_names());

    // A starred video gets the badge in its corner, above the controls.
    harness.key_press(Key::S);
    for _ in 0..12 {
        harness.step();
    }
    screenshot(&mut harness, "video-starred");

    // In a slideshow the video plays, and the show moves on when it ends.
    harness.key_press_modifiers(Modifiers::COMMAND, Key::Num4);
    harness.step();
    let at = harness.state().slideshow_index().expect("slideshow");
    wait_until(&mut harness, "the video to play", |app| app.slideshow_video_playing());
    wait_until(&mut harness, "the next slide", |app| app.slideshow_index() != Some(at));
}

#[test]
fn video_shows_every_frame_while_playing() {
    let Some(ffmpeg) = pico_core::video::ffmpeg() else { return };
    let fixture = Fixture::new("video-frames");
    let made = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=160x120:rate=10:duration=4"])
        .args(["-pix_fmt", "yuv420p"])
        .arg(fixture.dir.join("clip.mp4"))
        .status()
        .unwrap();
    assert!(made.success());
    let dir = fixture.dir.clone();
    // Two frames of the UI per video frame, as the player asks for.
    let mut harness = Harness::builder()
        .with_size([900.0, 650.0])
        .with_step_dt(0.05)
        .build_eframe(move |cc| PicoApp::new(&cc.egui_ctx, vec![dir], false));
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("clip.mp4").click();
    harness.step();
    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the first frame", |app| app.player_state().2);

    harness.key_press(Key::Slash);
    let mut shown = std::collections::BTreeSet::new();
    for _ in 0..50 {
        harness.step();
        // Real time for ffmpeg to keep up.
        std::thread::sleep(Duration::from_millis(30));
        shown.insert(harness.state().player_state().1);
    }
    assert!(shown.len() >= 15, "only {} of about 25 frames were shown: {shown:?}", shown.len());

    // Leaving for the library or another file stops it.
    assert!(harness.state().player_state().0);
    harness.key_press(Key::Escape);
    harness.step();
    assert!(!harness.state().is_viewing());
    assert!(!harness.state().player_state().0, "stopped in the library");
    harness.key_press(Key::Enter);
    wait_until(&mut harness, "the first frame", |app| app.player_state().2);
    harness.key_press(Key::Slash);
    harness.step();
    assert!(harness.state().player_state().0);
    harness.key_press(Key::ArrowLeft);
    harness.step();
    assert!(!harness.state().player_state().0, "stopped on the photo before");

    // A click on the picture plays once it is clearly not a double-click;
    // a double-click goes back to the library, as for photos.
    harness.key_press(Key::ArrowRight);
    wait_until(&mut harness, "the first frame", |app| app.player_state().2);
    harness.get_by_label("Video").click();
    harness.step();
    assert!(!harness.state().player_state().0, "waits for a second click");
    for _ in 0..8 {
        harness.step();
    }
    assert!(harness.state().player_state().0, "a single click plays");
    double_click(&mut harness, "Video");
    harness.step();
    assert!(!harness.state().is_viewing());
    assert!(!harness.state().player_state().0);
}

#[test]
fn batch_commands_search_and_filters() {
    let fixture = Fixture::new("batch");
    // A second folder, so search and Starred span the library.
    let other = fixture.dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    RgbImage::from_pixel(60, 40, Rgb([90, 90, 90])).save(other.join("sea.png")).unwrap();
    std::fs::write(other.join(".picasa.ini"), "[sea.png]\r\nkeywords=beach,summer\r\nstar=yes\r\n").unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 2);
    // Folders sort naturally by path: the root folder comes first.
    harness.state_mut().select_folder(0);
    harness.step();

    // Ctrl-click and Shift-click build a selection; batch commands write once.
    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("c.png").click_modifiers(Modifiers::COMMAND);
    harness.step();
    assert_eq!(harness.state().selected_count(), 2);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::R);
    harness.step();
    let ini = fixture.ini();
    assert!(ini.contains("[a.png]\r\nrotate=rotate(1)") && ini.contains("[c.png]\r\nrotate=rotate(1)"), "{ini}");
    assert!(!ini.contains("[b.png]\r\nbackuphash=4242\r\nrotate"), "b.png was not selected: {ini}");

    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("c.png").click_modifiers(Modifiers::SHIFT);
    harness.step();
    assert_eq!(harness.state().selected_count(), 3, "shift-click selects the range");
    harness.key_press(Key::S);
    harness.step();
    assert_eq!(fixture.ini().matches("star=yes").count(), 3, "{}", fixture.ini());

    // Copy one photo's effects and paste them onto the others.
    harness.get_by_label("a.png").click();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::B);
    harness.step();
    harness.state_mut().copy_and_paste_to_all();
    harness.step();
    assert_eq!(fixture.ini().matches("filters=bw=1;").count(), 3, "{}", fixture.ini());

    // Hiding removes the photo from the grid until hidden photos are shown.
    harness.get_by_label("b.png").click();
    harness.step();
    harness.state_mut().hide_current();
    harness.step();
    assert!(fixture.ini().contains("hidden=yes"), "{}", fixture.ini());
    assert_eq!(harness.state().listing_names(), ["a.png", "c.png"]);

    // Search covers every folder: captions, keywords and names.
    harness.state_mut().set_search("summer");
    harness.step();
    assert_eq!(harness.state().listing_names(), ["sea.png"]);
    harness.state_mut().set_search("");
    harness.state_mut().show_starred();
    harness.step();
    assert_eq!(harness.state().listing_names(), ["a.png", "c.png", "sea.png"], "hidden b.png stays hidden");
    screenshot(&mut harness, "starred");
}

#[test]
fn save_revert_export_and_rename_from_menus() {
    let fixture = Fixture::new("fileops");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("b.png").click();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::B);
    harness.step();
    let original = std::fs::read(fixture.dir.join("b.png")).unwrap();

    harness.get_by_label("File").click();
    harness.step();
    harness.step();
    harness.get_by_label("Save").click();
    harness.step();
    assert_eq!(std::fs::read(fixture.dir.join(".picasaoriginals/b.png")).unwrap(), original, "original kept");
    assert!(!fixture.ini().contains("filters="), "edits are baked in: {}", fixture.ini());
    let saved = pico_core::image::open(fixture.dir.join("b.png")).unwrap().to_rgb8();
    let p = saved.get_pixel(5, 5).0;
    assert!(p[0] == p[1] && p[1] == p[2], "saved file is B&W: {p:?}");

    harness.get_by_label("File").click();
    harness.step();
    harness.step();
    harness.get_by_label("Revert").click();
    harness.step();
    assert_eq!(std::fs::read(fixture.dir.join("b.png")).unwrap(), original);
    assert!(fixture.ini().contains("filters=bw=1;"), "{}", fixture.ini());

    harness.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    harness.step();
    let out = fixture.dir.join("exported");
    harness.step();
    harness.state_mut().set_export_dir(&out);
    harness.step();
    harness.get_by_label("Export 1 picture").click();
    let target = out.join("b.jpg");
    wait_until(&mut harness, "the export", move |app| target.is_file() && !app.exporting());
    let exported = pico_core::image::open(out.join("b.jpg")).expect("exported JPEG").to_rgb8();
    let p = exported.get_pixel(5, 5).0;
    assert!(p[0].abs_diff(p[1]) < 6 && p[1].abs_diff(p[2]) < 6, "export has the edits: {p:?}");

    harness.key_press(Key::F2);
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    // The rename field has focus; typing replaces the selected old name.
    harness.event(egui::Event::Text("green.png".into()));
    harness.step();
    harness.key_press(Key::Enter);
    harness.step();
    assert!(fixture.dir.join("green.png").is_file() && !fixture.dir.join("b.png").exists());
    assert!(fixture.ini().contains("[green.png]\r\nbackuphash=4242"), "{}", fixture.ini());
}

#[test]
fn sorting_by_name_date_and_size() {
    let fixture = Fixture::new("sort");
    // Sizes: a tiny, b medium, c large (noise does not compress).
    RgbImage::from_pixel(4, 4, Rgb([1, 2, 3])).save(fixture.dir.join("a.png")).unwrap();
    let noise = RgbImage::from_fn(200, 200, |x, y| Rgb([(x * 7 + y * 13) as u8, (x * y) as u8, (x ^ y) as u8]));
    noise.save(fixture.dir.join("c.png")).unwrap();
    // Dates: c oldest, then a, then b.
    let day = Duration::from_secs(86_400);
    let base = std::time::SystemTime::now() - day * 30;
    for (name, days) in [("c.png", 0), ("a.png", 1), ("b.png", 2)] {
        let file = std::fs::File::options().write(true).open(fixture.dir.join(name)).unwrap();
        file.set_modified(base + day * days).unwrap();
    }
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);

    assert_eq!(harness.state().listing_names(), ["a.png", "b.png", "c.png"]);
    harness.state_mut().sort("date", false);
    assert_eq!(harness.state().listing_names(), ["c.png", "a.png", "b.png"]);
    harness.state_mut().sort("size", false);
    assert_eq!(harness.state().listing_names(), ["a.png", "b.png", "c.png"]);
    harness.state_mut().sort("date", true);
    assert_eq!(harness.state().listing_names(), ["b.png", "a.png", "c.png"]);
}

#[test]
fn info_panel_tags_selected_photos() {
    let fixture = Fixture::new("tags");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("b.png").click_modifiers(Modifiers::COMMAND);
    harness.step();

    harness.key_press_modifiers(Modifiers::ALT, Key::Enter);
    harness.step();
    assert!(!harness.state().is_viewing(), "Alt+Enter shows Info, it does not open the photo");
    harness.key_press_modifiers(Modifiers::COMMAND, Key::K);
    harness.step();
    harness.step();
    harness.event(egui::Event::Text("holiday".into()));
    harness.step();
    harness.key_press(Key::Enter);
    harness.step();
    let ini = fixture.ini();
    assert!(ini.contains("[a.png]\r\nkeywords=holiday") && ini.contains("keywords=holiday"), "{ini}");
    assert_eq!(harness.state().keywords_of("b.png"), ["holiday"]);
    assert!(harness.state().keywords_of("c.png").is_empty());
    screenshot(&mut harness, "info");

    harness.step();
    harness.get_by_label("holiday (2/2) ×").click();
    harness.step();
    assert!(!fixture.ini().contains("keywords="), "{}", fixture.ini());
}

#[test]
fn albums_create_open_add_and_remove() {
    let fixture = Fixture::new("albums");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("a.png").click();
    harness.step();
    harness.key_press_modifiers(Modifiers::COMMAND, Key::N);
    harness.step();
    harness.step();
    harness.event(egui::Event::Text("Trip".into()));
    harness.step();
    harness.key_press(Key::Enter);
    harness.step();
    let ini = fixture.ini();
    let token = ini.split("[.album:").nth(1).and_then(|r| r.split(']').next()).expect("album declared").to_owned();
    assert!(ini.contains(&format!("name=Trip\r\ntoken={token}\r\ndate=")), "{ini}");
    assert!(ini.contains(&format!("albums={token}")), "{ini}");
    assert_eq!(harness.state().album_names(), ["Trip (1)"]);

    // Add a second photo through Picture > Add to Album.
    harness.get_by_label("c.png").click();
    harness.step();
    harness.get_by_label("Picture").click();
    harness.step();
    harness.step();
    harness.get_by_label("Add to Album").hover();
    harness.step();
    harness.step();
    harness.get_by_label("Trip").click();
    harness.step();
    assert_eq!(harness.state().album_names(), ["Trip (2)"]);

    harness.state_mut().open_album("Trip");
    harness.step();
    assert_eq!(harness.state().listing_names(), ["a.png", "c.png"]);
    harness.get_by_label("a.png").click();
    harness.step();
    harness.get_by_label("Picture").click();
    harness.step();
    harness.step();
    harness.get_by_label("Remove from Album").click();
    harness.step();
    assert_eq!(harness.state().listing_names(), ["c.png"]);
    assert_eq!(fixture.ini().matches(&format!("albums={token}")).count(), 1, "{}", fixture.ini());
}

#[test]
fn tools_duplicates_folder_manager_and_options() {
    let fixture = Fixture::new("tools-menu");
    std::fs::create_dir_all(fixture.dir.join("copies")).unwrap();
    std::fs::copy(fixture.dir.join("a.png"), fixture.dir.join("copies/a again.png")).unwrap();
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 2);

    harness.get_by_label("Tools").click();
    harness.step();
    harness.step();
    harness.get_by_label("Show Duplicate Files").click();
    harness.step(); // the click starts the search
    wait_until(&mut harness, "the duplicate search", |app| !app.finding_duplicates());
    assert!(harness.state().status_text().contains("1 group"), "{}", harness.state().status_text());
    assert_eq!(harness.state().listing_names(), ["a.png", "a again.png"]);

    harness.get_by_label("Tools").click();
    harness.step();
    harness.step();
    harness.get_by_label("Options…").click();
    harness.step();
    harness.step();
    harness.get_by_label("Clear Thumbnail Cache");
    screenshot(&mut harness, "options");

    harness.get_by_label("Tools").click();
    harness.step();
    harness.step();
    harness.get_by_label("Folder Manager…").click();
    harness.step();
    harness.step();
    // The manager opens on the watched folder; typing extends its path.
    let dir = fixture.dir.display().to_string();
    let path = harness.get_by(|n| n.role() == egui::accesskit::Role::TextInput && n.value().as_deref() == Some(&dir));
    path.focus();
    path.type_text("/copies");
    harness.step();
    harness.key_press(Key::Enter);
    harness.step();
    harness.step();
    harness.get_by_label("Leave out of Pico").click();
    harness.step();
    screenshot(&mut harness, "folder-manager");
    harness.get_by_label("Watching 1 folder, leaving out 1 folder.");
    harness.get_by_label("OK").click();
    harness.step();
    wait_until(&mut harness, "the rescan", |app| app.folder_count() == 1);
    assert_eq!(harness.state().watched_count(), 1);

    harness.get_by_label("Tools").click();
    harness.step();
    harness.step();
    harness.get_by_label("Folder Manager…").click();
    harness.step();
    harness.step();
    harness.get_by_label("Leave out of Pico").click();
    harness.step();
    harness.get_by_label("OK").click();
    harness.step();
    assert_eq!(harness.state().watched_count(), 0);
    wait_until(&mut harness, "the rescan", |app| app.folder_count() == 0);
    assert!(fixture.dir.join("a.png").is_file(), "removing a folder from Pico leaves the files alone");
}

#[test]
fn timeline_groups_by_month() {
    let fixture = Fixture::new("timeline");
    // a and c in March 2021 (c first), b in June 2021; UTC noon avoids date edges.
    let at = |secs: u64| std::time::UNIX_EPOCH + Duration::from_secs(secs);
    for (name, when) in [("a.png", at(1_615_896_000)), ("b.png", at(1_623_844_800)), ("c.png", at(1_614_686_400))] {
        let file = std::fs::File::options().write(true).open(fixture.dir.join(name)).unwrap();
        file.set_modified(when).unwrap();
    }
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.get_by_label("Timeline").click();
    harness.step();
    harness.step();
    harness.get_by_label("2021  (3)").click();
    harness.step();
    harness.step();
    harness.get_by_label("March 2021  (2)").click();
    harness.step();
    assert_eq!(harness.state().listing_names(), ["c.png", "a.png"], "in the order they were taken");
    harness.state_mut().open_month("2021:06");
    assert_eq!(harness.state().listing_names(), ["b.png"]);
    assert!(harness.state().place_keys().is_empty(), "no GPS, no places");
}

#[test]
fn create_movie_from_selection() {
    if pico_core::video::ffmpeg().is_none() || pico_core::video::ffprobe().is_none() {
        return;
    }
    let fixture = Fixture::new("movie");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_label("Create").click();
    harness.step();
    harness.step();
    harness.get_by_label("Movie…").click();
    harness.step();
    harness.step();
    let out = fixture.dir.join("holiday.mp4");
    harness.state_mut().set_movie_output(&out);
    harness.step();
    harness.get_by_label("Make the Movie").click();
    harness.step();
    harness.step(); // a new window is sized, unseen, in its first frame
    harness.step();
    if harness.state().making_movie() {
        assert!(
            harness.query_by_label_contains("Preparing photo").is_some()
                || harness.query_by_label("Encoding the movie…").is_some(),
            "a progress window while the movie is made"
        );
        screenshot(&mut harness, "progress");
    }
    let target = out.clone();
    wait_until(&mut harness, "the movie", move |app| !app.making_movie() && target.is_file());
    let info = pico_core::video::probe(&out).unwrap();
    assert_eq!((info.width, info.height), (1280, 720));
    assert!((info.duration.as_secs_f32() - 3.0).abs() < 0.2, "3 photos x 1 s: {:?}", info.duration);
    assert!(harness.state().file_names().contains(&"holiday.mp4".to_owned()), "the movie shows up in its folder");
}

#[test]
fn picture_collage_previews_saves_and_files_into_album() {
    let fixture = Fixture::new("collage");
    let mut harness = harness(&fixture.dir);
    wait_until(&mut harness, "the folder scan", |app| app.folder_count() == 1);
    harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
    harness.step();
    harness.get_by_label("Create").click();
    harness.step();
    harness.step();
    harness.get_by_label("Picture Collage…").click();
    harness.step();
    assert!(harness.state().in_collage());
    wait_until(&mut harness, "the collage preview", |app| app.collage_ready());
    screenshot(&mut harness, "collage-mosaic");

    harness.state_mut().collage_layout(pico_core::collage::Layout::Grid);
    harness.step();
    wait_until(&mut harness, "the grid preview", |app| app.collage_ready());
    screenshot(&mut harness, "collage-grid");

    harness.get_by_label("Create Collage").click();
    harness.step();
    let saved = fixture.dir.join("Collage 1.jpg");
    let target = saved.clone();
    wait_until(&mut harness, "the saved collage", move |app| {
        target.is_file() && app.status_text().contains("Collage saved")
    });
    let image = pico_core::image::open(&saved).unwrap();
    assert_eq!((image.width(), image.height()), (3000, 2000));
    assert_eq!(harness.state().album_names(), ["Collages (1)"]);

    harness.key_press(Key::Escape);
    harness.step();
    assert!(!harness.state().in_collage());
}
