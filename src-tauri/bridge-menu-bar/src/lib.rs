//! Presentation only. The host sends versioned JSON snapshots and receives
//! action IDs; no credentials, provider requests, database, or Tauri state live here.

pub type ActionHandler = extern "C" fn(i32);
pub const REFRESH: i32 = 1;
pub const SETTINGS: i32 = 2;
pub const OPEN_BRIDGE: i32 = 3;
pub const OPENED: i32 = 4;
pub const QUIT: i32 = 5;

#[cfg(target_os = "macos")]
extern "C" {
    fn bridge_menu_bar_create(callback: ActionHandler) -> bool;
    fn bridge_menu_bar_update(bytes: *const u8, count: usize) -> bool;
    fn bridge_menu_bar_show();
    fn bridge_menu_bar_destroy();
}

/// # Safety
/// All functions in this module must be called on the AppKit main thread,
/// after NSApplication initialization. The callback must not unwind or block.
#[cfg(target_os = "macos")]
pub unsafe fn create(callback: ActionHandler) -> bool {
    bridge_menu_bar_create(callback)
}

/// # Safety
/// Must run on the AppKit main thread. Swift copies the bytes synchronously.
#[cfg(target_os = "macos")]
pub unsafe fn update(snapshot: &[u8]) -> bool {
    bridge_menu_bar_update(snapshot.as_ptr(), snapshot.len())
}

/// # Safety
/// Must run on the AppKit main thread after create.
#[cfg(target_os = "macos")]
pub unsafe fn show() {
    bridge_menu_bar_show()
}

/// # Safety
/// Must run on the AppKit main thread, before its event loop stops.
#[cfg(target_os = "macos")]
pub unsafe fn destroy() {
    bridge_menu_bar_destroy()
}
