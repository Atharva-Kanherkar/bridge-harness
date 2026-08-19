//! Native window chrome that the web layer cannot reach.
//!
//! The macOS traffic lights are drawn by the system on top of our content, so no
//! amount of CSS can quiet them. Bridge hides them at startup and reveals them
//! while the pointer is in the corner they occupy, which is how Claude Code's
//! desktop app behaves. The frontend owns the hover test — it is the only side
//! that sees the cursor — and calls in through one local command.

/// Command name the frontend invokes. Deliberately not a `bridge-protocol`
/// method: this is local window chrome, not something a remote daemon can serve,
/// so it is routed before the protocol lookup and stays out of
/// `generate_handler![...]`.
pub const SET_TRAFFIC_LIGHTS_COMMAND: &str = "set_traffic_lights_visible";

#[cfg(target_os = "macos")]
pub fn set_traffic_lights_visible<R: tauri::Runtime>(window: tauri::Window<R>, visible: bool) {
    use objc2_app_kit::{NSWindow, NSWindowButton};

    let Ok(handle) = window.ns_window() else { return };
    if handle.is_null() {
        return;
    }
    // `ns_window()` hands back the NSWindow this window is drawn by; it outlives
    // this borrow because the window itself is alive for the call.
    let ns_window: &NSWindow = unsafe { &*handle.cast::<NSWindow>() };
    for button in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton(button) {
            button.setHidden(!visible);
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_traffic_lights_visible<R: tauri::Runtime>(_window: tauri::Window<R>, _visible: bool) {
    // Only macOS overlays its window controls on the client area; every other
    // platform keeps them in chrome we do not draw over.
}

/// True for the one command this module owns. Checked before the protocol router
/// looks the command up, since resolving an invoke consumes it.
pub fn owns(command: &str) -> bool {
    command == SET_TRAFFIC_LIGHTS_COMMAND
}

/// Handles the local chrome command. Only call it when [`owns`] said so.
pub fn handle_invoke<R: tauri::Runtime>(invoke: tauri::ipc::Invoke<R>) -> bool {
    let visible = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(value) => value.get("visible").and_then(|v| v.as_bool()),
        tauri::ipc::InvokeBody::Raw(bytes) => serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|value| value.get("visible").and_then(|v| v.as_bool())),
    };
    match visible {
        Some(visible) => {
            set_traffic_lights_visible(invoke.message.webview_ref().window(), visible);
            invoke.resolver.resolve(());
        }
        None => invoke
            .resolver
            .reject(format!("{SET_TRAFFIC_LIGHTS_COMMAND} needs a boolean `visible`")),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chrome_command_is_not_a_protocol_method() {
        // If it ever became one, the 1:1 registry test would demand it appear in
        // generate_handler![...], where a window-chrome toggle does not belong.
        assert!(bridge_protocol::MethodName::from_command(SET_TRAFFIC_LIGHTS_COMMAND).is_none());
    }
}
