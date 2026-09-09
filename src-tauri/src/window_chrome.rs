//! Native window chrome that the web layer cannot reach.
//!
//! AppKit owns the titlebar and traffic lights. Do not walk or resize its
//! private titlebar container: those views change during fullscreen/layout.
//!
//! Wallpaper tint is the same story: CSS cannot sample the desktop. The
//! Sidebar material (`NSVisualEffectView`) sits behind a transparent webview
//! and AppKit mixes in the wallpaper when System Settings → Appearance →
//! "Allow wallpaper tinting in windows" is on.
//!
//! Install one Sidebar material using public AppKit APIs and round its layer.
//! window-vibrancy uses the private NSVisualEffectView.setCornerRadius selector,
//! including during Tauri window creation, before our exception boundary exists.

/// Windowed chrome stays
/// a 16pt rounded overlay; fullscreen and zoom must be a flush rectangle.
pub const WINDOWED_CORNER_RADIUS: f64 = 16.0;

#[cfg(target_os = "macos")]
thread_local! {
    // This is our material, not Wry's content view. AppKit objects stay on the
    // main thread, and Destroyed releases our ownership before a later launch.
    static MATERIALS: std::cell::RefCell<std::collections::HashMap<String, std::rc::Rc<Material>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

#[cfg(target_os = "macos")]
struct Material {
    view: objc2::rc::Retained<objc2_app_kit::NSVisualEffectView>,
}

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
    on_next_main_loop(window, "fullscreen chrome", |window, _mtm| {
        sync_fullscreen_chrome_inner(window);
    });
    #[cfg(not(target_os = "macos"))]
    sync_fullscreen_chrome_inner(window);
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

/// A real asynchronous dispatch avoids mutating AppKit during its current
/// layout/fullscreen callback. Tauri's run_on_main_thread executes inline when
/// called on the main thread, so it does not provide that guarantee.
#[cfg(target_os = "macos")]
fn on_next_main_loop<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    operation: &'static str,
    f: impl FnOnce(&tauri::Window<R>, objc2::MainThreadMarker) + Send + 'static,
) {
    let owned = window.clone();
    dispatch2::DispatchQueue::main().exec_async(move || {
        if let Err(error) = crate::diagnostics::native_boundary(|| {
            use tauri::Manager;
            if owned
                .app_handle()
                .get_webview_window(owned.label())
                .is_none()
            {
                return;
            }
            let Some(mtm) = objc2::MainThreadMarker::new() else {
                crate::diagnostics::record(&format!(
                    "{operation}: AppKit work was not on the main thread"
                ));
                return;
            };
            f(&owned, mtm);
        }) {
            crate::diagnostics::record(&format!("{operation}: {error}"));
        }
    });
}

/// Keep one Sidebar material behind the transparent webview. Its public
/// CALayer API handles rounding; no private NSVisualEffectView selectors.
#[cfg(target_os = "macos")]
pub fn apply_wallpaper_tint<R: tauri::Runtime>(window: &tauri::Window<R>) {
    on_next_main_loop(window, "wallpaper tint", apply_wallpaper_tint_inner);
}

#[cfg(target_os = "macos")]
fn apply_wallpaper_tint_inner<R: tauri::Runtime>(
    window: &tauri::Window<R>,
    mtm: objc2::MainThreadMarker,
) {
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSColor, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowOrderingMode,
    };

    if MATERIALS.with(|materials| materials.borrow().contains_key(window.label())) {
        return;
    }
    let Ok(handle) = window.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    // Only startup borrows Wry's content view. Keep an explicit balanced
    // retain instead of depending on optimized autoreleased return handling.
    // The live Window and main-thread dispatch bound the raw-pointer borrow.
    let ns_window: &NSWindow = unsafe { &*handle.cast::<NSWindow>() };
    let raw: *mut objc2_app_kit::NSView = unsafe { objc2::msg_send![ns_window, contentView] };
    let Some(content) = (unsafe { objc2::rc::Retained::retain(raw) }) else {
        return;
    };
    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    let view = NSVisualEffectView::initWithFrame(mtm.alloc(), content.bounds());
    view.setMaterial(NSVisualEffectMaterial::Sidebar);
    view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    view.setState(NSVisualEffectState::FollowsWindowActiveState);
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    set_layer_radius(&view, WINDOWED_CORNER_RADIUS);
    content.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
    MATERIALS.with(|materials| {
        materials.borrow_mut().insert(
            window.label().to_owned(),
            std::rc::Rc::new(Material { view }),
        );
    });
    sync_fullscreen_chrome_inner(window);
}

/// Destroyed callbacks run on the event-loop thread. Release only the material
/// we own; AppKit handles removal of the closing window's native hierarchy.
/// Do not use on_next_main_loop here: its live-window guard skips destroyed
/// windows, retaining the cached effect after close.
pub fn release_window_material(label: &str) {
    #[cfg(target_os = "macos")]
    {
        if let Err(error) = crate::diagnostics::native_boundary(|| {
            let Some(_mtm) = objc2::MainThreadMarker::new() else {
                crate::diagnostics::record("window material cleanup was not on the main thread");
                return;
            };
            let material = MATERIALS.with(|materials| materials.borrow_mut().remove(label));
            drop(material);
        }) {
            crate::diagnostics::record(&format!("window material cleanup: {error}"));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = label;
}

#[cfg(not(target_os = "macos"))]
pub fn apply_wallpaper_tint<R: tauri::Runtime>(_window: &tauri::Window<R>) {}

/// Mutate the existing material using the public CALayer API.
#[cfg(target_os = "macos")]
fn apply_window_corner_radius<R: tauri::Runtime>(window: &tauri::Window<R>, radius: f64) {
    // Never retrieve or traverse Wry's content view during resize/fullscreen.
    // A retained temporary from that path reached NSView::dealloc in our
    // release-build zoom reproduction. Update only Bridge's explicitly owned
    // effect view. The layer setter itself avoids redundant mutations and
    // also handles AppKit replacing the material's backing layer.
    let material = MATERIALS.with(|materials| materials.borrow().get(window.label()).cloned());
    if let Some(material) = material {
        set_layer_radius(&material.view, radius);
    }
}

#[cfg(target_os = "macos")]
fn set_layer_radius(view: &objc2_app_kit::NSView, radius: f64) {
    if !view.wantsLayer() {
        view.setWantsLayer(true);
    }
    if let Some(layer) = view.layer() {
        if (layer.cornerRadius() - radius).abs() > f64::EPSILON {
            layer.setCornerRadius(radius);
        }
        if layer.masksToBounds() != (radius > 0.0) {
            layer.setMasksToBounds(radius > 0.0);
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn apply_window_corner_radius<R: tauri::Runtime>(_window: &tauri::Window<R>, _radius: f64) {}

#[cfg(test)]
mod tests {
    #[test]
    fn windowed_radius_is_16_points() {
        assert_eq!(super::WINDOWED_CORNER_RADIUS, 16.0);
    }

    #[test]
    fn window_creation_does_not_use_private_vibrancy_setters() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        for window in config["app"]["windows"].as_array().unwrap() {
            assert!(
                window.get("windowEffects").is_none(),
                "Tauri windowEffects bypasses our AppKit exception boundary"
            );
            assert!(
                window.get("trafficLightPosition").is_none(),
                "tao trafficLightPosition mutates AppKit's private titlebar during drawRect"
            );
        }
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
