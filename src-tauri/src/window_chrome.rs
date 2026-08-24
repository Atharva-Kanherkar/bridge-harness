//! Native window chrome that the web layer cannot reach.
//!
//! The traffic lights belong on the titlebar strip's midline, beside the brand.
//! macOS rebuilds the titlebar on some window operations and restores the
//! buttons to their default corner, and tao's config-time inset does not
//! survive that, so Bridge positions the buttons itself from their measured
//! frames and reapplies on window events.
//!
//! Wallpaper tint is the same story: CSS cannot sample the desktop. The
//! Sidebar material (`NSVisualEffectView`) sits behind a transparent webview
//! and AppKit mixes in the wallpaper when System Settings → Appearance →
//! "Allow wallpaper tinting in windows" is on.

/// Where the buttons sit: left inset, and the strip midline they center on.
/// The strip is 44pt tall with the brand centered, so the midline is 22pt.
pub const TRAFFIC_LIGHT_X: f64 = 20.0;
pub const STRIP_MIDLINE_Y: f64 = 22.0;

#[cfg(target_os = "macos")]
pub fn position_traffic_lights<R: tauri::Runtime>(window: &tauri::Window<R>) {
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};

    let Ok(handle) = window.ns_window() else { return };
    if handle.is_null() {
        return;
    }
    // `ns_window()` hands back the NSWindow this window is drawn by; it outlives
    // this borrow because the window itself is alive for the call.
    let ns_window: &NSWindow = unsafe { &*handle.cast::<NSWindow>() };
    let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(miniaturize) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton) else {
        return;
    };
    let Some(zoom) = ns_window.standardWindowButton(NSWindowButton::ZoomButton) else {
        return;
    };
    // The buttons live in the titlebar view inside the titlebar container; the
    // container is pinned to the window's top edge, so a button centered in
    // container coordinates is centered against the window top.
    let Some(container) = (unsafe { close.superview().and_then(|view| view.superview()) }) else {
        return;
    };

    let container_height = NSView::frame(&container).size.height;
    let close_frame = NSView::frame(&close);
    let spacing = NSView::frame(&miniaturize).origin.x - close_frame.origin.x;

    for (index, button) in [close, miniaturize, zoom].into_iter().enumerate() {
        let mut frame = NSView::frame(&button);
        frame.origin.x = TRAFFIC_LIGHT_X + index as f64 * spacing;
        frame.origin.y = container_height - STRIP_MIDLINE_Y - frame.size.height / 2.0;
        button.setFrameOrigin(frame.origin);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn position_traffic_lights<R: tauri::Runtime>(_window: &tauri::Window<R>) {
    // Only macOS overlays its window controls on the client area; every other
    // platform keeps them in chrome we do not draw over.
}

/// Put the native Sidebar material behind the webview so wallpaper tint can
/// reach the rail. A no-op off macOS — CSS in Chrome stays fully opaque.
#[cfg(target_os = "macos")]
pub fn apply_wallpaper_tint<R: tauri::Runtime>(window: &tauri::Window<R>) {
    use objc2_app_kit::{NSColor, NSWindow};
    use tauri::window::{Color, Effect, EffectState, EffectsBuilder};

    let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
    let _ = window.set_effects(
        EffectsBuilder::new()
            .effect(Effect::Sidebar)
            .state(EffectState::FollowsWindowActiveState)
            .radius(16.0)
            .build(),
    );

    let Ok(handle) = window.ns_window() else { return };
    if handle.is_null() {
        return;
    }
    let ns_window: &NSWindow = unsafe { &*handle.cast::<NSWindow>() };
    ns_window.setOpaque(false);
    let clear = NSColor::clearColor();
    ns_window.setBackgroundColor(Some(&clear));
}

#[cfg(not(target_os = "macos"))]
pub fn apply_wallpaper_tint<R: tauri::Runtime>(_window: &tauri::Window<R>) {}
