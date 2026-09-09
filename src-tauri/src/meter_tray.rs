//! macOS menu-bar tray for the usage meter (CodexBar port, v1).
//!
//! CodexBar is a menu-bar-only app (`LSUIElement`, one `NSStatusItem` per
//! provider). Bridge stays a windowed app; the tray is a companion surface:
//! one status item with a native menu (Show Bridge, Refresh usage, Quit) plus
//! a tooltip carrying the worst live window. The live percent ring already
//! lives in-app (`UsageWidget`); per-window pace and reset countdowns render
//! in the meter popover (`src/components/meter/`).
//!
//! The tray never blocks startup: any failure here is swallowed so a tray
//! regression cannot take down the desktop shell.

use tauri::{tray::TrayIconBuilder, AppHandle, Emitter, Manager};

const TRAY_ID: &str = "bridge-meter";

/// Build the meter tray item. Idempotent best-effort: returns `Ok(())` when
/// the tray already exists or when tray construction fails.
pub fn build(app: &tauri::App<tauri::Wry>) -> Result<(), String> {
    let handle = app.handle();
    if handle.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let menu = tauri::menu::MenuBuilder::new(handle)
        .text("show-bridge", "Show Bridge")
        .text("refresh-meter", "Refresh usage")
        .separator()
        .text("quit", "Quit Bridge")
        .build()
        .map_err(|error| error.to_string())?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Bridge — usage meter")
        .show_menu_on_left_click(false);
    if let Some(icon) = handle.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show-bridge" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "refresh-meter" => {
                app.emit("bridge-meter-tray", "refresh")
                    .unwrap_or_default();
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                tauri::tray::TrayIconEvent::Click {
                    button: tauri::tray::MouseButton::Left,
                    ..
                }
            ) {
                let app = tray.app_handle().clone();
                app.emit("bridge-meter-tray", "open-popover")
                    .unwrap_or_default();
            }
        })
        .build(handle)
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// Update the tray tooltip with the worst live window, when the frontend
/// reports one. `None` restores the default tooltip.
pub fn set_tooltip(app: &AppHandle, tooltip: Option<String>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_tooltip(tooltip.as_deref());
    }
}
