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
    fn bridge_menu_bar_schedule(
        callback: extern "C" fn(*mut std::ffi::c_void),
        context: *mut std::ffi::c_void,
    );
}

/// Delivers an owned operation during either normal app operation or an open
/// menu's tracking loop. Swift's one-shot scheduler calls the trampoline once.
#[cfg(target_os = "macos")]
pub fn run_on_main_thread(operation: impl FnOnce() + Send + 'static) {
    type Operation = Box<dyn FnOnce() + Send>;
    extern "C" fn run(context: *mut std::ffi::c_void) {
        // The outer Box gives the trait object a thin pointer for the C ABI.
        let operation = unsafe { Box::from_raw(context.cast::<Operation>()) };
        // Never unwind a Rust panic through Swift's non-unwinding callback.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation));
    }
    let operation: Box<Operation> = Box::new(Box::new(operation));
    unsafe { bridge_menu_bar_schedule(run, Box::into_raw(operation).cast()) }
}

/// # Safety
/// Must be called on the AppKit main thread,
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
