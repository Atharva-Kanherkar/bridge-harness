//! The piece that makes the browser-clone scenario actually run: it ties the
//! clone process (part 1), the leak guard (part 2), the cookie import and the
//! agent tool (part 3) into one flow behind a small API.
//!
//! When an agent asks for a site and the person approves, [`CloneOrchestrator`]
//! spawns a *guarded* clone, signs it in (import from the user's browser, or a
//! login inside the clone), arms the guard's allow list and secrets, mints the
//! agent's tool capability, and starts the lease clock. It also loads the code
//! under test into the clone, hands back live frames, and destroys everything
//! on takeover-less idle, lease expiry, task end, or quit.
//!
//! This is deliberately thin: every hard part already lives in the modules it
//! composes. What was missing — and what issue #758 is — is the composition.

use crate::browser_clone::{CloneError, CloneSupervisor, CookieSpec};
use crate::browser_clone_signin::{import_cookies, Browser, ImportError};
use crate::clone_browser_tool::CloneBrowserTool;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Which browser a clone runs, and whose cookie store an import reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloneBrowser {
    Chrome,
    Brave,
}

impl CloneBrowser {
    fn signin(self) -> Browser {
        match self {
            Self::Chrome => Browser::Chrome,
            Self::Brave => Browser::Brave,
        }
    }
}

/// How the clone gets signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInPath {
    /// Copy the approved site's cookies from the user's browser.
    Import,
    /// Read nothing; the person signs in inside the clone.
    SignInInside,
}

/// What the surface shows and the agent turn checks: no cookie values, ever.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CloneView {
    pub session_id: String,
    pub clone_id: String,
    pub domain: String,
    pub status: CloneStatus,
    pub sign_in_path: SignInPath,
    pub minutes_left: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CloneStatus {
    Acting,
    WaitingForYou,
    TakenOver,
}

/// Input the person sends to a clone they have taken over.
#[derive(Debug, Clone, PartialEq)]
pub enum CloneInput {
    /// A click at a point given as a fraction of the viewport (0..1), so the
    /// dock's scaled frame maps straight onto the page.
    Click { x: f64, y: f64 },
    Scroll { x: f64, y: f64, delta_y: f64 },
    Type { text: String },
    /// One of the keys a login form needs.
    Key { key: String },
}

/// CDP codes for the keys a sign-in needs. Anything else is refused.
fn key_codes(key: &str) -> Option<(&'static str, i64, Option<&'static str>)> {
    Some(match key {
        "Enter" => ("Enter", 13, Some("\r")),
        "Tab" => ("Tab", 9, None),
        "Backspace" => ("Backspace", 8, None),
        "Escape" => ("Escape", 27, None),
        "ArrowUp" => ("ArrowUp", 38, None),
        "ArrowDown" => ("ArrowDown", 40, None),
        "ArrowLeft" => ("ArrowLeft", 37, None),
        "ArrowRight" => ("ArrowRight", 39, None),
        "Delete" => ("Delete", 46, None),
        "Home" => ("Home", 36, None),
        "End" => ("End", 35, None),
        "PageUp" => ("PageUp", 33, None),
        "PageDown" => ("PageDown", 34, None),
        _ => return None,
    })
}

struct Active {
    clone_id: String,
    domain: String,
    status: CloneStatus,
    sign_in_path: SignInPath,
    expires_at: Instant,
    /// The person approved the agent acting on this clone. Takeover pauses the
    /// agent; handing back restores its actions only if this is set.
    actions_approved: bool,
    /// The agent process the tool capability is bound to; read back only in the
    /// end-to-end test's assertion today.
    #[cfg_attr(not(test), allow(dead_code))]
    runtime_pid: u32,
}

/// One clone per Bridge session. Composition only: the supervisor is guarded,
/// so every clone it starts is launched behind the egress proxy with the
/// request checker armed.
pub struct CloneOrchestrator {
    supervisor: Arc<CloneSupervisor>,
    tool: Arc<CloneBrowserTool>,
    active: Mutex<HashMap<String, Active>>,
    default_ttl: Duration,
    cookie_importer: fn(Browser, &str, &[String]) -> Result<Vec<CookieSpec>, ImportError>,
}

// A startup error must clean up even before the clone reaches the active map.
struct PendingClone<'a> {
    supervisor: &'a CloneSupervisor,
    id: Option<String>,
}

impl Drop for PendingClone<'_> {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = self.supervisor.destroy(&id);
        }
    }
}

impl CloneOrchestrator {
    pub fn new(supervisor: Arc<CloneSupervisor>, tool: Arc<CloneBrowserTool>) -> Arc<Self> {
        Arc::new(Self {
            supervisor,
            tool,
            active: Mutex::new(HashMap::new()),
            default_ttl: Duration::from_secs(30 * 60),
            cookie_importer: import_cookies,
        })
    }

