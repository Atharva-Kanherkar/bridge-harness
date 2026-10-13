//! Command-line entry points, for launchers, notifications and the bar.
//!
//! `bridge-deck --open <sessionId>` and friends reach the running app through
//! the single-instance plugin: the second process hands its argv over and
//! exits, and the first turns it into a `bridge-launch` event for the webview.
//! A first launch with arguments keeps the request until the webview asks for
//! it, since the window boots after the shell. `--omarchy setup|remove` never
//! reach the window: they edit files and exit (see `omarchy_setup`).

use serde::Serialize;

/// Shell to webview: one parsed launch request.
pub const LAUNCH_EVENT: &str = "bridge-launch";
/// Webview to shell: "send me the launch request I booted with, if any".
pub const LAUNCH_REQUEST_EVENT: &str = "bridge-launch-request";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OmarchyAction {
    Setup,
    Remove,
    Retint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Launch {
    Open {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    NewChat {
        project: Option<String>,
        /// Text to start the chat with (the quick prompt's submit).
        prompt: Option<String>,
    },
    Quick,
    Approvals,
    MissionControl,
    Omarchy {
        action: OmarchyAction,
        /// `setup --hyprland`: also write the Hyprland rules and binding.
        hyprland: bool,
    },
}

/// The first launch request in `args` (program name excluded). Unknown flags
/// are ignored so a launcher passing extra arguments still opens the app.
pub fn parse<S: AsRef<str>>(args: &[S]) -> Option<Launch> {
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();
    let value_after = |flag: &str| {
        args.iter()
            .position(|arg| *arg == flag)
            .and_then(|index| args.get(index + 1))
            .filter(|value| !value.starts_with("--") && !value.is_empty())
            .map(|value| value.to_string())
    };
    for arg in &args {
        match *arg {
            "--open" => {
                if let Some(session_id) = value_after("--open") {
                    return Some(Launch::Open { session_id });
                }
            }
            "--new-chat" => return Some(Launch::NewChat { project: value_after("--project"), prompt: None }),
            "--quick" => return Some(Launch::Quick),
            "--approvals" => return Some(Launch::Approvals),
            "--mission-control" => return Some(Launch::MissionControl),
            "--omarchy" => {
                let action = match value_after("--omarchy").as_deref() {
                    Some("setup") => OmarchyAction::Setup,
                    Some("remove") => OmarchyAction::Remove,
                    Some("retint") => OmarchyAction::Retint,
                    _ => continue,
                };
                return Some(Launch::Omarchy { action, hyprland: args.contains(&"--hyprland") });
            }
            _ => {}
        }
    }
    None
}

/// Is a desktop copy already running for this data dir? Reads the same
/// advisory lock `DesktopLease` holds, without taking it.
pub fn desktop_running(data_dir: &std::path::Path) -> bool {
    let Ok(file) = std::fs::File::open(data_dir.join("desktop.lock")) else { return false };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(std::fs::TryLockError::WouldBlock) => true,
        Err(std::fs::TryLockError::Error(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_point_parses() {
        assert_eq!(parse(&["--open", "abc-123"]), Some(Launch::Open { session_id: "abc-123".into() }));
        assert_eq!(parse(&["--new-chat"]), Some(Launch::NewChat { project: None, prompt: None }));
        assert_eq!(
            parse(&["--new-chat", "--project", "/home/me/Work/app"]),
            Some(Launch::NewChat { project: Some("/home/me/Work/app".into()), prompt: None })
        );
        assert_eq!(parse(&["--quick"]), Some(Launch::Quick));
        assert_eq!(parse(&["--approvals"]), Some(Launch::Approvals));
        assert_eq!(parse(&["--mission-control"]), Some(Launch::MissionControl));
        assert_eq!(parse(&["--omarchy", "setup"]), Some(Launch::Omarchy { action: OmarchyAction::Setup, hyprland: false }));
        assert_eq!(parse(&["--omarchy", "setup", "--hyprland"]), Some(Launch::Omarchy { action: OmarchyAction::Setup, hyprland: true }));
        assert_eq!(parse(&["--omarchy", "remove"]), Some(Launch::Omarchy { action: OmarchyAction::Remove, hyprland: false }));
        assert_eq!(parse(&["--omarchy", "retint"]), Some(Launch::Omarchy { action: OmarchyAction::Retint, hyprland: false }));
    }

    #[test]
    fn malformed_and_unknown_arguments_open_nothing() {
        let none: [&str; 0] = [];
        assert_eq!(parse(&none), None);
        assert_eq!(parse(&["--open"]), None, "a flag without its value");
        assert_eq!(parse(&["--open", "--quick"]), Some(Launch::Quick), "a flag is never taken as a value");
        assert_eq!(parse(&["--omarchy", "explode"]), None);
        assert_eq!(parse(&["--psn_0_12345", "--unknown"]), None);
        assert_eq!(parse(&["--some-launcher-flag", "--approvals"]), Some(Launch::Approvals));
    }

    #[test]
    fn the_webview_payload_is_camel_cased_and_tagged() {
        let open = serde_json::to_value(Launch::Open { session_id: "s1".into() }).unwrap();
        assert_eq!(open, serde_json::json!({ "kind": "open", "sessionId": "s1" }));
        let chat = serde_json::to_value(Launch::NewChat { project: Some("/p".into()), prompt: Some("hi".into()) }).unwrap();
        assert_eq!(chat, serde_json::json!({ "kind": "new-chat", "project": "/p", "prompt": "hi" }));
        assert_eq!(serde_json::to_value(Launch::MissionControl).unwrap(), serde_json::json!({ "kind": "mission-control" }));
    }

    #[test]
    fn a_held_desktop_lock_means_bridge_is_running() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!desktop_running(dir.path()), "no lock file");
        let lock = std::fs::File::create(dir.path().join("desktop.lock")).unwrap();
        assert!(!desktop_running(dir.path()), "an unheld lock");
        lock.lock().unwrap();
        assert!(desktop_running(dir.path()));
        lock.unlock().unwrap();
        assert!(!desktop_running(dir.path()));
    }
}
