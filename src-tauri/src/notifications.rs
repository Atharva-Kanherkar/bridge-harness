//! Native notifications with action buttons, on Linux and macOS.
//!
//! `tauri-plugin-notification` posts plain banners. An approval deserves more:
//! Allow once / Decline / Open on the notification itself, the way Omarchy's
//! own alerts work on Linux and Notification Center's actions work on macOS
//! (`bridge-menu-bar/swift/Notifications.swift`). Core's attention ledger decides what to
//! notify about; the webview follows it and asks the shell to post it; a
//! clicked action comes back to the webview, which resolves it through
//! `bridgeApi.resolveApproval`, the same call the in-app card makes. The
//! policy engine stays the only authority, and the core's atomic claim makes
//! a second resolution of the same approval a no-op.

use serde::{Deserialize, Serialize};

/// Webview to shell: post this notification.
pub const NOTIFY_EVENT: &str = "bridge-native-notify";
/// Shell to webview: the user clicked one of its actions.
pub const ACTION_EVENT: &str = "bridge-notification-action";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
    /// Present when the notification is a pending approval Bridge can resolve.
    pub approval_event_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionTaken {
    pub action: &'static str,
    pub session_id: String,
    pub event_id: Option<i64>,
}

/// The buttons a request gets: approvals can be answered in place, anything
/// else can only be opened.
pub fn actions(request: &NotifyRequest) -> Vec<(&'static str, &'static str)> {
    match (&request.session_id, request.approval_event_id) {
        (Some(_), Some(_)) => vec![("allow-once", "Allow once"), ("decline", "Decline"), ("open", "Open")],
        (Some(_), None) => vec![("open", "Open")],
        _ => vec![],
    }
}

/// A click on a macOS banner, as the native center reports it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeClick {
    pub action: String,
    pub session_id: Option<String>,
    pub approval_event_id: Option<i64>,
}

/// What a macOS click means: the ids it carried, under the same rules as a
/// Linux click, so a forged or stale allow without an approval id does nothing.
pub fn native_action_taken(click: &NativeClick) -> Option<ActionTaken> {
    let request = NotifyRequest {
        title: String::new(),
        body: String::new(),
        session_id: click.session_id.clone(),
        approval_event_id: click.approval_event_id,
    };
    action_taken(&request, &click.action)
}

/// What a clicked action means. `default` is a click on the body.
pub fn action_taken(request: &NotifyRequest, action: &str) -> Option<ActionTaken> {
    let session_id = request.session_id.clone()?;
    let action = match action {
        "allow-once" if request.approval_event_id.is_some() => "allow-once",
        "decline" if request.approval_event_id.is_some() => "decline",
        "open" | "default" => "open",
        _ => return None,
    };
    Some(ActionTaken { action, session_id, event_id: request.approval_event_id })
}

/// Listen for the webview's requests. Linux posts through notify-rust, macOS
/// through the native center, or plain plugin banners without an app bundle.
/// On macOS, call this on the main thread during launch.
pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    #[cfg(target_os = "macos")]
    {
        use tauri::Listener;
        let native = macos::install(app.clone());
        let handle = app.clone();
        let _ = app.listen(NOTIFY_EVENT, move |event| {
            let Ok(request) = serde_json::from_str::<NotifyRequest>(event.payload()) else { return };
            if native {
                // The webview's JSON is the request Swift reads.
                let bytes = event.payload().as_bytes().to_vec();
                bridge_menu_bar::run_on_main_thread(move || {
                    // SAFETY: Swift copies the bytes before returning.
                    let _ = unsafe { bridge_menu_bar::notifications_post(&bytes) };
                });
            } else {
                // `tauri dev` runs a bare binary, which the center refuses.
                use tauri_plugin_notification::NotificationExt;
                let _ = handle.notification().builder().title(&request.title).body(&request.body).show();
            }
        });
    }
    #[cfg(target_os = "linux")]
    {
        use tauri::Listener;
        let handle = app.clone();
        let _ = app.listen(NOTIFY_EVENT, move |event| {
            let Ok(request) = serde_json::from_str::<NotifyRequest>(event.payload()) else { return };
            let handle = handle.clone();
            // `wait_for_action` blocks until the notification closes.
            let _ = std::thread::Builder::new().name("bridge-notification".into()).spawn(move || show(&handle, request));
        });
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = app;
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{native_action_taken, ActionTaken, NativeClick, ACTION_EVENT};
    use std::ffi::{c_char, CStr};
    use std::sync::OnceLock;
    use tauri::{Emitter, Manager};

    type OnClick = Box<dyn Fn(ActionTaken) + Send + Sync>;
    static ON_CLICK: OnceLock<OnClick> = OnceLock::new();

    /// Swift calls this with the click's JSON, on any thread. It must never
    /// unwind into Swift.
    extern "C" fn clicked(json: *const c_char) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if json.is_null() {
                return;
            }
            // SAFETY: Swift passes a NUL-terminated string valid for this call.
            let Ok(text) = unsafe { CStr::from_ptr(json) }.to_str() else { return };
            let Ok(click) = serde_json::from_str::<NativeClick>(text) else { return };
            let Some(taken) = native_action_taken(&click) else { return };
            if let Some(on_click) = ON_CLICK.get() {
                on_click(taken);
            }
        }));
    }

    /// Install the native center. False without an app bundle.
    pub fn install<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> bool {
        let handle = app.clone();
        let _ = ON_CLICK.set(Box::new(move |taken: ActionTaken| {
            if taken.action == "open" {
                let main = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    if let Some(window) = main.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                    }
                });
            }
            let _ = handle.emit(ACTION_EVENT, taken);
        }));
        // SAFETY: called from `setup`, on the main thread during launch; the
        // handler never unwinds.
        unsafe { bridge_menu_bar::notifications_install(clicked) }
    }
}