    /// Approve-and-go: spawn a guarded clone for `session_id`, sign it in, arm
    /// the guard, mint the agent tool, and start the lease. `runtime_pid` is the
    /// agent process the tool capability is bound to. Replaces any existing
    /// clone for the session.
    pub fn request_clone(
        &self,
        session_id: &str,
        domain: &str,
        browser: CloneBrowser,
        path: SignInPath,
        ttl: Option<Duration>,
        runtime_pid: u32,
    ) -> Result<CloneView, CloneError> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        self.request_clone_locked(session_id, domain, browser, path, ttl, runtime_pid, &[])
    }

    /// A person explicitly starts a browser through the native UI.
    pub fn start_approved_clone(&self, session_id: &str, domain: &str, browser: CloneBrowser, path: SignInPath, ttl: Option<Duration>, runtime_pid: u32, vision: bool) -> Result<CloneView, CloneError> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        let view = self.request_clone_locked(session_id, domain, browser, path, ttl, runtime_pid, &[])?;
        self.tool.set_vision(session_id, vision);
        if let Some(entry) = self.active.lock().unwrap_or_else(|p| p.into_inner()).get_mut(session_id) { entry.actions_approved = true; }
        if path == SignInPath::Import { self.tool.allow_mutations(session_id); }
        Ok(view)
    }

    fn request_clone_locked(
        &self, session_id: &str, domain: &str, browser: CloneBrowser,
        path: SignInPath, ttl: Option<Duration>, runtime_pid: u32, additional_domains: &[String],
    ) -> Result<CloneView, CloneError> {
        let domain = normalize_domain(domain)
            .ok_or_else(|| CloneError::Launch("invalid approved domain".into()))?;
        let mut domains = vec![domain.clone()];
        for host in additional_domains {
            domains.push(normalize_domain(host)
                .ok_or_else(|| CloneError::Launch("invalid approved domain".into()))?);
        }
        domains.sort();
        domains.dedup();
        self.destroy_browser_locked(session_id);

        let info = self.supervisor.spawn_clone()?;
        let mut pending_clone = PendingClone { supervisor: &self.supervisor, id: Some(info.id.clone()) };
        let guard = self
            .supervisor
            .clone_guard(&info.id)
            .ok_or_else(|| CloneError::Launch("clone is not guarded".into()))?;

        // The clone may reach the approved site; nothing else.
        {
            let mut guard = guard.lock().unwrap_or_else(|p| p.into_inner());
            for host in &domains { guard.allow_host(host); }
        }

        // Sign in. Import copies all explicitly approved domains' cookies from the user's
        // browser; sign-in-inside reads nothing and the person logs in later.
        if path == SignInPath::Import {
            let cookies = (self.cookie_importer)(browser.signin(), "Default", &domains).map_err(map_import)?;
            {
                let mut guard = guard.lock().unwrap_or_else(|p| p.into_inner());
                for cookie in &cookies {
                    guard.add_secret(&cookie.domain, &cookie.value);
                }
            }
            if let Err(error) = self.load_session(&info.id, cookies) {
                return Err(error);
            }
        }

        if path == SignInPath::SignInInside {
            self.supervisor.page_call(&info.id, "Page.navigate", json!({"url":format!("https://{domain}")}))?;
            self.tool.pause(session_id);
        }
        // Bind the agent tool to the agent's runtime process, not the clone.
        if self.tool.capability_context(session_id, &info.id, runtime_pid, &domain).is_none() {
            self.tool.revoke_browser(session_id);
            return Err(CloneError::Launch("browser tool unavailable".into()));
        }

        let status = match path {
            SignInPath::Import => CloneStatus::Acting,
            SignInPath::SignInInside => CloneStatus::WaitingForYou,
        };
        let ttl = ttl.unwrap_or(self.default_ttl);
        let active = Active {
            clone_id: info.id.clone(),
            domain: domain.clone(),
            status,
            sign_in_path: path,
            expires_at: Instant::now() + ttl,
            actions_approved: false,
            runtime_pid,
        };
        let view = view_of(session_id, &active, ttl);
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(session_id.to_owned(), active);
        pending_clone.id = None;
        Ok(view)
    }

    fn load_session(&self, clone_id: &str, cookies: Vec<CookieSpec>) -> Result<(), CloneError> {
        self.supervisor.load_session(clone_id, cookies)
    }

    /// Load an unpacked extension — the code under test — into the clone. Uses
    /// the Extensions.loadUnpacked CDP command the launch flag enables. Returns
    /// the loaded extension id.
    pub fn load_extension(&self, session_id: &str, path: &str) -> Result<String, CloneError> {
        let clone_id = self.clone_id(session_id)?;
        let reply =
            self.supervisor
                .tool_call(&clone_id, "Extensions.loadUnpacked", json!({ "path": path }))?;
        Ok(reply
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// A fresh local frame of the clone, base64 PNG, for the dock's live
    /// view. (Streaming screencast is a later optimization; a captured frame is
    /// the same picture.)
    pub fn frame(&self, session_id: &str) -> Result<String, CloneError> {
        let clone_id = self.clone_id(session_id)?;
        let reply = self.supervisor.page_call(
            &clone_id,
            "Page.captureScreenshot",
            json!({ "format": "png" }),
        )?;
        reply
            .get("data")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| CloneError::Cdp {
                method: "Page.captureScreenshot".into(),
                message: "no frame".into(),
            })
    }

    /// The agent tool wrapper for this session, re-minted so a resumed turn gets
    /// a live capability. `None` when the session has no clone. This is what
    /// `live_turn` injects into the agent's application context.
    pub fn capability_context(&self, session_id: &str, runtime_pid: u32) -> Option<String> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        let (clone_id, domain) = {
            let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
            let entry = active.get(session_id)?;
            (entry.clone_id.clone(), entry.domain.clone())
        };
        self.tool
            .capability_context(session_id, &clone_id, runtime_pid, &domain)
    }

    /// The agent's "ask for a clone" capability, minted every turn (see
    /// `live_turn`) so the agent can request one before any exists.
    pub fn request_capability_context(&self, session_id: &str, runtime_pid: u32) -> Option<String> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        self.tool.request_capability_context(session_id, runtime_pid)
    }

    /// The domain a session's agent has asked for, waiting on the person. The
    /// dock turns this into the Allow/Deny card.
    pub fn pending_request(&self, session_id: &str) -> Option<String> {
        self.tool.pending_request(session_id)
    }

    /// The person allowed the agent's request: spawn the clone for the asked
    /// domain (import the sign-in), and approve page actions on it.
    pub fn approve_request(&self, session_id: &str, request_id: &str, runtime_pid: u32, path: SignInPath, ttl: Option<Duration>, vision: bool) -> Result<CloneView, CloneError> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        let request = self.tool.take_pending_request(session_id, request_id, runtime_pid).map_err(CloneError::Launch)?;
        let replaces_browser = self.view(session_id).is_some();
        let started = self.request_clone_locked(session_id, &request.domain, CloneBrowser::Chrome, path, ttl, runtime_pid, &request.additional_domains);
        let view = match started {
            Ok(view) => view,
            Err(error) => {
                self.tool.answer_request(session_id, request_id, json!({"ok":false,"awaiting":false,"error":startup_error_message(&error),"previousBrowserDestroyed":replaces_browser}));
                return Err(error);
            }
        };
        if let Some(extension) = request.extension_path {
            if let Err(error) = self.load_extension(session_id, &extension) {
                self.destroy_browser_locked(session_id);
                self.tool.answer_request(session_id, request_id, json!({"ok":false,"awaiting":false,"error":"extension loading failed","previousBrowserDestroyed":replaces_browser}));
                return Err(error);
            }
        }
        self.tool.set_vision(session_id, vision);
        // The person approved the agent acting, so page actions are allowed.
        if let Some(entry) = self.active.lock().unwrap_or_else(|p| p.into_inner()).get_mut(session_id) {
            entry.actions_approved = true;
        }
        if path == SignInPath::SignInInside { self.tool.pause(session_id); } else { self.tool.allow_mutations(session_id); }
        let context = match self.tool.capability_context(session_id, &view.clone_id, runtime_pid, &view.domain) {
            Some(context) => context,
            None => {
                self.destroy_browser_locked(session_id);
                self.tool.answer_request(session_id, request_id, json!({"ok":false,"awaiting":false,"error":"browser tool unavailable","previousBrowserDestroyed":replaces_browser}));
                return Err(CloneError::Launch("browser tool unavailable".into()));
            }
        };
        self.tool.answer_request(session_id, request_id, json!({"ok":true,"awaiting":false,"tool":context,"waitingForUser":path == SignInPath::SignInInside,"previousBrowserDestroyed":replaces_browser,"message":if replaces_browser { "The previous browser was destroyed. Use these fresh browser tool instructions." } else { "Browser ready. Use these browser tool instructions." }}));
        Ok(view)
    }

    /// The person denied the request; drop it.
    pub fn deny_request(&self, session_id: &str, request_id: &str) -> Result<(), CloneError> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        let request = self.tool.pending_details(session_id).ok_or_else(|| CloneError::Launch("no pending clone request".into()))?;
        self.tool.take_pending_request(session_id, request_id, request.runtime_pid).map_err(CloneError::Launch)?;
        self.tool.answer_request(session_id, request_id, json!({"ok":false,"awaiting":false,"error":"request denied"}));
        Ok(())
    }

    pub fn pending_requests(&self) -> Vec<bridge_protocol::messages::CloneRequest> { self.tool.pending_requests() }

    /// Whether the session's agent can see screenshots of its clone.
    pub fn agent_vision(&self, session_id: &str) -> bool { self.tool.vision(session_id) }

    /// Where the agent's pointer last landed, as viewport fractions.
    pub fn agent_pointer(&self, session_id: &str) -> Option<(f64, f64, &'static str, u64)> { self.tool.pointer(session_id) }

    pub fn pending_details(&self, session_id: &str) -> Option<crate::clone_browser_tool::PendingRequest> {
        self.tool.pending_details(session_id)
    }

    pub fn view(&self, session_id: &str) -> Option<CloneView> {
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        let entry = active.get(session_id)?;
        Some(view_of(session_id, entry, entry.expires_at.saturating_duration_since(Instant::now())))
    }

    /// The person takes control (to sign in, finish 2FA). The agent is paused:
    /// its page actions are refused until the clone is handed back.
    pub fn take_over(&self, session_id: &str) {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        self.set_status(session_id, CloneStatus::TakenOver);
        self.tool.pause(session_id);
    }

    /// The person hands control back. The agent's page actions come back only
    /// if the person had approved them.
    pub fn hand_back(&self, session_id: &str) {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        self.set_status(session_id, CloneStatus::Acting);
        self.tool.resume(session_id);
        let approved = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .is_some_and(|entry| entry.actions_approved);
        if approved {
            self.tool.allow_mutations(session_id);
        }
    }

    /// Input from the person while they hold the clone (takeover): a click or
    /// scroll at a point given as a fraction of the viewport, typed text, or one
    /// of a small set of keys. Refused unless the clone is taken over, so this
    /// can never become a side door for the agent.
    pub fn forward_input(&self, session_id: &str, input: CloneInput) -> Result<(), CloneError> {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        let (clone_id, status) = {
            let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
            let entry = active
                .get(session_id)
                .ok_or_else(|| CloneError::UnknownClone(session_id.to_owned()))?;
            (entry.clone_id.clone(), entry.status)
        };
        if status != CloneStatus::TakenOver {
            return Err(CloneError::Launch("take over the clone before typing into it".into()));
        }
        match input {
            CloneInput::Click { x, y } => {
                let (px, py) = self.viewport_point(&clone_id, x, y)?;
                for kind in ["mousePressed", "mouseReleased"] {
                    self.supervisor.page_call(
                        &clone_id,
                        "Input.dispatchMouseEvent",
                        json!({"type": kind, "x": px, "y": py, "button": "left", "clickCount": 1}),
                    )?;
                }
            }
            CloneInput::Scroll { x, y, delta_y } => {
                let (px, py) = self.viewport_point(&clone_id, x, y)?;
                self.supervisor.page_call(
                    &clone_id,
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseWheel", "x": px, "y": py, "deltaX": 0, "deltaY": delta_y}),
                )?;
            }
            CloneInput::Type { text } => {
                // A burst long enough to be a credential is kept from the
                // agent; single keystrokes would scrub ordinary letters.
                if let Some(guard) = self.supervisor.clone_guard(&clone_id).filter(|_| text.chars().count() >= 4) {
                    let history = self.supervisor.page_call(&clone_id, "Page.getNavigationHistory", json!({}))?;
                    let current = history.get("currentIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
                    let domain = history.get("entries").and_then(Value::as_array).and_then(|entries| entries.get(current))
                        .and_then(|entry| entry.get("url")).and_then(Value::as_str)
                        .and_then(|url| reqwest::Url::parse(url).ok()).and_then(|url| url.host_str().map(str::to_owned))
                        .unwrap_or_else(|| self.active.lock().unwrap_or_else(|p| p.into_inner()).get(session_id).map(|entry| entry.domain.clone()).unwrap_or_default());
                    guard.lock().unwrap_or_else(|p| p.into_inner()).add_secret(&domain, &text);
                }
                self.supervisor
                    .page_call(&clone_id, "Input.insertText", json!({ "text": text }))?;
            }
            CloneInput::Key { key } => {
                let (code, vk, text) = key_codes(&key)
                    .ok_or_else(|| CloneError::Launch(format!("unsupported key {key}")))?;
                let mut down = json!({"type": "keyDown", "key": key, "code": code, "windowsVirtualKeyCode": vk});
                if let Some(text) = text {
                    down["text"] = json!(text);
                }
                self.supervisor.page_call(&clone_id, "Input.dispatchKeyEvent", down)?;
                self.supervisor.page_call(
                    &clone_id,
                    "Input.dispatchKeyEvent",
                    json!({"type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk}),
                )?;
            }
        }
        Ok(())
    }

    /// Map a viewport fraction (0..1) onto CSS pixels of the page.
    fn viewport_point(&self, clone_id: &str, x: f64, y: f64) -> Result<(f64, f64), CloneError> {
        let metrics = self
            .supervisor
            .page_call(clone_id, "Page.getLayoutMetrics", json!({}))?;
        let viewport = metrics
            .get("cssVisualViewport")
            .or_else(|| metrics.get("visualViewport"))
            .cloned()
            .unwrap_or(Value::Null);
        let width = viewport.get("clientWidth").and_then(Value::as_f64).unwrap_or(800.0);
        let height = viewport.get("clientHeight").and_then(Value::as_f64).unwrap_or(600.0);
        Ok((x.clamp(0.0, 1.0) * width, y.clamp(0.0, 1.0) * height))
    }

    fn set_status(&self, session_id: &str, status: CloneStatus) {
        if let Some(entry) = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(session_id)
        {
            entry.status = status;
        }
    }

    /// Tear down the session's clone: kill the process, eject the RAM disk,
    /// revoke the agent tool. Idempotent.
    pub fn destroy(&self, session_id: &str) {
        let operation = self.tool.session_operation(session_id);
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        self.destroy_locked(session_id);
    }

    fn destroy_locked(&self, session_id: &str) {
        self.destroy_browser_locked(session_id);
        self.tool.revoke_session(session_id);
    }

    fn destroy_browser_locked(&self, session_id: &str) {
        let clone_id = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id)
            .map(|entry| entry.clone_id);
        if let Some(clone_id) = clone_id {
            let _ = self.supervisor.destroy(&clone_id);
        }
        self.tool.revoke_browser(session_id);
    }

    /// Destroy every clone whose lease has run out. The host calls this on a
    /// timer; it is also what a 30-minute default comes to.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let expired: Vec<(String, String)> = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(session, entry)| (session.clone(), entry.clone_id.clone()))
            .collect();
        for (session, clone_id) in expired {
            let operation = self.tool.session_operation(&session);
            let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
            let still_expired = self.active.lock().unwrap_or_else(|p| p.into_inner())
                .get(&session).is_some_and(|entry| entry.clone_id == clone_id && entry.expires_at <= now);
            if still_expired { self.destroy_locked(&session); }
        }
    }

    fn clone_id(&self, session_id: &str) -> Result<String, CloneError> {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(session_id)
            .map(|entry| entry.clone_id.clone())
            .ok_or_else(|| CloneError::UnknownClone(session_id.to_owned()))
    }

    #[cfg(test)]
    fn runtime_pid(&self, session_id: &str) -> Option<u32> {
        self.active
            .lock()
            .unwrap()
            .get(session_id)
            .map(|entry| entry.runtime_pid)
    }
}

impl Drop for CloneOrchestrator {
    fn drop(&mut self) {
        let sessions: Vec<String> = self.active.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect();
        for session in sessions {
            self.destroy(&session);
        }
    }
}

fn view_of(session_id: &str, active: &Active, remaining: Duration) -> CloneView {
    CloneView {
        session_id: session_id.to_owned(),
        clone_id: active.clone_id.clone(),
        domain: active.domain.clone(),
        status: active.status,
        sign_in_path: active.sign_in_path,
        minutes_left: remaining.as_secs().div_ceil(60),
    }
}

fn normalize_domain(domain: &str) -> Option<String> {
    let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    (domain.len() <= 253 && domain.contains('.') && domain.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })).then_some(domain)
}

