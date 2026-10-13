//! Linux desktop integration that has to happen before or around the window.
//!
//! Three things, all inert on macOS:
//!
//! - **App id.** GTK takes the Wayland app_id / X11 WM_CLASS from the program
//!   name, which would be the binary's (`bridge-deck`). Pin it to `bridge` so
//!   a Hyprland rule can match `class:^(bridge)$`.
//! - **Hyprland owns the frame.** Under Hyprland the compositor draws borders,
//!   gaps and rounding, so the main window drops client decorations and the
//!   transparent ground. GNOME and KDE keep their server-side decorations.
//! - **NVIDIA on Wayland.** WebKitGTK's DMABUF renderer draws a blank or
//!   corrupted window on the NVIDIA driver under Wayland; turning it off there,
//!   and only there, is the documented workaround.

/// The Wayland app id and X11 class Bridge's windows carry on Linux.
pub const APP_ID: &str = "bridge";

const DMABUF_ENV: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

/// Whether the compositor should draw the window frame.
pub fn hyprland_owns_chrome() -> bool {
    cfg!(target_os = "linux") && hyprland_session(std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").as_deref())
}

fn hyprland_session(signature: Option<&std::ffi::OsStr>) -> bool {
    signature.is_some_and(|value| !value.is_empty())
}

/// The DMABUF workaround applies to NVIDIA under Wayland, unless the user
/// already chose a value themselves.
pub fn needs_dmabuf_workaround(nvidia_driver: bool, wayland: bool, already_set: bool) -> bool {
    nvidia_driver && wayland && !already_set
}

fn wayland_session(wayland_display: Option<&std::ffi::OsStr>, session_type: Option<&str>) -> bool {
    wayland_display.is_some_and(|value| !value.is_empty()) || session_type == Some("wayland")
}

/// Process-wide setup. Must run before Tauri initializes GTK and WebKit.
pub fn prepare_process() {
    #[cfg(target_os = "linux")]
    {
        let wayland = wayland_session(
            std::env::var_os("WAYLAND_DISPLAY").as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        );
        let nvidia = std::path::Path::new("/proc/driver/nvidia").exists();
        if needs_dmabuf_workaround(nvidia, wayland, std::env::var_os(DMABUF_ENV).is_some()) {
            // SAFETY: called first thing in `run`, before any thread exists.
            unsafe { std::env::set_var(DMABUF_ENV, "1") };
        }
        glib::set_prgname(Some(APP_ID));
        glib::set_application_name("Bridge");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn hyprland_is_detected_from_its_instance_signature() {
        assert!(hyprland_session(Some(OsStr::new("abc_123_456"))));
        assert!(!hyprland_session(Some(OsStr::new(""))));
        assert!(!hyprland_session(None));
    }

    #[test]
    fn the_dmabuf_workaround_is_nvidia_on_wayland_only_and_never_overrides_the_user() {
        assert!(needs_dmabuf_workaround(true, true, false));
        assert!(!needs_dmabuf_workaround(true, false, false), "X11 is unaffected");
        assert!(!needs_dmabuf_workaround(false, true, false), "Intel/AMD are unaffected");
        assert!(!needs_dmabuf_workaround(true, true, true), "an explicit user value wins");
    }

    #[test]
    fn wayland_is_detected_from_the_display_or_session_type() {
        assert!(wayland_session(Some(OsStr::new("wayland-1")), None));
        assert!(wayland_session(None, Some("wayland")));
        assert!(!wayland_session(None, Some("x11")));
        assert!(!wayland_session(Some(OsStr::new("")), None));
    }
}
