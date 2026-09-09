//! macOS menu-bar meter: a status item with the live percentage, and a panel
//! that opens under it.
//!
//! CodexBar is a menu-bar app — the number lives in the menu bar and a click
//! drops a card down beneath it. Bridge stays a windowed app, so this is a
//! companion surface, but the surface itself has to behave like a menu-bar
//! one. It previously did not: the tray raised the main window and rendered
//! the meter as a centred modal over the app, which is not a menu bar dropping
//! down a panel, it is an app interrupting you.
//!
//! Two rules follow from that, and they are the reason for the shape here.
//!
//! **The panel is its own window.** Borderless, floating, positioned from the
//! status item's own rect, hidden until asked for.
//!
//! **Showing it must not activate Bridge.** `WebviewWindow::set_focus` runs
//! `activateIgnoringOtherApps: YES` (tao `platform_impl/macos/util/async.rs`),
//! which orders every Bridge window forward — the main window included. So the
//! panel is only ever `show()`n, never focused, and is built non-focusable with
//! first-mouse acceptance so its controls still take a click without the click
//! making it key. A menu-bar panel that yanks your editor forward is the bug,
//! not a detail.
//!
//! The tray never blocks startup: any failure here is swallowed so a tray
//! regression cannot take down the desktop shell.

use tauri::{
    tray::TrayIconBuilder, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl,
    WebviewWindowBuilder,
};

const TRAY_ID: &str = "bridge-meter";
pub const PANEL_LABEL: &str = "meter";

const PANEL_WIDTH: f64 = 360.0;
const PANEL_HEIGHT: f64 = 460.0;
/// Clearance under the menu bar so the panel reads as hanging from the icon.
const PANEL_GAP: f64 = 6.0;
/// Keep the panel off the exact screen edge when the icon sits in a corner.
const SCREEN_MARGIN: f64 = 8.0;

/// Create the meter panel, hidden. Built once at startup rather than per click
/// so opening it is a `show()` rather than a webview cold start.
pub fn build_panel(app: &tauri::App<tauri::Wry>) -> Result<(), String> {
    let handle = app.handle();
    if handle.get_webview_window(PANEL_LABEL).is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(
        handle,
        PANEL_LABEL,
        WebviewUrl::App("index.html?window=meter".into()),
    )
    .title("Bridge Meter")
    .inner_size(PANEL_WIDTH, PANEL_HEIGHT)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    // Never key: a click inside must not activate Bridge and pull the main
    // window forward. `accept_first_mouse` is what keeps the buttons live
    // while the window stays unfocused.
    .focused(false)
    .accept_first_mouse(true)
    .build()
    .map_err(|error| error.to_string())?;
    Ok(())
}

/// Show the panel under `anchor_x` (the status item's horizontal centre, in
/// logical points), or hide it when it is already up.
pub fn toggle_panel(app: &tauri::AppHandle, anchor_x: Option<f64>) {
    let Some(panel) = app.get_webview_window(PANEL_LABEL) else {
        return;
    };
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
        return;
    }
    if let Some(position) = panel_position(&panel, anchor_x) {
        let _ = panel.set_position(position);
    }
    // `show` only — see the module note on `set_focus` activating the app.
    let _ = panel.show();
}

pub fn hide_panel(app: &tauri::AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL_LABEL) {
        let _ = panel.hide();
    }
}

/// Where the panel should sit: centred under the status item, pushed back
/// inside the screen's visible area when the icon is near an edge.
fn panel_position(
    panel: &tauri::WebviewWindow,
    anchor_x: Option<f64>,
) -> Option<LogicalPosition<f64>> {
    let monitor = panel
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| panel.primary_monitor().ok().flatten())?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let origin = LogicalPosition::<f64>::from_physical(area.position, scale);
    let size = LogicalSize::<f64>::from_physical(area.size, scale);
    let anchor = anchor_x.unwrap_or(origin.x + size.width - PANEL_WIDTH / 2.0 - SCREEN_MARGIN);
    let (x, y) = clamp_to_work_area(anchor, origin.x, origin.y, size.width);
    Some(LogicalPosition::new(x, y))
}

