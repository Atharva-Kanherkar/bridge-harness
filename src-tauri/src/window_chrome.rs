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
//!
//! The 16pt corner radius is the same material, not CSS. Calling `set_effects`
//! to change it would stack another effect view, so fullscreen zeros the
//! radius on the view window-vibrancy already installed.

/// Where the buttons sit: left inset, and the strip midline they center on.
/// The strip is 44pt tall with the brand centered, so the midline is 22pt.
pub const TRAFFIC_LIGHT_X: f64 = 20.0;
pub const STRIP_MIDLINE_Y: f64 = 22.0;

/// Matches `windowEffects.radius` in tauri.conf.json. Windowed chrome stays
/// a 16pt rounded overlay; fullscreen and zoom must be a flush rectangle.
pub const WINDOWED_CORNER_RADIUS: f64 = 16.0;

/// `window-vibrancy` tags the Sidebar material with this so later calls can
/// find it. Must stay in lockstep with `NS_VIEW_TAG_BLUR_VIEW` there.
pub const NS_VIEW_TAG_BLUR_VIEW: isize = 91376254;

static LAYOUT_FULLSCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn parse_layout_fullscreen_payload(payload: &str) -> bool {
    serde_json::from_str::<bool>(payload).unwrap_or(false)
}

/// In-window ⌥⌘F is not OS fullscreen, but it still has to be a rectangle.
pub fn set_layout_fullscreen(on: bool) {
    LAYOUT_FULLSCREEN.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Square the window when it fills the display or the in-app layout is
/// fullscreen; restore the 16pt radius when it does not. Also stamps
/// `data-flush-window` on the document so CSS can drop nested squircles
/// without a frontend window API.
pub fn sync_fullscreen_chrome<R: tauri::Runtime>(window: &tauri::Window<R>) {
    #[cfg(target_os = "macos")]
    {
        let _ = catch_objc(std::panic::AssertUnwindSafe(|| {
            sync_fullscreen_chrome_inner(window)
        }));
    }
    #[cfg(not(target_os = "macos"))]
    {
        sync_fullscreen_chrome_inner(window);
    }
}

fn sync_fullscreen_chrome_inner<R: tauri::Runtime>(window: &tauri::Window<R>) {
    let flush = LAYOUT_FULLSCREEN.load(std::sync::atomic::Ordering::Relaxed)
        || window.is_fullscreen().unwrap_or(false)
        || window.is_maximized().unwrap_or(false);
    apply_window_corner_radius(window, if flush { 0.0 } else { WINDOWED_CORNER_RADIUS });
    let native = window.is_fullscreen().unwrap_or(false);
    set_document_flush_window(window, flush, native);
}

fn set_document_flush_window<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    flush: bool,
    native_fullscreen: bool,
) {
    let js = format!(
        r#"document.documentElement.toggleAttribute("data-flush-window",{flush});document.documentElement.toggleAttribute("data-native-fullscreen",{native});document.documentElement.dispatchEvent(new Event("bridge-flush-window"));"#,
        flush = if flush { "true" } else { "false" },
        native = if native_fullscreen { "true" } else { "false" },
    );
    for webview in window.webviews() {
        let _ = webview.eval(&js);
    }
}

/// AppKit can throw through these calls. tao's run-loop observer uses Rust
/// `catch_unwind`, which aborts on a foreign Objective-C exception.
#[cfg(target_os = "macos")]
fn catch_objc<R>(f: impl FnOnce() -> R + std::panic::UnwindSafe) -> Option<R> {
    match objc2::exception::catch(f) {
        Ok(value) => Some(value),
        Err(_) => {
            eprintln!("bridge: ignored Objective-C exception in window chrome");
            None
        }
    }
}

#[cfg(target_os = "macos")]
pub fn position_traffic_lights<R: tauri::Runtime>(window: &tauri::Window<R>) {
    let _ = catch_objc(std::panic::AssertUnwindSafe(|| {
        position_traffic_lights_inner(window)
    }));
}

#[cfg(target_os = "macos")]
fn position_traffic_lights_inner<R: tauri::Runtime>(window: &tauri::Window<R>) {
    use objc2_app_kit::{NSView, NSWindow, NSWindowButton};

    let Ok(handle) = window.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    // `ns_window()` hands back the NSWindow this window is drawn by; it outlives
    // this borrow because the window itself is alive for the call.
    let ns_window: &NSWindow = unsafe { &*handle.cast::<NSWindow>() };
    let Some(close) = ns_window.standardWindowButton(NSWindowButton::CloseButton) else {
        return;
    };
    let Some(miniaturize) = ns_window.standardWindowButton(NSWindowButton::MiniaturizeButton)
    else {
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

/// Keep the window clear so the Sidebar material behind the webview stays
/// visible. The material itself comes from `windowEffects` in tauri.conf.json,
/// installed once at window creation — calling `set_effects` here again would
/// stack another `NSVisualEffectView` on every theme change, because tauri's
/// macOS path only ever adds effect views. A no-op off macOS.
#[cfg(target_os = "macos")]
pub fn apply_wallpaper_tint<R: tauri::Runtime>(window: &tauri::Window<R>) {
    let _ = catch_objc(std::panic::AssertUnwindSafe(|| {
        apply_wallpaper_tint_inner(window)
    }));
}

#[cfg(target_os = "macos")]
fn apply_wallpaper_tint_inner<R: tauri::Runtime>(window: &tauri::Window<R>) {
    use objc2_app_kit::{NSColor, NSWindow};

    let Ok(handle) = window.ns_window() else {
        return;
    };
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

/// Mutate the existing Sidebar material. Do not call `set_effects` — tauri's
/// macOS path only ever adds effect views, so a second call stacks another.
#[cfg(target_os = "macos")]
fn apply_window_corner_radius<R: tauri::Runtime>(window: &tauri::Window<R>, radius: f64) {
    use objc2::msg_send;
    use objc2_app_kit::NSWindow;

    let Ok(handle) = window.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    let ns_window: &NSWindow = unsafe { &*handle.cast::<NSWindow>() };
    let Some(content) = ns_window.contentView() else {
        return;
    };

    if let Some(blur) = content.viewWithTag(NS_VIEW_TAG_BLUR_VIEW) {
        // Private NSVisualEffectView setter — the same one window-vibrancy uses
        // when the material is first installed.
        let _: () = unsafe { msg_send![&*blur, setCornerRadius: radius] };
        set_layer_radius(&blur, radius);
    }
}

#[cfg(target_os = "macos")]
fn set_layer_radius(view: &objc2_app_kit::NSView, radius: f64) {
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(radius);
        layer.setMasksToBounds(radius > 0.0);
    }
}

#[cfg(not(target_os = "macos"))]
fn apply_window_corner_radius<R: tauri::Runtime>(_window: &tauri::Window<R>, _radius: f64) {}

#[cfg(test)]
mod tests {
    #[test]
    fn windowed_radius_matches_tauri_conf() {
        assert_eq!(super::WINDOWED_CORNER_RADIUS, 16.0);
    }

    #[test]
    fn blur_view_tag_matches_window_vibrancy() {
        assert_eq!(super::NS_VIEW_TAG_BLUR_VIEW, 91376254);
    }

    #[test]
    fn layout_fullscreen_can_toggle() {
        super::set_layout_fullscreen(true);
        super::set_layout_fullscreen(false);
    }

    #[test]
    fn layout_fullscreen_payload_is_a_json_bool() {
        assert!(super::parse_layout_fullscreen_payload("true"));
        assert!(!super::parse_layout_fullscreen_payload("false"));
        assert!(!super::parse_layout_fullscreen_payload("not-json"));
    }
}
