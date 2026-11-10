//! Native menu host. UI actions cross a bounded queue; all provider, ledger,
//! and preference operations use the same versioned backend methods as React.
mod connect;
use crate::{diagnostics, HostMode};
use bridge_protocol::{
    messages::{MenuBarProvider, MenuBarSettings, ProviderUsageOverviews},
    methods::MethodName,
};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Emitter, Listener, Manager};

#[derive(Clone, Copy)]
enum Work {
    Refresh,
    Select(MenuBarProvider),
    RefreshInterval(u64),
    Opened,
    SettingsChanged,
    Snapshot,
    /// Unread chats changed: reread them alone, with no usage work.
    Attention,
    Stop,
}

struct NativeHost {
    app: tauri::AppHandle,
    send: mpsc::SyncSender<Work>,
}
static NATIVE: OnceLock<NativeHost> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Presentation {
    settings: MenuBarSettings,
    usage: Option<ProviderUsageOverviews>,
    refreshing: bool,
    error: Option<String>,
    /// Unread chats, from `attention/unread_summary`. `None` until the first
    /// read succeeds; a failed read keeps the last counts.
    attention: Option<AttentionCounts>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AttentionCounts {
    total: u32,
    needs_you: u32,
}

/// A read-only ledger query: it never starts a provider or a usage refresh.
fn read_attention(app: &tauri::AppHandle, host: &OnceLock<HostMode>, presentation: &mut Presentation) {
    let summary = call(app, host, MethodName::UnreadSummary)
        .and_then(|v| serde_json::from_value::<bridge_protocol::messages::UnreadSummary>(v).map_err(|e| e.to_string()));
    if let Ok(summary) = summary {
        presentation.attention = Some(AttentionCounts { total: summary.total, needs_you: summary.needs_you });
    }
}

fn call(
    app: &tauri::AppHandle,
    host: &OnceLock<HostMode>,
    method: MethodName,
) -> Result<Value, String> {
    call_with_params(app, host, method, None)
}

fn call_with_params(
    app: &tauri::AppHandle,
    host: &OnceLock<HostMode>,
    method: MethodName,
    params: Option<Value>,
) -> Result<Value, String> {
    match host.get() {
        Some(HostMode::Daemon(runtime)) => runtime.proxy.call(method, params),
        Some(HostMode::Embedded) => {
            let core = app.state::<Arc<bridge_core::BridgeCore>>();
            let value = match method {
                MethodName::GetMenuBarSettings => serde_json::to_value(
                    bridge_core::api::get_menu_bar_settings(&core).map_err(|e| e.to_string())?,
                ),
                MethodName::UnreadSummary => serde_json::to_value(
                    bridge_core::api::unread_summary(&core).map_err(|e| e.to_string())?,
                ),
                MethodName::GetProviderUsageOverviews => serde_json::to_value(
                    bridge_core::api::get_provider_usage_overviews(&core)
                        .map_err(|e| e.to_string())?,
                ),
                MethodName::RefreshProviderUsageOverviews => serde_json::to_value(
                    bridge_core::api::refresh_provider_usage_overviews(&core)
                        .map_err(|e| e.to_string())?,
                ),
                MethodName::RefreshProviderUsageOverviewsInteractive => serde_json::to_value(
                    bridge_core::api::refresh_provider_usage_overviews_interactive(&core)
                        .map_err(|e| e.to_string())?,
                ),
                MethodName::SaveOpencodeUsageSession => {
                    let params: bridge_protocol::messages::SaveOpencodeUsageSessionParams =
                        serde_json::from_value(params.ok_or("Missing OpenCode session")?)
                            .map_err(|_| "Invalid OpenCode session")?;
                    bridge_core::api::save_opencode_usage_session(
                        &core,
                        &params.cookie,
                        &params.workspace,
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(Value::Null)
                }
                MethodName::SaveMenuBarSettings => {
                    let params: bridge_protocol::messages::SaveMenuBarSettingsParams =
                        serde_json::from_value(params.ok_or("Missing menu settings")?)
                            .map_err(|e| e.to_string())?;
                    serde_json::to_value(
                        bridge_core::api::save_menu_bar_settings(&core, &params.settings)
                            .map_err(|e| e.to_string())?,
                    )
                }
                _ => return Err("Unsupported menu request".into()),
            };
            value.map_err(|e| e.to_string())
        }
        _ => Err("Bridge is still starting".into()),
    }
}

#[cfg(target_os = "macos")]
extern "C" fn action(action: i32) {
    let Some(native) = NATIVE.get() else {
        return;
    };
    match action {
        bridge_menu_bar::REFRESH => {
            let _ = native.send.try_send(Work::Refresh);
        }
        bridge_menu_bar::OPENED => {
            let _ = native.send.try_send(Work::Opened);
        }
        bridge_menu_bar::SETTINGS | bridge_menu_bar::OPEN_BRIDGE => {
            if let Some(window) = native.app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            if action == bridge_menu_bar::SETTINGS {
                let _ = native.app.emit("bridge-menu-bar-settings", ());
            }
        }
        bridge_menu_bar::QUIT => native.app.exit(0),
        100..=103 => {
            let _ = native
                .send
                .try_send(Work::Select(MenuBarProvider::ALL[(action - 100) as usize]));
        }
        200..=204 => {
            let seconds = [0, 60, 300, 900, 1800][(action - 200) as usize];
            let _ = native.send.try_send(Work::RefreshInterval(seconds));
        }
        _ => {}
    }
}

fn publish(app: &tauri::AppHandle, presentation: &Presentation) {
    if let Some(usage) = &presentation.usage {
        let _ = app.emit("bridge-provider-usage-overviews", usage);
        if let Some(codex) = usage.providers.iter().find(|p| p.provider == "codex") {
            let _ = app.emit("bridge-usage-overview", codex);
        }
    }
    publish_native(presentation);
}

/// The native item only. An unread change has no usage news for the webview.
fn publish_native(presentation: &Presentation) {
    let Ok(bytes) = serde_json::to_vec(presentation) else {
        return;
    };
    #[cfg(target_os = "macos")]
    bridge_menu_bar::run_on_main_thread(move || {
        let result = diagnostics::native_boundary(|| unsafe { bridge_menu_bar::update(&bytes) });
        if !matches!(result, Ok(true)) {
            diagnostics::record("Menu Bar rejected a presentation snapshot");
        }
    });
    #[cfg(not(target_os = "macos"))]
    let _ = bytes;
}

/// Called after selecting the backend, on AppKit's main thread. The old meter
/// remains an internal escape hatch until supported-OS parity is complete.
pub fn install(app: &tauri::App, host: Arc<OnceLock<HostMode>>) -> Result<bool, String> {
    if std::env::var("BRIDGE_LEGACY_METER").as_deref() == Ok("1") {
        return Ok(false);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, host);
        Ok(false)
    }
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = mpsc::sync_channel(8);
        let handle = app.handle().clone();
        NATIVE
            .set(NativeHost {
                app: handle.clone(),
                send: send.clone(),
            })
            .map_err(|_| "Menu Bar already initialized")?;
        if !unsafe { bridge_menu_bar::create(action) } {
            return Err("Native Menu Bar initialization failed".into());
        }
        let signal = send.clone();
        app.listen("bridge-menu-bar-settings-changed", move |_| {
            let _ = signal.try_send(Work::SettingsChanged);
        });
        let connection_app = handle.clone();
        app.listen("bridge-menu-bar-connect-opencode", move |_| {
            let message = match connect::open_default_browser() {
                Ok(()) => "OpenCode sign-in opened in your default browser. Copy the Go API key, then paste it here.".into(),
                Err(error) => error,
            };
            let _ = connection_app.emit("bridge-menu-bar-connection", message);
        });
        let signal = send.clone();
        app.listen("account-usage", move |_| {
            let _ = signal.try_send(Work::Snapshot);
        });
        // The webview sends the unread total for the Dock badge whenever it
        // changes; the same moment is when the menu bar should reread it.
        let signal = send.clone();
        app.listen(crate::dock_badge::BADGE_EVENT, move |_| {
            let _ = signal.try_send(Work::Attention);
        });
        let worker = std::thread::Builder::new()
            .name("menu-bar-host".into())
            .spawn(move || {
                let mut presentation = Presentation {
                    settings: MenuBarSettings::default(),
                    usage: None,
                    refreshing: false,
                    error: None,
                    attention: None,
                };
                let mut last_refresh: Option<Instant> = None;
                let mut last_snapshot = Instant::now();
                let mut next = Work::SettingsChanged;
                // When the next quiet-period snapshot is due. Only a full pass
                // moves it, so unread wakes cannot starve the usage tick.
                let mut tick = Instant::now() + Duration::from_secs(30);
                loop {
                    if matches!(next, Work::Stop) {
                        break;
                    }
                    read_attention(&handle, &host, &mut presentation);
                    if matches!(next, Work::Attention) {
                        // Unread changes come often while agents work; none of
                        // them is a reason to touch settings or usage.
                        publish_native(&presentation);
                        next = match receive.recv_timeout(tick.saturating_duration_since(Instant::now())) {
                            Ok(work) => work,
                            Err(mpsc::RecvTimeoutError::Timeout) => Work::Snapshot,
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        };
                        continue;
                    }
                    // Settings are read before every refresh, so a hidden or
                    // disabled provider never starts another scheduled probe.
                    let previous_settings = presentation.settings.clone();
                    let mut selection_error = None;
                    let settings_error = match call(&handle, &host, MethodName::GetMenuBarSettings)
                        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
                    {
                        Ok(settings) => {
                            presentation.settings = settings;
                            None
                        }
                        Err(error) => Some(error),
                    };
                    if matches!(next, Work::Select(_) | Work::RefreshInterval(_)) {
                        let selected = match next { Work::Select(provider) => Some(provider), _ => None };
                        if selected.is_none_or(|provider| presentation.settings.provider_visible(provider)) {
                            let mut settings = presentation.settings.clone();
                            match next {
                                Work::Select(provider) => settings.selected_provider = provider,
                                Work::RefreshInterval(seconds) => settings.refresh_seconds = seconds,
                                _ => {}
                            }
                            match call_with_params(
                                &handle,
                                &host,
                                MethodName::SaveMenuBarSettings,
                                Some(serde_json::json!({"settings":settings})),
                            ) {
                                Ok(value) => {
                                    if let Ok(settings) = serde_json::from_value(value) {
                                        presentation.settings = settings;
                                    }
                                }
                                Err(error) => selection_error = Some(error),
                            }
                            let _ = handle.emit("bridge-menu-bar-settings-changed", ());
                        }
                    }
                    let enabled = settings_error.is_none()
                        && (presentation.settings.enabled || matches!(next, Work::Refresh))
                        && MenuBarProvider::ALL
                            .into_iter()
                            .any(|p| presentation.settings.provider_enabled(p));
                    let interval = presentation.settings.refresh_seconds;
                    let due = interval > 0
                        && last_refresh.is_none_or(|t| t.elapsed().as_secs() >= interval);
                    let newly_enabled = MenuBarProvider::ALL.into_iter().any(|p| {
                        presentation.settings.provider_enabled(p)
                            && !previous_settings.provider_enabled(p)
                    });
                    let should_refresh = enabled
                        && (matches!(next, Work::Refresh)
                            || due
                            || (newly_enabled && interval > 0));
                    if should_refresh {
                        // Paint the retained snapshot before credential/network work.
                        // A slow provider must not leave the first menu empty.
                        if presentation.usage.is_none() {
                            if let Ok(usage) = call(&handle, &host, MethodName::GetProviderUsageOverviews)
                                .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
                            {
                                presentation.usage = Some(usage);
                            }
                        }
                        presentation.refreshing = true;
                        publish(&handle, &presentation);
                    }
                    // Expiry and recorded tokens are re-evaluated at most every
                    // 30 seconds without launching any provider request.
                    let should_snapshot = should_refresh
                        || presentation.usage.is_none()
                        || last_snapshot.elapsed().as_secs() >= 30
                        || matches!(next, Work::Opened | Work::SettingsChanged | Work::Select(_) | Work::RefreshInterval(_));
                    if should_snapshot {
                        let method = if should_refresh {
                            if matches!(next, Work::Refresh) {
                                MethodName::RefreshProviderUsageOverviewsInteractive
                            } else {
                                MethodName::RefreshProviderUsageOverviews
                            }
                        } else {
                            MethodName::GetProviderUsageOverviews
                        };
                        match call(&handle, &host, method)
                            .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
                        {
                            Ok(usage) => {
                                presentation.usage = Some(usage);
                                presentation.error = None;
                            }
                            Err(error) => presentation.error = Some(error),
                        }
                        last_snapshot = Instant::now();
                    }
                    if should_refresh {
                        last_refresh = Some(Instant::now());
                    }
                    if let Some(error) = settings_error.or(selection_error) {
                        presentation.error = Some(error);
                    }
                    presentation.refreshing = false;
                    publish(&handle, &presentation);
                    tick = Instant::now() + Duration::from_secs(30);
                    next = match receive.recv_timeout(Duration::from_secs(30)) {
                        Ok(work) => work,
                        Err(mpsc::RecvTimeoutError::Timeout) => Work::Snapshot,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                }
            });
        if let Err(error) = worker {
            unsafe {
                bridge_menu_bar::destroy();
            }
            return Err(error.to_string());
        }
        ACTIVE.store(true, Ordering::Release);
        Ok(true)
    }
}

pub fn show() -> bool {
    #[cfg(target_os = "macos")]
    if ACTIVE.load(Ordering::Acquire) {
        unsafe {
            bridge_menu_bar::show();
        }
        return true;
    }
    false
}

pub fn shutdown() {
    if let Some(native) = NATIVE.get() {
        let _ = native.send.try_send(Work::Stop);
    }
    #[cfg(target_os = "macos")]
    if ACTIVE.swap(false, Ordering::AcqRel) {
        unsafe {
            bridge_menu_bar::destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Swift tests decode this same fixture, so both sides agree on the
    /// unread counts' wire names.
    #[test]
    fn the_snapshot_carries_unread_counts_as_the_native_fixture_does() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../bridge-protocol/tests/fixtures/menu-bar-presentation.json")).unwrap();
        let presentation = Presentation {
            settings: serde_json::from_value(fixture["settings"].clone()).unwrap(),
            usage: serde_json::from_value(fixture["usage"].clone()).unwrap(),
            refreshing: false,
            error: None,
            attention: Some(AttentionCounts { total: 4, needs_you: 1 }),
        };
        assert_eq!(serde_json::to_value(&presentation).unwrap(), fixture);
        let unknown = Presentation { attention: None, ..presentation };
        assert_eq!(serde_json::to_value(&unknown).unwrap()["attention"], Value::Null);
    }
}
