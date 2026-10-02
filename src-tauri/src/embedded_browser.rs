//! Desktop-owned browser views. Remote pages get no Tauri capabilities.
//!
//! This is deliberately a shell plugin: a native child view has no daemon
//! representation, and its commands must not be sent through the core protocol.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder},
    Manager, Webview, WebviewUrl,
};

const PAGE_SCRIPT: &str = include_str!("browser_page.js");
const MAX_TABS: usize = 64;
const MAX_URL: usize = 8192;
type TabKey = (String, String);
type Result<T> = std::result::Result<T, String>;

#[derive(Default, Clone)]
struct Registry(Arc<Mutex<HashMap<TabKey, Tab>>>);
#[derive(Clone)]
struct Tab {
    label: String,
    state: Arc<Mutex<TabState>>,
}
#[derive(Default)]
struct TabState {
    snapshot: Snapshot,
    requested_url: Option<String>,
    pending_since: Option<Instant>,
    finished: bool,
    script_revision: Option<u64>,
    previous_url: String,
    committed_url: Option<String>,
    inspect_epoch: u64,
    inspecting: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    url: String,
    title: String,
    can_go_back: bool,
    can_go_forward: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    back_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    forward_url: Option<String>,
    loading: bool,
    navigation_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection: Option<Selection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    popup_url: Option<String>,
    cancelled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    shortcut: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    history_action: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Selection {
    selector: String,
    snippet: String,
    bounds: Bounds,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}
#[derive(Default, Deserialize)]
struct PageSnapshot {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    revision: u64,
    selection: Option<Selection>,
    #[serde(default)]
    cancelled: bool,
    shortcut: Option<String>,
    #[serde(rename = "historyAction")]
    history_action: Option<String>,
    #[serde(rename = "popupUrl")]
    popup_url: Option<String>,
}

pub(crate) fn trusted_shell(label: &str) -> bool {
    label == "main" || label == crate::meter_tray::PANEL_LABEL
}

fn controller(caller: &Webview) -> Result<()> {
    if caller.label() != "main" {
        return Err("Browser controls are only available to the Bridge window".into());
    }
    Ok(())
}
fn key(session_id: String, tab_id: String) -> Result<TabKey> {
    if [&session_id, &tab_id]
        .iter()
        .any(|id| id.is_empty() || id.len() > 200 || id.chars().any(char::is_control))
    {
        return Err("Invalid browser task or tab identifier".into());
    }
    Ok((session_id, tab_id))
}
fn parse_url(value: &str) -> Result<tauri::Url> {
    if value.len() > MAX_URL {
        return Err("Browser address is too long".into());
    }
    let url = tauri::Url::parse(if value.is_empty() {
        "about:blank"
    } else {
        value
    })
    .map_err(|_| "Invalid browser address")?;
    if !allowed_url(&url) || !url.username().is_empty() || url.password().is_some() {
        return Err("Only HTTP and HTTPS pages can be opened in the browser".into());
    }
    Ok(url)
}
fn allowed_url(url: &tauri::Url) -> bool {
    ((matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        || url.as_str() == "about:blank")
        && url.username().is_empty()
        && url.password().is_none()
}
fn begin_navigation(state: &mut TabState, requested_url: Option<String>) {
    state.inspect_epoch = state.inspect_epoch.saturating_add(1);
    state.inspecting = false;
    state.previous_url = state.snapshot.url.clone();
    state.committed_url = None;
    state.snapshot.navigation_id = state.snapshot.navigation_id.saturating_add(1);
    state.snapshot.selection = None;
    state.snapshot.cancelled = true;
    state.snapshot.loading = true;
    state.snapshot.error = None;
    state.pending_since = Some(Instant::now());
    if let Some(url) = requested_url.as_ref() {
        state.snapshot.url = url.clone();
        state.snapshot.title.clear();
    }
    state.requested_url = requested_url;
    state.finished = false;
    state.script_revision = None;
    if state.requested_url.as_deref() == Some("about:blank") {
        finish_blank_navigation(state);
    }
}
fn blank_target(state: &TabState) -> bool {
    state
        .requested_url
        .as_deref()
        .unwrap_or(&state.snapshot.url)
        == "about:blank"
}
fn finish_blank_navigation(state: &mut TabState) {
    state.snapshot.url = "about:blank".into();
    state.snapshot.loading = false;
    state.snapshot.error = None;
    state.finished = true;
    state.pending_since = None;
    state.requested_url = None;
    state.committed_url = Some("about:blank".into());
}
fn get_tab(registry: &Registry, key: &TabKey, app: &tauri::AppHandle) -> Result<(Tab, Webview)> {
    let tab = registry
        .0
        .lock()
        .map_err(|_| "Browser state unavailable")?
        .get(key)
        .cloned()
        .ok_or("Browser tab is closed")?;
    let view = app
        .get_webview(&tab.label)
        .ok_or("Browser view is unavailable")?;
    Ok((tab, view))
}
fn remove_reservation(registry: &Registry, key: &TabKey, label: &str) -> Result<()> {
    let mut entries = registry.0.lock().map_err(|_| "Browser state unavailable")?;
    if entries.get(key).is_some_and(|tab| tab.label == label) {
        entries.remove(key);
    }
    Ok(())
}
fn drain_snapshot(state: &mut TabState) -> Snapshot {
    let result = state.snapshot.clone();
    state.snapshot.selection = None;
    state.snapshot.popup_url = None;
    state.snapshot.shortcut = None;
    state.snapshot.history_action = None;
    state.snapshot.cancelled = false;
    result
}
async fn worker<T: Send + 'static>(task: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn create(caller: Webview, session_id: String, tab_id: String, url: String) -> Result<()> {
    controller(&caller)?;
    let key = key(session_id, tab_id)?;
    let url = parse_url(&url)?;
    let registry = caller.state::<Registry>().inner().clone();
    let app = caller.app_handle().clone();
    worker(move || {
        let label = format!("browser-{}", uuid::Uuid::new_v4());
        let mut state = TabState {
            snapshot: Snapshot {
                url: url.to_string(),
                ..Default::default()
            },
            requested_url: None,
            pending_since: None,
            finished: false,
            script_revision: None,
            previous_url: String::new(),
            committed_url: None,
            inspect_epoch: 0,
            inspecting: false,
        };
        begin_navigation(&mut state, Some(url.to_string()));
        let state = Arc::new(Mutex::new(state));
        {
            let mut entries = registry.0.lock().map_err(|_| "Browser state unavailable")?;
            if entries.contains_key(&key) {
                return Ok(());
            }
            if entries.len() >= MAX_TABS {
                return Err("Close a browser tab before opening another".into());
            }
            entries.insert(
                key.clone(),
                Tab {
                    label: label.clone(),
                    state: state.clone(),
                },
            );
        }
        let loaded = state.clone();
        let titled = state.clone();
        let popup = state.clone();
        let builder = WebviewBuilder::new(&label, WebviewUrl::External(url))
            .initialization_script(PAGE_SCRIPT)
            .on_navigation(allowed_url)
            .on_document_title_changed(move |_, title| {
                if let Ok(mut state) = titled.lock() {
                    if state.snapshot.error.is_none() {
                        state.snapshot.title = title.chars().take(512).collect();
                    }
                }
            })
            .on_page_load(move |_, payload| {
                if let Ok(mut state) = loaded.lock() {
                    // A blank tab is an idle starting surface, not a network
                    // request. WebKit need not emit a Finished event for it.
                    if payload.url().as_str() == "about:blank" && blank_target(&state) {
                        finish_blank_navigation(&mut state);
                        return;
                    }
                    match payload.event() {
                        PageLoadEvent::Started => {
                            // A queued callback from the previous document must
                            // not complete a newly requested navigation.
                            if state
                                .requested_url
                                .as_ref()
                                .is_some_and(|target| target != payload.url().as_str())
                                && state.previous_url == payload.url().as_str()
                            {
                                return;
                            }
                            if state.pending_since.is_none() {
                                begin_navigation(&mut state, None);
                            }
                            state.committed_url = Some(payload.url().to_string());
                            state.snapshot.loading = true;
                        }
                        PageLoadEvent::Finished => {
                            if state
                                .requested_url
                                .as_ref()
                                .is_some_and(|target| target != payload.url().as_str())
                                && state.committed_url.as_deref() != Some(payload.url().as_str())
                            {
                                return;
                            }
                            state.finished = true;
                            state.snapshot.loading = false;
                            state.snapshot.error = None;
                            state.pending_since = None;
                            state.requested_url = None;
                        }
                    }
                    state.snapshot.url = payload.url().to_string();
                }
            })
            .on_new_window(move |url, _| {
                if parse_url(url.as_str()).is_ok() {
                    if let Ok(mut state) = popup.lock() {
                        state.snapshot.popup_url = Some(url.to_string());
                    }
                }
                NewWindowResponse::Deny
            })
            // Downloads have no destination chooser in this surface yet. Never
            // allow a page to write an unsolicited download to a default path.
            .on_download(|_, _| false);
        let result = app
            .get_window("main")
            .ok_or_else(|| "Bridge window is unavailable".to_string())
            .and_then(|window| {
                window
                    .add_child(
                        builder,
                        tauri::LogicalPosition::new(-10000.0, -10000.0),
                        tauri::LogicalSize::new(1.0, 1.0),
                    )
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(view) => {
                // A frontend reload may have removed the reservation while the
                // main thread was creating this view. Never leave an orphan.
                let retained = registry
                    .0
                    .lock()
                    .map_err(|_| "Browser state unavailable")?
                    .get(&key)
                    .is_some_and(|tab| tab.label == label);
                if !retained {
                    let _ = view.close();
                    return Err("The browser owner was reloaded".into());
                }
                if let Err(error) = view.set_auto_resize(false).and_then(|_| view.hide()) {
                    let _ = view.close();
                    remove_reservation(&registry, &key, &label)?;
                    return Err(error.to_string());
                }
                Ok(())
            }
            Err(error) => {
                remove_reservation(&registry, &key, &label)?;
                Err(error.to_string())
            }
        }
    })
    .await
}

#[tauri::command]
async fn close(caller: Webview, session_id: String, tab_id: String) -> Result<()> {
    controller(&caller)?;
    let key = key(session_id, tab_id)?;
    let registry = caller.state::<Registry>().inner().clone();
    let app = caller.app_handle().clone();
    worker(move || {
        let tab = registry
            .0
            .lock()
            .map_err(|_| "Browser state unavailable")?
            .remove(&key);
        if let Some(view) = tab.and_then(|tab| app.get_webview(&tab.label)) {
            view.close().map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
async fn navigate(caller: Webview, session_id: String, tab_id: String, url: String) -> Result<()> {
    controller(&caller)?;
    let key = key(session_id, tab_id)?;
    let url = parse_url(&url)?;
    let registry = caller.state::<Registry>().inner().clone();
    let app = caller.app_handle().clone();
    worker(move || {
        let (tab, view) = get_tab(&registry, &key, &app)?;
        {
            begin_navigation(
                &mut *tab.state.lock().map_err(|_| "Browser state unavailable")?,
                Some(url.to_string()),
            );
        }
        view.navigate(url).map_err(|error| error.to_string())
    })
    .await
}

#[tauri::command]
async fn action(caller: Webview, session_id: String, tab_id: String, action: String) -> Result<()> {
    controller(&caller)?;
    let key = key(session_id, tab_id)?;
    let registry = caller.state::<Registry>().inner().clone();
    let app = caller.app_handle().clone();
    worker(move || {
        let (tab, view) = get_tab(&registry, &key, &app)?;
        match action.as_str() {
            "inspect" | "cancel_inspect" => {
                let enabled = action == "inspect";
                {
                    let mut state = tab.state.lock().map_err(|_| "Browser state unavailable")?;
                    state.inspect_epoch = state.inspect_epoch.saturating_add(1);
                    state.inspecting = enabled;
                    state.snapshot.selection = None;
                }
                view.eval(format!("window.__bridgeBrowser?.setInspect({enabled})"))
                    .map_err(|error| error.to_string())
            }
            "reload" => {
                let retry = {
                    let mut state = tab.state.lock().map_err(|_| "Browser state unavailable")?;
                    let retry = if state.snapshot.error.is_some() {
                        state.requested_url.clone()
                    } else {
                        None
                    };
                    begin_navigation(&mut state, retry.clone());
                    retry
                };
                if let Some(url) = retry {
                    view.navigate(parse_url(&url)?)
                        .map_err(|error| error.to_string())
                } else {
                    view.reload().map_err(|error| error.to_string())
                }
            }
            "back" | "forward" => {
                begin_navigation(
                    &mut *tab.state.lock().map_err(|_| "Browser state unavailable")?,
                    None,
                );
                native_action(&view, &action)?;
                Ok(())
            }
            "stop" => {
                native_action(&view, &action)?;
                let mut state = tab.state.lock().map_err(|_| "Browser state unavailable")?;
                state.snapshot.loading = false;
                state.snapshot.selection = None;
                state.pending_since = None;
                state.requested_url = None;
                state.finished = true;
                Ok(())
            }
            _ => Err("Unknown browser action".into()),
        }
    })
    .await
}

#[tauri::command]
async fn layout(
    caller: Webview,
    session_id: String,
    tab_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    visible: bool,
) -> Result<()> {
    controller(&caller)?;
    let key = key(session_id, tab_id)?;
    if [x, y, width, height].iter().any(|value| !value.is_finite()) || width < 0.0 || height < 0.0 {
        return Err("Invalid browser viewport".into());
    }
    let registry = caller.state::<Registry>().inner().clone();
    let app = caller.app_handle().clone();
    worker(move || {
        let (_, view) = get_tab(&registry, &key, &app)?;
        if !visible || width < 1.0 || height < 1.0 {
            return view.hide().map_err(|error| error.to_string());
        }
        view.set_bounds(tauri::Rect {
            position: tauri::LogicalPosition::new(x, y).into(),
            size: tauri::LogicalSize::new(width, height).into(),
        })
        .map_err(|error| error.to_string())?;
        view.show().map_err(|error| error.to_string())
    })
    .await
}

fn valid_selection(selection: &Selection) -> bool {
    !selection.selector.is_empty()
        && selection.selector.len() <= 4096
        && selection.snippet.len() <= 6400
        && [
            &selection.bounds.x,
            &selection.bounds.y,
            &selection.bounds.width,
            &selection.bounds.height,
        ]
        .iter()
        .all(|value| value.is_finite())
        && selection.bounds.width >= 0.0
        && selection.bounds.height >= 0.0
}
fn accept_selection(state: &mut TabState, epoch: u64, selection: Option<Selection>) {
    if state.inspecting && state.inspect_epoch == epoch {
        state.snapshot.selection = selection.filter(valid_selection);
        if state.snapshot.selection.is_some() {
            state.inspecting = false;
        }
    }
}
fn page_snapshot(view: &Webview) -> Result<PageSnapshot> {
    let (tx, rx) = mpsc::channel();
    view.eval_with_callback(
        "window.__bridgeBrowser?.snapshot() ?? null",
        move |result| {
            let _ = tx.send(result);
        },
    )
    .map_err(|error| error.to_string())?;
    let data = rx
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "The page is not responding")?;
    if data.len() > 32768 {
        return Err("Page context is too large".into());
    }
    serde_json::from_str(&data).map_err(|_| "Page context is unavailable".into())
}

#[derive(Default)]
struct NativeSnapshot {
    url: String,
    can_go_back: bool,
    can_go_forward: bool,
    back_url: Option<String>,
    forward_url: Option<String>,
    loading: bool,
}

#[cfg(target_os = "macos")]
fn native_snapshot(view: &Webview) -> Result<NativeSnapshot> {
    let (tx, rx) = mpsc::channel();
    view.with_webview(move |platform| {
        // Tauri executes this closure on the main thread. The pointer is owned
        // by its live child view and never retained or sent across threads.
        let webview = unsafe { &*platform.inner().cast::<objc2_web_kit::WKWebView>() };
        let value = unsafe {
            NativeSnapshot {
                url: webview
                    .URL()
                    .and_then(|url| url.absoluteString())
                    .map(|url| url.to_string())
                    .unwrap_or_default(),
                can_go_back: webview.canGoBack(),
                can_go_forward: webview.canGoForward(),
                loading: webview.isLoading(),
                back_url: webview
                    .backForwardList()
                    .backItem()
                    .and_then(|item| item.URL().absoluteString())
                    .map(|url| url.to_string()),
                forward_url: webview
                    .backForwardList()
                    .forwardItem()
                    .and_then(|item| item.URL().absoluteString())
                    .map(|url| url.to_string()),
            }
        };
        let _ = tx.send(value);
    })
    .map_err(|error| error.to_string())?;
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "The browser view is not responding".into())
}
#[cfg(not(target_os = "macos"))]
fn native_snapshot(view: &Webview) -> Result<NativeSnapshot> {
    Ok(NativeSnapshot {
        url: view.url().map_err(|error| error.to_string())?.to_string(),
        ..Default::default()
    })
}
#[cfg(target_os = "macos")]
fn native_action(view: &Webview, action: &str) -> Result<()> {
    let action = action.to_owned();
    let (tx, rx) = mpsc::channel();
    view.with_webview(move |platform| {
        let view = unsafe { &*platform.inner().cast::<objc2_web_kit::WKWebView>() };
        let result = unsafe {
            match action.as_str() {
                "back" if view.canGoBack() => {
                    view.goBack();
                    Ok(())
                }
                "forward" if view.canGoForward() => {
                    view.goForward();
                    Ok(())
                }
                "stop" => {
                    view.stopLoading();
                    Ok(())
                }
                _ => Err("No page is available in that history direction".to_string()),
            }
        };
        let _ = tx.send(result);
    })
    .map_err(|error| error.to_string())?;
    rx.recv_timeout(Duration::from_secs(2))
        .map_err(|_| "The browser view is not responding".to_string())?
}
#[cfg(not(target_os = "macos"))]
fn native_action(view: &Webview, action: &str) -> Result<()> {
    if action == "stop" {
        return view
            .eval("window.stop()")
            .map_err(|error| error.to_string());
    }
    Err("Native browser history controls are currently available on macOS".into())
}

#[tauri::command]
async fn snapshot(caller: Webview, session_id: String, tab_id: String) -> Result<Snapshot> {
    controller(&caller)?;
    let key = key(session_id, tab_id)?;
    let registry = caller.state::<Registry>().inner().clone();
    let app = caller.app_handle().clone();
    worker(move || {
        let (tab, view) = get_tab(&registry, &key, &app)?;
        let (generation, inspect_epoch) = {
            let state = tab.state.lock().map_err(|_| "Browser state unavailable")?;
            (state.snapshot.navigation_id, state.inspect_epoch)
        };
        let page = page_snapshot(&view).ok();
        // Read native state after JS, which may wait for a document to become
        // runnable. Never use a loading/URL observation from before that wait.
        let native = native_snapshot(&view)?;
        let mut state = tab.state.lock().map_err(|_| "Browser state unavailable")?;
        // Native callbacks and commands can run while page evaluation is pending.
        // Never attach an old document result to a newer navigation.
        if generation != state.snapshot.navigation_id {
            return Ok(drain_snapshot(&mut state));
        }
        let effective_url = if state.snapshot.error.is_some()
            || (state.pending_since.is_some() && state.committed_url.is_none())
        {
            state
                .requested_url
                .clone()
                .unwrap_or_else(|| native.url.clone())
        } else {
            native.url.clone()
        };
        let route_changed = !effective_url.is_empty() && effective_url != state.snapshot.url;
        let page = page.filter(|page| page.url == native.url && page.url.len() <= MAX_URL);
        let script_changed = page.as_ref().is_some_and(|page| {
            state
                .script_revision
                .is_some_and(|revision| revision != page.revision)
        });
        if route_changed || script_changed {
            state.snapshot.navigation_id = state.snapshot.navigation_id.saturating_add(1);
            state.snapshot.selection = None;
            state.snapshot.cancelled = true;
        }
        if !effective_url.is_empty() {
            state.snapshot.url = effective_url;
        }
        state.snapshot.can_go_back = native.can_go_back;
        state.snapshot.can_go_forward = native.can_go_forward;
        state.snapshot.back_url = native.back_url;
        state.snapshot.forward_url = native.forward_url;
        #[cfg(target_os = "macos")]
        {
            if blank_target(&state) && (native.url.is_empty() || native.url == "about:blank") {
                finish_blank_navigation(&mut state);
            } else if native.loading {
                if state.pending_since.is_none() {
                    begin_navigation(&mut state, None);
                }
                state.snapshot.loading = true;
            } else if !state.finished
                && state
                    .pending_since
                    .is_some_and(|start| start.elapsed() >= Duration::from_secs(1))
            {
                // This is a native idle signal after an unfinished navigation,
                // not a timer used to diagnose framing policy. Slow pages keep
                // WKWebView.isLoading=true and are never removed.
                state.snapshot.loading = false;
                state.snapshot.error = Some(
                    "The page did not finish loading. Retry or open it in your system browser."
                        .into(),
                );
                state.pending_since = None;
                if let Some(requested) = state.requested_url.clone() {
                    state.snapshot.url = requested;
                }
            } else if state.finished {
                state.snapshot.loading = false;
            }
        }
        if let Some(page) = page.filter(|_| {
            state.snapshot.error.is_none()
                && (!state.snapshot.loading || state.committed_url.is_some())
        }) {
            state.script_revision = Some(page.revision);
            if !page.title.is_empty() {
                state.snapshot.title = page.title.chars().take(512).collect();
            }
            if let Some(url) = page.popup_url.filter(|url| parse_url(url).is_ok()) {
                state.snapshot.popup_url = Some(url);
            }
            state.snapshot.cancelled |= page.cancelled;
            if page.cancelled {
                state.inspecting = false;
            }
            state.snapshot.history_action = page
                .history_action
                .filter(|value| matches!(value.as_str(), "push" | "replace" | "traverse"));
            state.snapshot.shortcut = page.shortcut.filter(|value| {
                matches!(
                    value.as_str(),
                    "address" | "new_tab" | "close_tab" | "reopen_tab"
                )
            });
            // Navigation always wins over a stale pending element selection.
            if !route_changed && !script_changed && !native.loading {
                accept_selection(&mut state, inspect_epoch, page.selection);
            }
        }
        Ok(drain_snapshot(&mut state))
    })
    .await
}

pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("embedded-browser")
        .setup(|app, _| {
            app.manage(Registry::default());
            Ok(())
        })
        .on_page_load(|view, payload| {
            if view.label() != "main" || !matches!(payload.event(), PageLoadEvent::Started) {
                return;
            }
            let app = view.app_handle().clone();
            let tabs: Vec<_> = app
                .state::<Registry>()
                .0
                .lock()
                .map(|mut tabs| tabs.drain().map(|(_, tab)| tab.label).collect())
                .unwrap_or_default();
            // Do not block the native page-load callback on view destruction.
            tauri::async_runtime::spawn_blocking(move || {
                for label in tabs {
                    if let Some(view) = app.get_webview(&label) {
                        let _ = view.close();
                    }
                }
            });
        })
        .invoke_handler(tauri::generate_handler![
            create, close, navigate, action, layout, snapshot
        ])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_old_creation_cannot_remove_a_replacement_tab() {
        let registry = Registry::default();
        let key = ("task".into(), "tab".into());
        registry.0.lock().unwrap().insert(
            key.clone(),
            Tab {
                label: "new-view".into(),
                state: Arc::new(Mutex::new(TabState::default())),
            },
        );
        remove_reservation(&registry, &key, "old-view").unwrap();
        assert_eq!(
            registry.0.lock().unwrap().get(&key).unwrap().label,
            "new-view"
        );
        remove_reservation(&registry, &key, "new-view").unwrap();
        assert!(registry.0.lock().unwrap().is_empty());
    }
    #[test]
    fn cancelled_or_replaced_inspection_rejects_late_selection() {
        let selection = Selection {
            selector: "button".into(),
            snippet: "<button>Save</button>".into(),
            bounds: Bounds {
                x: 1.0,
                y: 1.0,
                width: 20.0,
                height: 10.0,
            },
        };
        let mut state = TabState {
            inspecting: true,
            inspect_epoch: 3,
            ..Default::default()
        };
        accept_selection(&mut state, 2, Some(selection.clone()));
        assert!(state.snapshot.selection.is_none());
        state.inspecting = false;
        accept_selection(&mut state, 3, Some(selection.clone()));
        assert!(state.snapshot.selection.is_none());
        state.inspecting = true;
        accept_selection(&mut state, 3, Some(selection));
        assert!(state.snapshot.selection.is_some());
        assert!(!state.inspecting);
    }
    #[test]
    fn transient_events_are_delivered_once_even_after_a_generation_race() {
        let mut state = TabState {
            snapshot: Snapshot {
                popup_url: Some("https://example.com/".into()),
                shortcut: Some("new_tab".into()),
                cancelled: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let first = drain_snapshot(&mut state);
        let second = drain_snapshot(&mut state);
        assert!(first.popup_url.is_some());
        assert!(first.shortcut.is_some());
        assert!(first.cancelled);
        assert!(second.popup_url.is_none());
        assert!(second.shortcut.is_none());
        assert!(!second.cancelled);
    }
    #[test]
    fn browser_children_cannot_reach_shell_commands() {
        assert!(trusted_shell("main"));
        assert!(trusted_shell(crate::meter_tray::PANEL_LABEL));
        assert!(!trusted_shell("browser-123"));
        assert!(!trusted_shell("main-child"));
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert!(capability.get("windows").is_none());
        assert_eq!(capability["webviews"], serde_json::json!(["main"]));
        assert!(capability.get("remote").is_none());
    }
    #[test]
    fn only_web_pages_and_initial_blank_are_navigable() {
        for url in [
            "https://example.com",
            "http://localhost:1420",
            "http://[::1]:8766",
            "",
            "about:blank",
        ] {
            assert!(parse_url(url).is_ok(), "{url}");
        }
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "tauri://localhost",
            "data:text/html,hi",
            "https://user:secret@example.com",
        ] {
            assert!(parse_url(url).is_err(), "{url}");
        }
    }
    #[test]
    fn malformed_page_context_cannot_escape_the_transport_budget() {
        let mut selection = Selection {
            selector: "button".into(),
            snippet: "<button>Save</button>".into(),
            bounds: Bounds {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 20.0,
            },
        };
        assert!(valid_selection(&selection));
        selection.bounds.x = f64::NAN;
        assert!(!valid_selection(&selection));
        selection.bounds.x = 0.0;
        selection.snippet = "x".repeat(6401);
        assert!(!valid_selection(&selection));
    }
    #[test]
    fn blank_tab_is_idle_until_a_real_navigation_starts() {
        let mut state = TabState::default();
        begin_navigation(&mut state, Some("about:blank".into()));
        assert!(blank_target(&state));
        assert!(state.finished);
        assert!(!state.snapshot.loading);
        assert!(state.pending_since.is_none());
        assert!(state.snapshot.error.is_none());
        let blank_generation = state.snapshot.navigation_id;
        begin_navigation(&mut state, Some("http://localhost:8767/".into()));
        assert!(!blank_target(&state));
        assert!(!state.finished);
        assert!(state.snapshot.loading);
        assert!(state.pending_since.is_some());
        assert!(state.snapshot.error.is_none());
        assert!(state.snapshot.navigation_id > blank_generation);
    }
    #[test]
    fn beginning_navigation_invalidates_context_and_previous_failure() {
        let mut state = TabState {
            snapshot: Snapshot {
                error: Some("failed".into()),
                navigation_id: 7,
                ..Default::default()
            },
            requested_url: None,
            pending_since: None,
            finished: true,
            script_revision: Some(4),
            previous_url: String::new(),
            committed_url: None,
            inspect_epoch: 0,
            inspecting: false,
        };
        begin_navigation(&mut state, Some("https://example.com".into()));
        assert_eq!(state.snapshot.navigation_id, 8);
        assert!(state.snapshot.loading);
        assert!(state.snapshot.cancelled);
        assert!(state.snapshot.error.is_none());
        assert!(state.script_revision.is_none());
    }
}
