//! Light or dark, as the desktop wants it.
//!
//! On Windows and macOS egui learns this from the window system. On Linux,
//! winit does not ask the desktop under Wayland, so Pico reads the XDG
//! portal's `color-scheme` (GNOME, KDE and others publish it there) and
//! watches it, so a switch in the system settings shows up right away.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// `Some(true)` for dark. Sent once at start and again on every change.
pub fn watch(ctx: &eframe::egui::Context) -> Option<Receiver<bool>> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return None;
    }
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let mut last = None;
        loop {
            let dark = prefers_dark();
            if dark.is_some() && dark != last {
                last = dark;
                if tx.send(dark == Some(true)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            // No portal at all: nothing to watch.
            if dark.is_none() && last.is_none() {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    Some(rx)
}

/// The portal's answer: 1 means dark; 0 (no preference) and 2 mean light,
/// which is what GNOME's "Light" setting and GTK apps go by.
#[cfg(all(unix, not(target_os = "macos")))]
fn prefers_dark() -> Option<bool> {
    use zbus::zvariant::OwnedValue;
    let connection = zbus::blocking::Connection::session().ok()?;
    let call = |method: &str| {
        connection.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Settings"),
            method,
            &("org.freedesktop.appearance", "color-scheme"),
        )
    };
    // ReadOne is newer; older portals only have Read, which wraps the value
    // in one more variant.
    let value: OwnedValue = match call("ReadOne") {
        Ok(reply) => reply.body().deserialize().ok()?,
        Err(_) => {
            let outer: OwnedValue = call("Read").ok()?.body().deserialize().ok()?;
            let inner: zbus::zvariant::Value = outer.downcast_ref().ok()?;
            inner.try_to_owned().ok()?
        }
    };
    let scheme: u32 = value.downcast_ref().ok()?;
    Some(scheme == 1)
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn prefers_dark() -> Option<bool> {
    None
}
