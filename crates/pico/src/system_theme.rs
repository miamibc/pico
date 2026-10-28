//! Light or dark, as the desktop wants it.
//!
//! On Windows and macOS egui learns this from the window system: winit
//! reports a theme change as an event and "as the system" follows it. On
//! Linux, winit does not ask the desktop under Wayland, so Pico reads the XDG
//! portal's `color-scheme` (GNOME, KDE and others publish it there) and
//! listens for the portal's `SettingChanged` signal, so a switch in the
//! system settings shows up right away without polling.

use std::sync::mpsc::{Receiver, channel};

/// `Some(true)` for dark. Sent once at start and again on every change.
pub fn watch(ctx: &eframe::egui::Context) -> Option<Receiver<bool>> {
    if !cfg!(all(unix, not(target_os = "macos"))) {
        return None;
    }
    let (tx, rx) = channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        // Subscribe before the first read, so a change in between is not lost.
        let Some(changes) = changes() else { return };
        // No portal at all: nothing to watch.
        let Some(dark) = prefers_dark() else { return };
        let mut last = dark;
        if tx.send(dark).is_err() {
            return;
        }
        ctx.request_repaint();
        for dark in changes {
            if dark != last {
                last = dark;
                if tx.send(dark).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        }
    });
    Some(rx)
}

/// Every new `color-scheme` the portal announces, as "is it dark". Blocks
/// between signals; ends when the session bus goes away.
#[cfg(all(unix, not(target_os = "macos")))]
fn changes() -> Option<impl Iterator<Item = bool>> {
    use zbus::zvariant::OwnedValue;
    let connection = zbus::blocking::Connection::session().ok()?;
    let proxy = zbus::blocking::Proxy::new_owned(
        connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
    )
    .ok()?;
    let signals = proxy
        .receive_signal_with_args("SettingChanged", &[(0, "org.freedesktop.appearance"), (1, "color-scheme")])
        .ok()?;
    Some(signals.filter_map(|message| {
        let (_, _, value): (String, String, OwnedValue) = message.body().deserialize().ok()?;
        let scheme: u32 = value.downcast_ref().ok()?;
        Some(scheme == 1)
    }))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn changes() -> Option<std::iter::Empty<bool>> {
    None
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