#[cfg(target_os = "linux")]
fn show<R: tauri::Runtime>(app: &tauri::AppHandle<R>, request: NotifyRequest) {
    use notify_rust::{Notification, Urgency};
    use tauri::{Emitter, Manager};
    let mut notification = Notification::new();
    notification
        .appname("Bridge")
        .icon(crate::linux_desktop::APP_ID)
        .summary(&request.title)
        .body(&request.body)
        // Approvals are urgent: critical passes Omarchy's do-not-disturb.
        .urgency(if request.approval_event_id.is_some() { Urgency::Critical } else { Urgency::Normal });
    for (id, label) in actions(&request) {
        notification.action(id, label);
    }
    if request.session_id.is_some() {
        notification.action("default", "Open");
    }
    let Ok(handle) = notification.show() else { return };
    handle.wait_for_action(|action| {
        let Some(taken) = action_taken(&request, action) else { return };
        if taken.action == "open" {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
        let _ = app.emit(ACTION_EVENT, taken);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(session: Option<&str>, approval: Option<i64>) -> NotifyRequest {
        NotifyRequest { title: "t".into(), body: "b".into(), session_id: session.map(str::to_owned), approval_event_id: approval }
    }

    #[test]
    fn approvals_get_allow_decline_and_open_and_the_rest_only_open() {
        let ids = |r: &NotifyRequest| actions(r).into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        assert_eq!(ids(&request(Some("s"), Some(7))), ["allow-once", "decline", "open"]);
        assert_eq!(ids(&request(Some("s"), None)), ["open"]);
        assert!(ids(&request(None, None)).is_empty());
    }

    #[test]
    fn clicked_actions_map_to_what_the_webview_resolves() {
        let approval = request(Some("s"), Some(7));
        assert_eq!(action_taken(&approval, "allow-once"), Some(ActionTaken { action: "allow-once", session_id: "s".into(), event_id: Some(7) }));
        assert_eq!(action_taken(&approval, "decline").unwrap().action, "decline");
        assert_eq!(action_taken(&approval, "default").unwrap().action, "open");
        assert_eq!(action_taken(&approval, "__closed"), None, "dismissing resolves nothing");
        // Without an approval id, an allow click (a stale or forged action) does nothing.
        assert_eq!(action_taken(&request(Some("s"), None), "allow-once"), None);
        assert_eq!(action_taken(&request(None, Some(7)), "open"), None);
    }

    #[test]
    fn a_macos_click_maps_like_a_linux_click() {
        let click = |json: &str| native_action_taken(&serde_json::from_str::<NativeClick>(json).unwrap());
        // The JSON shapes Notifications.swift sends.
        assert_eq!(
            click(r#"{"action":"allow-once","approvalEventId":42,"sessionId":"s1"}"#),
            Some(ActionTaken { action: "allow-once", session_id: "s1".into(), event_id: Some(42) })
        );
        assert_eq!(click(r#"{"action":"decline","approvalEventId":42,"sessionId":"s1"}"#).unwrap().action, "decline");
        assert_eq!(click(r#"{"action":"default","approvalEventId":42,"sessionId":"s1"}"#).unwrap().action, "open");
        assert_eq!(click(r#"{"action":"open","sessionId":"s2"}"#), Some(ActionTaken { action: "open", session_id: "s2".into(), event_id: None }));
        assert_eq!(click(r#"{"action":"allow-once","sessionId":"s2"}"#), None, "an allow without an approval id does nothing");
        assert_eq!(click(r#"{"action":"open"}"#), None, "nothing to open");
        assert_eq!(click(r#"{"action":"snooze","sessionId":"s1"}"#), None);
    }

    #[test]
    fn the_webview_payload_is_camel_cased() {
        let parsed: NotifyRequest = serde_json::from_str(r#"{"title":"Needs you","body":"Run tests?","sessionId":"s1","approvalEventId":42}"#).unwrap();
        assert_eq!(parsed, NotifyRequest { title: "Needs you".into(), body: "Run tests?".into(), session_id: Some("s1".into()), approval_event_id: Some(42) });
        let taken = serde_json::to_value(ActionTaken { action: "decline", session_id: "s1".into(), event_id: Some(42) }).unwrap();
        assert_eq!(taken, serde_json::json!({ "action": "decline", "sessionId": "s1", "eventId": 42 }));
    }
}
