//! The quick prompt: a small Bridge composer that floats over whatever you
//! are doing, toggled by `bridge-deck --quick` (Omarchy binds it to
//! SUPER + SHIFT + CTRL + B). It owns no session. Submitting hands the text to
//! the main window, which starts the chat exactly as its welcome composer
//! would, and the quick window hides; Esc hides it too. Either way the work
//! keeps running in the main window.

use tauri::{Emitter, Listener, Manager};

pub const LABEL: &str = "quick";
/// Its title, which the Hyprland rule written by `--omarchy setup` matches.
pub const TITLE: &str = "Bridge Quick";
/// Quick window to shell: hide me (Esc, or after a submit).
pub const HIDE_EVENT: &str = "bridge-quick-hide";
/// Quick window to shell: start a chat with this text.
pub const SUBMIT_EVENT: &str = "bridge-quick-submit";

/// The text a submit carries, if it carries any.
pub fn submitted_text(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let text = value.get("text")?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Show the quick window, building it on first use, or hide it if showing.
pub fn toggle<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(window) = app.get_webview_window(LABEL) {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            let _ = window.show();
            let _ = window.set_focus();
        }
        return;
    }
    let builder = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App("index.html?window=quick".into()))
        .title(TITLE)
        .inner_size(864.0, 470.0)
        .center()
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        // Hyprland's rule floats and frames it; elsewhere it is a plain panel.
        .decorations(false);
    if let Ok(window) = builder.build() {
        let _ = window.set_focus();
    }
}

pub fn install<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let hide_handle = app.clone();
    let _ = app.listen(HIDE_EVENT, move |_| {
        if let Some(window) = hide_handle.get_webview_window(LABEL) {
            let _ = window.hide();
        }
    });
    let submit_handle = app.clone();
    let _ = app.listen(SUBMIT_EVENT, move |event| {
        let Some(text) = submitted_text(event.payload()) else { return };
        if let Some(window) = submit_handle.get_webview_window(LABEL) {
            let _ = window.hide();
        }
        let launch = crate::launch::Launch::NewChat { project: None, prompt: Some(text) };
        let _ = submit_handle.emit_to("main", crate::launch::LAUNCH_EVENT, launch);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_submit_carries_trimmed_text_or_nothing() {
        assert_eq!(submitted_text(r#"{"text":"  fix the flaky test  "}"#).as_deref(), Some("fix the flaky test"));
        assert_eq!(submitted_text(r#"{"text":"   "}"#), None);
        assert_eq!(submitted_text(r#"{"nope":1}"#), None);
        assert_eq!(submitted_text("not json"), None);
    }

    #[test]
    fn the_title_matches_the_hyprland_rule_setup_writes() {
        assert!(crate::omarchy_setup::tests_support::hypr_module_for_test().contains(&format!("title = \"^{TITLE}$\"")));
    }
}