/// Split out from monitor lookup so the clamping is unit-testable.
fn clamp_to_work_area(
    anchor_x: f64,
    work_left: f64,
    work_top: f64,
    work_width: f64,
) -> (f64, f64) {
    let ideal = anchor_x - PANEL_WIDTH / 2.0;
    let leftmost = work_left + SCREEN_MARGIN;
    // `work_area` already excludes the menu bar, so its top is the first pixel
    // the panel may occupy.
    let rightmost = (work_left + work_width - PANEL_WIDTH - SCREEN_MARGIN).max(leftmost);
    (ideal.clamp(leftmost, rightmost), work_top + PANEL_GAP)
}

/// Put the worst live window's percentage in the menu bar. Empty clears it.
pub fn set_tray_title(app: &tauri::AppHandle, title: &str) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_title(if title.is_empty() { None } else { Some(title) });
    }
}

/// Build the meter tray item. Idempotent best-effort: returns `Ok(())` when
/// the tray already exists or when tray construction fails.
pub fn build(app: &tauri::App<tauri::Wry>) -> Result<(), String> {
    let handle = app.handle();
    if handle.tray_by_id(TRAY_ID).is_some() {
        return Ok(());
    }
    let menu = tauri::menu::MenuBuilder::new(handle)
        .text("open-meter", "Open Meter")
        .text("refresh-meter", "Refresh usage")
        .separator()
        .text("show-bridge", "Show Bridge")
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
            "open-meter" => toggle_panel(app, None),
            "refresh-meter" => {
                app.emit("bridge-meter-tray", "refresh").unwrap_or_default();
            }
            "show-bridge" => {
                // The panel is a menu-bar surface; asking for the app is the
                // one path that should raise the app.
                hide_panel(app);
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::Click {
                button: tauri::tray::MouseButton::Left,
                button_state: tauri::tray::MouseButtonState::Up,
                rect,
                ..
            } = event
            {
                let app = tray.app_handle().clone();
                // Centre on the status item itself, so the panel hangs from
                // the icon the user actually clicked.
                let scale = app
                    .get_webview_window(PANEL_LABEL)
                    .and_then(|panel| panel.scale_factor().ok())
                    .unwrap_or(1.0);
                // The rect arrives as `Position`/`Size`, either variant, so
                // normalise rather than assuming physical.
                let position: LogicalPosition<f64> = rect.position.to_logical(scale);
                let size: LogicalSize<f64> = rect.size.to_logical(scale);
                let anchor = position.x + size.width / 2.0;
                // Refresh on open so the panel never shows a stale number.
                app.emit("bridge-meter-tray", "refresh").unwrap_or_default();
                toggle_panel(&app, Some(anchor));
            }
        })
        .build(handle)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_centres_under_the_status_item() {
        let (x, y) = clamp_to_work_area(700.0, 0.0, 25.0, 1440.0);
        assert_eq!(x, 700.0 - PANEL_WIDTH / 2.0);
        assert_eq!(y, 25.0 + PANEL_GAP);
    }

    #[test]
    fn panel_stays_on_screen_at_the_right_edge() {
        // A status item hard against the right of a 1440pt display: centring
        // would put the panel's right edge past the screen.
        let (x, _) = clamp_to_work_area(1435.0, 0.0, 25.0, 1440.0);
        assert_eq!(x, 1440.0 - PANEL_WIDTH - SCREEN_MARGIN);
        assert!(x + PANEL_WIDTH <= 1440.0);
    }

    #[test]
    fn panel_stays_on_screen_at_the_left_edge() {
        let (x, _) = clamp_to_work_area(4.0, 0.0, 25.0, 1440.0);
        assert_eq!(x, SCREEN_MARGIN);
    }

    #[test]
    fn panel_respects_a_secondary_monitor_origin() {
        // A display to the right of the primary: coordinates are global, so
        // clamping must use the monitor's own origin rather than zero.
        let (x, y) = clamp_to_work_area(2500.0, 1440.0, 0.0, 1920.0);
        assert!(x >= 1440.0 + SCREEN_MARGIN);
        assert!(x + PANEL_WIDTH <= 1440.0 + 1920.0);
        assert_eq!(y, PANEL_GAP);
    }

    #[test]
    fn a_narrow_display_cannot_invert_the_clamp() {
        // Narrower than the panel: the bounds would cross, and `clamp` panics
        // when min exceeds max.
        let (x, _) = clamp_to_work_area(150.0, 0.0, 25.0, 300.0);
        assert_eq!(x, SCREEN_MARGIN);
    }
}