// Startup can fail after CDP has seen cookie values. Only forward known,
// value-free import causes; raw browser error messages must stay off the tool.
fn startup_error_message(error: &CloneError) -> String {
    if let CloneError::Launch(message) = error {
        for cause in [ImportError::Domain, ImportError::Store, ImportError::Keychain, ImportError::Decrypt] {
            let safe = format!("sign-in import: {cause}");
            if message == &safe { return safe; }
        }
    }
    "browser startup failed".into()
}

fn map_import(error: ImportError) -> CloneError {
    CloneError::Launch(format!("sign-in import: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_errors_never_forward_raw_browser_or_cookie_details() {
        let error = CloneError::Cdp { method: "Storage.setCookies".into(), message: "synthetic-cookie-value".into() };
        assert_eq!(startup_error_message(&error), "browser startup failed");
        let error = CloneError::Launch("sign-in import: untrusted-value".into());
        assert_eq!(startup_error_message(&error), "browser startup failed");
    }

    fn request_via_helper(tool: &CloneBrowserTool, session: &str, request: Value) -> Value {
        let output = std::process::Command::new(tool.socket_path().parent().unwrap().join(format!("clone-request-{session}")))
            .arg(request.to_string()).output().unwrap();
        serde_json::from_slice(&output.stdout).unwrap()
    }

    #[test]
    fn approved_replacement_reports_teardown_and_fresh_instructions() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), true);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let orchestrator = CloneOrchestrator::new(Arc::clone(&supervisor), Arc::clone(&tool));
        let pid = std::process::id();
        orchestrator.request_capability_context("s1", pid).unwrap();
        let request = request_via_helper(&tool, "s1", json!({"kind":"request","domain":"first.test"}));
        let id = request["requestId"].as_str().unwrap();
        let first = orchestrator.approve_request("s1", id, pid, SignInPath::SignInInside, None, true).unwrap();
        let answer = request_via_helper(&tool, "s1", json!({"kind":"request_status","requestId":id}));
        assert_eq!(answer["previousBrowserDestroyed"], false);
        let request = request_via_helper(&tool, "s1", json!({"kind":"request","domain":"second.test"}));
        let next_id = request["requestId"].as_str().unwrap();
        // Asking alone does not kill the existing browser.
        assert_eq!(orchestrator.view("s1").unwrap().clone_id, first.clone_id);
        let second = orchestrator.approve_request("s1", next_id, pid, SignInPath::SignInInside, None, true).unwrap();
        assert_ne!(first.clone_id, second.clone_id);
        assert!(supervisor.clone_guard(&first.clone_id).is_none());
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 1);
        let answer = request_via_helper(&tool, "s1", json!({"kind":"request_status","requestId":next_id}));
        assert_eq!(answer["previousBrowserDestroyed"], true);
        assert!(answer["message"].as_str().unwrap().contains("previous browser was destroyed"));
        assert!(answer["tool"].as_str().unwrap().contains("second.test"));
        let stale = request_via_helper(&tool, "s1", json!({"kind":"request_status","requestId":id}));
        assert!(stale["error"].as_str().unwrap().contains("superseded"));
    }

    #[test]
    fn import_failure_reaches_the_requesting_agent_with_a_value_free_cause() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), true);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let mut orchestrator = CloneOrchestrator::new(supervisor, Arc::clone(&tool));
        Arc::get_mut(&mut orchestrator).unwrap().cookie_importer = |_, _, _| Err(ImportError::Keychain);
        let pid = std::process::id();
        orchestrator.request_capability_context("s1", pid).unwrap();
        let request = request_via_helper(&tool, "s1", json!({"kind":"request","domain":"docs.google.com"}));
        let id = request["requestId"].as_str().unwrap();
        assert!(orchestrator.approve_request("s1", id, pid, SignInPath::Import, None, true).is_err());
        let answer = request_via_helper(&tool, "s1", json!({"kind":"request_status","requestId":id}));
        assert_eq!(answer["ok"], false);
        assert_eq!(answer["awaiting"], false);
        assert!(answer["error"].as_str().unwrap().contains("Safe Storage permission was denied or unavailable"));
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 0);
    }

    fn fixture_importer(_: Browser, profile: &str, domains: &[String]) -> Result<Vec<CookieSpec>, ImportError> {
        assert_eq!(profile, "Default");
        assert_eq!(domains, ["accounts.google.com", "docs.google.com"]);
        Ok(domains.iter().map(|domain| CookieSpec {
            name: "sid".into(), value: format!("synthetic-session-{domain}"),
            domain: format!(".login.{domain}"), path: "/".into(),
            secure: true, http_only: true, same_site: None,
        }).collect())
    }

    #[test]
    fn approved_dependencies_reach_the_cookie_jar_and_keep_their_actual_owner() {
        use crate::browser_clone_guard::{request_verdict, Verdict};
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), true);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let mut orchestrator = CloneOrchestrator::new(Arc::clone(&supervisor), tool);
        Arc::get_mut(&mut orchestrator).unwrap().cookie_importer = fixture_importer;
        let view = orchestrator.request_clone_locked("s1", "docs.google.com", CloneBrowser::Chrome, SignInPath::Import, None, std::process::id(), &["accounts.google.com".into()]).unwrap();
        let jar = supervisor.tool_call(&view.clone_id, "Storage.getCookies", json!({})).unwrap();
        let cookies = jar["cookies"].as_array().unwrap();
        assert_eq!(cookies.len(), 2);
        let guard = supervisor.clone_guard(&view.clone_id).unwrap();
        let guard = guard.lock().unwrap();
        for cookie in cookies {
            let owner = cookie["domain"].as_str().unwrap().trim_start_matches('.');
            let secret = cookie["value"].as_str().unwrap();
            assert_eq!(request_verdict(&guard, &format!("https://{owner}/"), secret), Verdict::Allow);
            assert!(matches!(request_verdict(&guard, "https://accounts.google.com/", secret), Verdict::Block(_)));
            assert!(matches!(request_verdict(&guard, "https://docs.google.com/", secret), Verdict::Block(_)));
            let mut reply = json!({"text": secret});
            guard.scrub_response(&mut reply);
            assert_eq!(reply["text"], "[redacted]");
        }
        assert!(!guard.allows_host("google.com"));
        assert!(!guard.allows_host("drive.google.com"));
    }

    #[test]
    fn failed_dependency_import_destroys_the_child_and_profile() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), true);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let mut orchestrator = CloneOrchestrator::new(supervisor, tool);
        Arc::get_mut(&mut orchestrator).unwrap().cookie_importer = |_, _, _| Err(ImportError::Decrypt);
        let error = orchestrator.request_clone_locked("s1", "docs.google.com", CloneBrowser::Chrome, SignInPath::Import, None, std::process::id(), &["accounts.google.com".into()]).unwrap_err();
        assert!(error.to_string().contains("browser cookie could not be decrypted"));
        assert!(orchestrator.view("s1").is_none());
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 0);
    }

    #[test]
    fn overlapping_starts_in_one_chat_leave_only_one_managed_profile() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), true);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let orchestrator = CloneOrchestrator::new(supervisor, tool);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = ["first.test", "second.test"].into_iter().map(|domain| {
            let orchestrator = Arc::clone(&orchestrator);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                orchestrator.request_clone("s1", domain, CloneBrowser::Chrome, SignInPath::SignInInside, None, std::process::id()).unwrap();
            })
        }).collect();
        barrier.wait();
        for thread in threads { thread.join().unwrap(); }
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 1);
        assert!(orchestrator.view("s1").is_some());
        orchestrator.destroy("s1");
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 0);
    }

    #[test]
    fn failure_after_spawn_destroys_the_untracked_child_and_profile() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), false);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let orchestrator = CloneOrchestrator::new(supervisor, tool);
        let result = orchestrator.request_clone("s1", "example.test", CloneBrowser::Chrome, SignInPath::SignInInside, None, std::process::id());
        assert!(result.is_err(), "an unguarded clone must be refused");
        assert!(orchestrator.view("s1").is_none());
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 0);
        let pid: u32 = std::fs::read_to_string(dir.path().join("browser.pid")).unwrap().parse().unwrap();
        assert_ne!(unsafe { libc::kill(pid as libc::pid_t, 0) }, 0, "failed startup left a child alive");
    }

    #[test]
    fn dropping_the_orchestrator_destroys_its_active_profiles() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = crate::browser_clone::tests::synthetic_supervisor(dir.path(), true);
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().join("tool")).unwrap();
        let orchestrator = CloneOrchestrator::new(Arc::clone(&supervisor), tool);
        let view = orchestrator.request_clone("s1", "example.test", CloneBrowser::Chrome, SignInPath::SignInInside, None, std::process::id()).unwrap();
        drop(orchestrator);
        assert!(supervisor.clone_guard(&view.clone_id).is_none());
        assert_eq!(std::fs::read_dir(dir.path().join("mounts")).unwrap().count(), 0);
    }

    #[test]
    fn a_domain_is_normalized_and_validated() {
        assert_eq!(normalize_domain(" .YouTube.com "), Some("youtube.com".into()));
        assert!(normalize_domain("bad domain").is_none());
        assert!(normalize_domain("").is_none());
    }

    #[test]
    fn a_view_never_carries_a_value() {
        let active = Active {
            clone_id: "c1".into(),
            domain: "example.com".into(),
            status: CloneStatus::WaitingForYou,
            sign_in_path: SignInPath::SignInInside,
            expires_at: Instant::now() + Duration::from_secs(600),
            actions_approved: false,
            runtime_pid: 1,
        };
        let json = serde_json::to_string(&view_of("s1", &active, Duration::from_secs(600))).unwrap();
        assert!(json.contains("waiting_for_you"));
        assert!(json.contains("\"minutes_left\":10"));
    }

    // The whole path against a real browser: request → guarded clone → agent
    // tool over its socket → extension under test loads → live frame → lease
    // expiry destroys everything. Env-gated like the other live clone tests.
    #[cfg(target_os = "macos")]
    mod live {
        use super::*;
        use crate::browser_clone::{discover_browser, CloneConfig};
        use std::io::{Read, Write};
        use std::os::unix::net::UnixStream;

        fn browser() -> Option<std::path::PathBuf> {
            if std::env::var("BRIDGE_CLONE_LIVE").as_deref() != Ok("1") {
                eprintln!("skipping: BRIDGE_CLONE_LIVE is not 1");
                return None;
            }
            CloneConfig::from_env().browser.or_else(discover_browser)
        }

        /// A minimal unpacked extension whose service worker just has to load.
        fn write_extension(dir: &std::path::Path) -> String {
            let ext = dir.join("ext");
            std::fs::create_dir_all(&ext).unwrap();
            std::fs::write(
                ext.join("manifest.json"),
                r#"{"manifest_version":3,"name":"overlay-under-test","version":"1","background":{"service_worker":"sw.js"}}"#,
            )
            .unwrap();
            std::fs::write(ext.join("sw.js"), "self.__loaded = true;").unwrap();
            ext.to_string_lossy().into_owned()
        }

        /// Call the agent tool the way the agent's command runner does: an HTTP
        /// POST over the unix socket with the capability headers.
        fn tool_call(tool: &CloneBrowserTool, session: &str, body: &str) -> (String, String) {
            call_script(tool, session, &format!("clone-browser-{session}"), body)
        }

        /// Drive the tool the way the agent's command runner does, reading the
        /// token from the named wrapper script (the drive tool or the request
        /// tool) the tool wrote for this session.
        fn call_script(
            tool: &CloneBrowserTool,
            session: &str,
            script_name: &str,
            body: &str,
        ) -> (String, String) {
            let mut stream = UnixStream::connect(tool.socket_path()).unwrap();
            let script =
                std::fs::read_to_string(tool.socket_path().parent().unwrap().join(script_name))
                    .unwrap();
            let token = script
                .split("Authorization: Bearer ")
                .nth(1)
                .and_then(|rest| rest.split('\'').next())
                .unwrap()
                .to_owned();
            let request = format!(
                "POST /v1/clone-browser HTTP/1.1\r\nHost: localhost\r\nX-Bridge-Session: {session}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            let status = response.lines().next().unwrap_or("").to_owned();
            let body = response.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
            (status, body)
        }

        #[test]
        fn the_whole_scenario_runs_against_a_real_browser() {
            let Some(browser_bin) = browser() else { return };
            let dir = tempfile::tempdir().unwrap();
            let supervisor = CloneSupervisor::with_ram_disk(
                dir.path().join("clones.json"),
                dir.path().join("mounts"),
                CloneConfig {
                    browser: Some(browser_bin),
                    headless: true,
                    guarded: true,
                    ..CloneConfig::default()
                },
            );
            // A short base dir: the tool's unix socket path must clear SUN_LEN
            // (~104 bytes on macOS), which a deep tempdir path would blow.
            let tool_dir = std::path::PathBuf::from("/tmp").join(format!("bct-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&tool_dir);
            let tool = CloneBrowserTool::new(Arc::clone(&supervisor), tool_dir.clone()).unwrap();
            let orchestrator = CloneOrchestrator::new(Arc::clone(&supervisor), Arc::clone(&tool));

            // The agent tool binds to this process; the test is the "runtime".
            let session = "session-live";
            // Sign-in-inside so the test needs no real Keychain or cookies.
            let view = orchestrator
                .request_clone(session, "127.0.0.1", CloneBrowser::Chrome, SignInPath::SignInInside, Some(Duration::from_millis(400)), std::process::id())
                .expect("a guarded clone comes up and is signed in");
            assert_eq!(view.domain, "127.0.0.1");
            assert!(orchestrator.runtime_pid(session).is_some());

            // The code under test loads into the clone.
            let ext_path = write_extension(dir.path());
            let ext_id = orchestrator.load_extension(session, &ext_path).expect("the extension loads");
            assert!(!ext_id.is_empty(), "loadUnpacked returned no id");

            orchestrator.hand_back(session);
            // The agent drives it through its real tool socket: a read command
            // returns, an out-of-contract one is refused.
            let (ok_status, ok_body) = tool_call(&tool, session, r#"{"kind":"inspect"}"#);
            assert!(ok_status.contains("200"), "{ok_status} {ok_body}");
            assert!(ok_body.contains("\"ok\":true"), "{ok_body}");
            let (bad_status, _) = tool_call(&tool, session, r#"{"kind":"cookie"}"#);
            assert!(bad_status.contains("403"), "an unknown kind was not refused: {bad_status}");

            // The person can see a live frame.
            let frame = orchestrator.frame(session).expect("a frame");
            assert!(frame.len() > 100, "empty frame");

            // The lease runs out and everything is torn down.
            std::thread::sleep(Duration::from_millis(500));
            orchestrator.sweep_expired();
            assert!(orchestrator.view(session).is_none(), "the clone outlived its lease");
            assert!(orchestrator.frame(session).is_err(), "the clone is still reachable after destroy");
        }

        fn build() -> (tempfile::TempDir, Arc<CloneSupervisor>, Arc<CloneBrowserTool>, Arc<CloneOrchestrator>) {
            let dir = tempfile::tempdir().unwrap();
            let supervisor = CloneSupervisor::with_ram_disk(
                dir.path().join("clones.json"),
                dir.path().join("mounts"),
                CloneConfig {
                    browser: Some(browser().unwrap()),
                    headless: true,
                    guarded: true,
                    ..CloneConfig::default()
                },
            );
            let tool_dir = std::path::PathBuf::from("/tmp").join(format!("bctl-{}", uuid::Uuid::new_v4().simple()));
            let tool = CloneBrowserTool::new(Arc::clone(&supervisor), tool_dir).unwrap();
            let orchestrator = CloneOrchestrator::new(Arc::clone(&supervisor), Arc::clone(&tool));
            (dir, supervisor, tool, orchestrator)
        }

        /// Real Chrome accepts both approved sessions and sends the primary
        /// session through the guard. All cookies are synthetic; no Keychain
        /// or user profile is read.
        #[test]
        fn approved_multi_host_sessions_work_in_a_real_browser() {
            if browser().is_none() { return }
            let (_dir, supervisor, tool, mut orchestrator) = build();
            Arc::get_mut(&mut orchestrator).unwrap().cookie_importer = |_, _, domains| {
                assert_eq!(domains, ["127.0.0.1", "auth.example.test"]);
                Ok(domains.iter().map(|domain| CookieSpec {
                    name: "sid".into(), value: "synthetic-shared-session".into(),
                    domain: domain.clone(), path: "/".into(), secure: false,
                    http_only: true, same_site: None,
                }).collect())
            };
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut stream = stream;
                    let mut buffer = [0u8; 8192];
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    let body = if request.contains("sid=synthetic-shared-session") {
                        "<html><body>Signed in fixture</body></html>"
                    } else { "<html><body>Signed out fixture</body></html>" };
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                }
            });
            let session = "multi-host";
            let pid = std::process::id();
            orchestrator.request_capability_context(session, pid).unwrap();
            let (status, body) = call_script(&tool, session, "clone-request-multi-host", r#"{"kind":"request","domain":"127.0.0.1","additionalDomains":["auth.example.test"]}"#);
            assert!(status.contains("200"));
            let request: Value = serde_json::from_str(&body).unwrap();
            let view = orchestrator.approve_request(session, request["requestId"].as_str().unwrap(), pid, SignInPath::Import, None, true).unwrap();
            let jar = supervisor.tool_call(&view.clone_id, "Storage.getCookies", json!({})).unwrap();
            for domain in ["127.0.0.1", "auth.example.test"] {
                assert!(jar["cookies"].as_array().unwrap().iter().any(|cookie| cookie["domain"] == domain && cookie["session"] == true));
            }
            let (status, body) = tool_call(&tool, session, &json!({"kind":"navigate","url":format!("http://127.0.0.1:{port}/")}).to_string());
            assert!(status.contains("200"), "{status} {body}");
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let (status, body) = tool_call(&tool, session, r#"{"kind":"get_text"}"#);
                assert!(status.contains("200"), "{status} {body}");
                if body.contains("Signed in fixture") { break; }
                assert!(Instant::now() < deadline, "the imported primary session did not sign in: {body}");
                std::thread::sleep(Duration::from_millis(100));
            }
            orchestrator.destroy(session);
            assert!(supervisor.clone_guard(&view.clone_id).is_none());
        }

        /// The whole agent-driven lifecycle: the agent asks Bridge for a clone,
        /// the person approves, the agent then acts (a mutating command that was
        /// refused before approval now succeeds), and it is destroyed.
        #[test]
        fn the_agent_asks_the_person_approves_and_the_agent_acts() {
            if browser().is_none() { return }
            let (_dir, _supervisor, tool, orchestrator) = build();
            let session = "sess-loop";
            let pid = std::process::id();

            // Turn 1: the agent is offered the "ask for a clone" capability and uses it.
            let ask = orchestrator.request_capability_context(session, pid).expect("ask capability");
            assert!(ask.contains("request"));
            let (status, _) = call_script(&tool, session, &format!("clone-request-{session}"), r#"{"kind":"request","domain":"127.0.0.1"}"#);
            assert!(status.contains("200"), "the request was refused: {status}");

            // Bridge now shows the person a pending request; no clone yet.
            assert_eq!(orchestrator.pending_request(session).as_deref(), Some("127.0.0.1"));
            assert!(orchestrator.view(session).is_none(), "a clone existed before approval");

            // The person approves. The clone is built and the agent gets its tool.
            orchestrator.approve_request(session, &orchestrator.pending_details(session).unwrap().id, pid, SignInPath::SignInInside, None, true).expect("approve builds the clone");
            orchestrator.hand_back(session);
            assert!(orchestrator.view(session).is_some(), "no clone after approval");
            assert!(orchestrator.pending_request(session).is_none(), "the request outlived approval");

            // The agent now acts: a mutating command (scroll) that was refused
            // before approval succeeds now.
            let (act_status, act_body) = tool_call(&tool, session, r#"{"kind":"scroll","x":10,"y":10,"deltaY":100}"#);
            assert!(act_status.contains("200"), "the approved agent could not act: {act_status} {act_body}");

            // Destroy tears it down and revokes the agent's access.
            orchestrator.destroy(session);
            assert!(orchestrator.view(session).is_none());
        }

        /// A mutating command is refused until the person approves.
        #[test]
        fn the_agent_cannot_act_before_approval() {
            if browser().is_none() { return }
            let (_dir, _supervisor, tool, orchestrator) = build();
            let session = "sess-noact";
            // A clone exists (say from a prior approval path) but this session's
            // actions are not approved: request one directly without approving.
            orchestrator
                .request_clone(session, "127.0.0.1", CloneBrowser::Chrome, SignInPath::SignInInside, Some(Duration::from_secs(60)), std::process::id())
                .unwrap();
            let (status, body) = tool_call(&tool, session, r#"{"kind":"scroll","x":1,"y":1,"deltaY":10}"#);
            assert!(status.contains("403"), "an unapproved action was allowed: {status} {body}");
            orchestrator.hand_back(session);
            let (read_status, _) = tool_call(&tool, session, r#"{"kind":"inspect"}"#);
            assert!(read_status.contains("200"), "reading should still work: {read_status}");
            orchestrator.destroy(session);
        }

        /// Two chats each get their own clone at the same time; destroying one
        /// leaves the other running.
        #[test]
        fn two_sessions_run_independent_clones_at_once() {
            if browser().is_none() { return }
            let (_dir, _supervisor, _tool, orchestrator) = build();
            let a = orchestrator.request_clone("chat-a", "127.0.0.1", CloneBrowser::Chrome, SignInPath::SignInInside, Some(Duration::from_secs(60)), std::process::id()).unwrap();
            let b = orchestrator.request_clone("chat-b", "127.0.0.1", CloneBrowser::Chrome, SignInPath::SignInInside, Some(Duration::from_secs(60)), std::process::id()).unwrap();
            assert_ne!(a.clone_id, b.clone_id, "the two chats shared a clone");
            assert!(orchestrator.view("chat-a").is_some() && orchestrator.view("chat-b").is_some());
            orchestrator.destroy("chat-a");
            assert!(orchestrator.view("chat-a").is_none(), "chat-a survived its destroy");
            assert!(orchestrator.view("chat-b").is_some(), "destroying chat-a took chat-b down");
            assert!(orchestrator.frame("chat-b").is_ok(), "chat-b's clone stopped working");
            orchestrator.destroy("chat-b");
        }

        /// The person takes over and types a login; input is refused until they
        /// do, and the agent's actions are paused while they hold it.
        #[test]
        fn the_person_takes_over_and_types_into_the_clone() {
            if browser().is_none() { return }
            let (_dir, supervisor, tool, orchestrator) = build();
            let session = "sess-typing";
            let pid = std::process::id();
            // Go through the real request -> approve flow so actions are approved.
            orchestrator.request_capability_context(session, pid).unwrap();
            call_script(&tool, session, &format!("clone-request-{session}"), r#"{"kind":"request","domain":"127.0.0.1"}"#);
            orchestrator.approve_request(session, &orchestrator.pending_details(session).unwrap().id, pid, SignInPath::SignInInside, None, true).unwrap();
            orchestrator.hand_back(session);
            let clone_id = orchestrator.view(session).unwrap().clone_id;
            supervisor
                .page_call(&clone_id, "Page.navigate", json!({ "url": "data:text/html,<input id=u style=%22position:fixed;inset:0;width:100%25;height:100%25;font-size:40px%22 autofocus>" }))
                .unwrap();
            std::thread::sleep(Duration::from_millis(400));

            // Before takeover: the person's input is refused, and the agent can act.
            assert!(orchestrator.forward_input(session, CloneInput::Type { text: "x".into() }).is_err());
            assert!(tool_call(&tool, session, r#"{"kind":"scroll","x":1,"y":1,"deltaY":5}"#).0.contains("200"));

            // Take over: the agent is paused, and the person can type.
            orchestrator.take_over(session);
            assert!(tool_call(&tool, session, r#"{"kind":"scroll","x":1,"y":1,"deltaY":5}"#).0.contains("403"), "the agent kept acting during takeover");
            orchestrator.forward_input(session, CloneInput::Click { x: 0.5, y: 0.5 }).unwrap();
            orchestrator.forward_input(session, CloneInput::Type { text: "hunter2".into() }).unwrap();
            let value = supervisor
                .page_call(&clone_id, "Runtime.evaluate", json!({ "expression": "document.getElementById('u').value", "returnByValue": true }))
                .unwrap();
            assert_eq!(value["result"]["value"].as_str(), Some("hunter2"), "the person's typing did not land");

            // Hand back: the agent's actions return (they were approved).
            orchestrator.hand_back(session);
            assert!(tool_call(&tool, session, r#"{"kind":"scroll","x":1,"y":1,"deltaY":5}"#).0.contains("200"), "the agent did not get control back");
            orchestrator.destroy(session);
        }

        /// The agent has full control of an approved clone: it sees the page
        /// (a screenshot in CSS pixels, a ref-addressed outline), clicks by ref,
        /// types, fills a select and a checkbox, presses keys, and runs page
        /// script. Turning vision off withholds screenshots and nothing else.
        #[test]
        fn the_agent_sees_and_fully_drives_an_approved_clone() {
            use base64::Engine as _;
            if browser().is_none() { return }
            let (_dir, supervisor, tool, orchestrator) = build();
            let session = "sess-full";
            let pid = std::process::id();
            orchestrator.request_capability_context(session, pid).unwrap();
            call_script(&tool, session, &format!("clone-request-{session}"), r#"{"kind":"request","domain":"127.0.0.1"}"#);
            orchestrator.approve_request(session, &orchestrator.pending_details(session).unwrap().id, pid, SignInPath::SignInInside, None, true).unwrap();
            orchestrator.hand_back(session);
            let clone_id = orchestrator.view(session).unwrap().clone_id;
            let page = r#"<h1>Checkout</h1><label for=e>Email</label><input id=e><select id=s><option>Red</option><option>Blue</option></select><input type=checkbox id=c aria-label=Agree><button id=b onclick="document.title='clicked '+document.getElementById('e').value">Pay</button><div id=k></div><script>addEventListener('keydown',e=>document.getElementById('k').textContent+=e.key)</script>"#;
            let url = format!("data:text/html;base64,{}", base64::engine::general_purpose::STANDARD.encode(page));
            supervisor.page_call(&clone_id, "Page.navigate", json!({ "url": url })).unwrap();
            std::thread::sleep(Duration::from_millis(500));
            let call = |body: &str| {
                let (status, reply) = tool_call(&tool, session, body);
                assert!(status.contains("200"), "{body} -> {status} {reply}");
                serde_json::from_str::<Value>(&reply).unwrap()
            };

            // It sees: a real PNG whose pixels are the page's CSS pixels.
            let shot = call(r#"{"kind":"screenshot"}"#);
            let png = std::fs::read(shot["path"].as_str().unwrap()).expect("the screenshot file exists");
            assert_eq!(&png[1..4], b"PNG");
            let dimension = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
            assert_eq!(f64::from(dimension(16)), shot["width"].as_f64().unwrap(), "png width is not css width");
            assert_eq!(f64::from(dimension(20)), shot["height"].as_f64().unwrap(), "png height is not css height");

            // It reads a ref-addressed outline, not a per-character tree.
            let outline = call(r#"{"kind":"read_page"}"#)["result"].as_str().unwrap().to_owned();
            assert!(outline.contains(r#"heading"#) || outline.contains(r#"h1 "Checkout""#), "{outline}");
            let reference = |needle: &str| outline.lines().find(|line| line.contains(needle))
                .and_then(|line| line.split(']').next()).map(|head| head.trim_start_matches('[').to_owned())
                .unwrap_or_else(|| panic!("{needle} not in {outline}"));
            let (email, select, agree, pay) = (reference("input:text"), reference("select"), reference("\"Agree\""), reference("button \"Pay\""));

            // It acts: click to focus, type, fill, check, press a key, click by ref.
            call(&format!(r#"{{"kind":"click","ref":"{email}"}}"#));
            call(r#"{"kind":"type","text":"a@b.co"}"#);
            call(&format!(r#"{{"kind":"form_input","ref":"{select}","value":"Blue"}}"#));
            call(&format!(r#"{{"kind":"form_input","ref":"{agree}","value":true}}"#));
            call(r#"{"kind":"key","key":"Enter"}"#);
            call(&format!(r#"{{"kind":"click","ref":"{pay}"}}"#));
            let state = call(r#"{"kind":"evaluate","expression":"({title:document.title,select:document.getElementById('s').value,agree:document.getElementById('c').checked,keys:document.getElementById('k').textContent})"}"#);
            assert_eq!(state["result"]["title"], "clicked a@b.co", "{state}");
            assert_eq!(state["result"]["select"], "Blue");
            assert_eq!(state["result"]["agree"], true);
            assert!(state["result"]["keys"].as_str().unwrap().contains("Enter"), "{state}");

            // The person's dock can draw where the agent last acted.
            let (px, py, action, age_ms) = orchestrator.agent_pointer(session).expect("a click leaves a pointer");
            assert_eq!(action, "click");
            assert!((0.0..=1.0).contains(&px) && (0.0..=1.0).contains(&py), "pointer outside the viewport: {px},{py}");
            assert!(age_ms < 10_000);
            call(r#"{"kind":"hover","x":0,"y":0}"#);
            let (hx, hy, action, _) = orchestrator.agent_pointer(session).unwrap();
            assert_eq!((hx, hy, action), (0.0, 0.0, "hover"));

            // Vision off withholds screenshots only.
            tool.set_vision(session, false);
            assert!(tool_call(&tool, session, r#"{"kind":"screenshot"}"#).0.contains("403"), "a blind clone returned a screenshot");
            call(r#"{"kind":"read_page","filter":"interactive"}"#);
            orchestrator.take_over(session);
            assert!(orchestrator.agent_pointer(session).is_none(), "the pointer outlived the hand-off");
            orchestrator.destroy(session);
            assert!(!std::path::Path::new(shot["path"].as_str().unwrap()).exists(), "the screenshot outlived its clone");
        }

        /// A clone whose lease has run out is swept away.
        #[test]
        fn an_expired_lease_is_swept() {
            if browser().is_none() { return }
            let (_dir, _supervisor, _tool, orchestrator) = build();
            orchestrator
                .request_clone("sess-ttl", "127.0.0.1", CloneBrowser::Chrome, SignInPath::SignInInside, Some(Duration::from_millis(1)), std::process::id())
                .unwrap();
            std::thread::sleep(Duration::from_millis(30));
            orchestrator.sweep_expired();
            assert!(orchestrator.view("sess-ttl").is_none(), "the expired clone was not swept");
        }
    }
}
