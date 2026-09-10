//! First-party login window. No Bridge capabilities are assigned to this webview.
use tauri::{Emitter, Manager};
const LABEL: &str = "menu-bar-opencode-login";
pub(super) fn open(
    app: &tauri::AppHandle,
    host: std::sync::Arc<std::sync::OnceLock<crate::HostMode>>,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.set_focus();
        return Ok(());
    }
    let window = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        tauri::WebviewUrl::External("https://opencode.ai/auth".parse().unwrap()),
    )
    .title("Connect OpenCode · Bridge")
    .inner_size(960.0, 720.0)
    .on_navigation(|url| url.scheme() == "https")
    .build()
    .map_err(|_| "Could not open OpenCode sign-in".to_string())?;
    let app = app.clone();
    std::thread::Builder::new().name("opencode-sign-in".into()).spawn(move || {
        // A SPA can change workspace without a page-load event. Poll the
        // explicit login window only; closing it cancels this bounded flow.
        for _ in 0..300 {
            if app.get_webview_window(LABEL).is_none() { return; }
            if let Ok(url) = window.url() {
                if let Some(workspace) = workspace(&url) {
                    let cookie_url = "https://opencode.ai/".parse().unwrap();
                    if let Ok(cookies) = window.cookies_for_url(cookie_url) {
                        let cookie = cookies.iter().filter(|c| ["auth","__Host-auth"].contains(&c.name()))
                            .map(|c| format!("{}={}",c.name(),c.value())).collect::<Vec<_>>().join("; ");
                        if !cookie.is_empty() {
                            match super::call_with_params(&app, &host, bridge_protocol::methods::MethodName::SaveOpencodeUsageSession,
                                Some(serde_json::json!({"cookie":cookie,"workspace":workspace}))) {
                                Ok(_) => { let _ = app.emit("bridge-menu-bar-connected", ()); let _ = app.emit("bridge-menu-bar-connection", "OpenCode connected. Refresh usage to read the selected workspace."); let _ = window.close(); }
                                Err(error) => { let _ = app.emit("bridge-menu-bar-connection", error); }
                            }
                            return;
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        let _ = app.emit("bridge-menu-bar-connection", "OpenCode sign-in timed out. Close the sign-in window and connect again.");
    }).map_err(|_| "Could not start OpenCode sign-in".to_string())?;
    Ok(())
}
fn workspace(url: &tauri::Url) -> Option<String> {
    if url.scheme() != "https"
        || url.host_str() != Some("opencode.ai")
        || url.port_or_known_default() != Some(443)
    {
        return None;
    }
    let mut path = url.path_segments()?;
    if path.next()? != "workspace" {
        return None;
    }
    let id = path.next()?;
    bridge_core::provider_usage::credentials::valid_workspace(id).then(|| id.into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_requires_the_first_party_workspace() {
        assert_eq!(
            workspace(
                &"https://opencode.ai/workspace/wrk_abc/billing"
                    .parse()
                    .unwrap()
            )
            .as_deref(),
            Some("wrk_abc")
        );
        for url in [
            "https://evil.example/workspace/wrk_abc",
            "https://opencode.ai.evil.example/workspace/wrk_abc",
            "http://opencode.ai/workspace/wrk_abc",
            "https://opencode.ai/auth",
            "https://opencode.ai:444/workspace/wrk_abc",
        ] {
            assert!(workspace(&url.parse().unwrap()).is_none());
        }
    }
}
