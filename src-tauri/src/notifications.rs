//! Native Linux notifications with action buttons.
//!
//! `tauri-plugin-notification` posts plain banners. On Linux an approval
//! deserves more: Allow once / Decline / Open on the notification itself, the
//! way Omarchy's own alerts work. The webview decides what to notify about
//! (it already diffs session status for attention) and asks the shell to post
//! it; a clicked action comes back to the webview, which resolves it through
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

/// Listen for the webview's requests. Linux only; elsewhere the webview keeps
/// using the notification plugin.
pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
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
    #[cfg(not(target_os = "linux"))]
    let _ = app;
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
    fn the_webview_payload_is_camel_cased() {
        let parsed: NotifyRequest = serde_json::from_str(r#"{"title":"Needs you","body":"Run tests?","sessionId":"s1","approvalEventId":42}"#).unwrap();
        assert_eq!(parsed, NotifyRequest { title: "Needs you".into(), body: "Run tests?".into(), session_id: Some("s1".into()), approval_event_id: Some(42) });
        let taken = serde_json::to_value(ActionTaken { action: "decline", session_id: "s1".into(), event_id: Some(42) }).unwrap();
        assert_eq!(taken, serde_json::json!({ "action": "decline", "sessionId": "s1", "eventId": 42 }));
    }
}
