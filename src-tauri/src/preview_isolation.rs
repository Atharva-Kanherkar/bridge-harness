//! Live personal-app previews render model-written HTML next to real MCP read
//! results. CSP blocks every fetch and navigation, but no engine applies CSP to
//! WebRTC: a view could still send read data out through STUN/TURN traffic.
//! Bridge itself never uses WebRTC, so the app's own webview turns it off.
//!
//! The setting is read back after it is written. The card mounts a preview only
//! after this answers, and allows live reads only when it answered `true`, so
//! an engine or OS without the setting fails closed.

#[cfg(target_os = "macos")]
fn disable(webview: &tauri::Webview) -> bool {
    use std::sync::mpsc;
    use std::time::Duration;
    let (tx, rx) = mpsc::channel();
    let sent = webview.with_webview(move |platform| {
        // Tauri runs this on the main thread. The pointer belongs to the live
        // webview and is never retained or sent across threads.
        drop(tx.send(unsafe { disable_peer_connections(platform.inner().cast()) }));
    });
    sent.is_ok() && rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false)
}

/// WKPreferences' `_peerConnectionEnabled` SPI (macOS 10.13.4+). New documents,
/// including frames created after this call, have no `RTCPeerConnection`.
#[cfg(target_os = "macos")]
unsafe fn disable_peer_connections(webview: *mut objc2::runtime::AnyObject) -> bool {
    unsafe {
        use objc2::runtime::AnyObject;
        use objc2::{msg_send, sel};
        let Some(webview) = webview.as_ref() else { return false };
        let configuration: *mut AnyObject = msg_send![webview, configuration];
        let Some(configuration) = configuration.as_ref() else { return false };
        let preferences: *mut AnyObject = msg_send![configuration, preferences];
        let Some(preferences) = preferences.as_ref() else { return false };
        let setter: bool = msg_send![preferences, respondsToSelector: sel!(_setPeerConnectionEnabled:)];
        let getter: bool = msg_send![preferences, respondsToSelector: sel!(_peerConnectionEnabled)];
        if !setter || !getter {
            return false;
        }
        let _: () = msg_send![preferences, _setPeerConnectionEnabled: false];
        let enabled: bool = msg_send![preferences, _peerConnectionEnabled];
        !enabled
    }
}

#[cfg(not(target_os = "macos"))]
fn disable(_webview: &tauri::Webview) -> bool {
    false
}

/// Turn WebRTC off in the calling webview and report whether it is off.
#[tauri::command(async)]
pub fn isolate_app_previews(webview: tauri::Webview) -> bool {
    disable(&webview)
}

/// Answered by this process in every host mode: a daemon cannot reach the
/// window's webview, so the invoke is never proxied.
pub fn commands(invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
    let handler: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![isolate_app_previews]);
    handler(invoke)
}
