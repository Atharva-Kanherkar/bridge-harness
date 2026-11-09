//! The unread count on the app icon.
//!
//! Core's `attention/unread_summary` decides what is unread; the webview reads
//! it and sends the total here, because the shell owns the window and window
//! calls from the webview are ACL-gated. macOS draws it on the Dock icon and
//! Linux on launchers that support it; elsewhere setting it fails quietly.

use tauri::{Listener, Manager};

/// Webview to shell: the unread total, a number. Zero clears the badge.
pub const BADGE_EVENT: &str = "bridge-unread-badge";

/// What to set the badge to: `Some(None)` clears it, `Some(Some(n))` shows n,
/// and `None` leaves it alone because the payload was not a count.
pub fn parse_unread_badge(payload: &str) -> Option<Option<i64>> {
    let count = serde_json::from_str::<u32>(payload).ok()?;
    Some((count > 0).then_some(i64::from(count)))
}

pub fn install(app: &tauri::AppHandle) {
    let handle = app.clone();
    let _ = app.listen(BADGE_EVENT, move |event| {
        let Some(count) = parse_unread_badge(event.payload()) else { return };
        let main = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if let Some(window) = main.get_webview_window("main") {
                let _ = window.set_badge_count(count);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unread_badge_clears_at_zero_and_ignores_garbage() {
        assert_eq!(parse_unread_badge("0"), Some(None));
        assert_eq!(parse_unread_badge("3"), Some(Some(3)));
        assert_eq!(parse_unread_badge("-1"), None);
        assert_eq!(parse_unread_badge("\"3\""), None);
        assert_eq!(parse_unread_badge("null"), None);
    }
}
