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
}

impl CloneOrchestrator {
    pub fn new(supervisor: Arc<CloneSupervisor>, tool: Arc<CloneBrowserTool>) -> Arc<Self> {
        Arc::new(Self {
            supervisor,
            tool,
            active: Mutex::new(HashMap::new()),
            default_ttl: Duration::from_secs(30 * 60),
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
        let domain = normalize_domain(domain)
            .ok_or_else(|| CloneError::Launch("invalid approved domain".into()))?;
        self.destroy(session_id);

        let info = self.supervisor.spawn_clone()?;
        let guard = self
            .supervisor
            .clone_guard(&info.id)
            .ok_or_else(|| CloneError::Launch("clone is not guarded".into()))?;

        // The clone may reach the approved site; nothing else.
        {
            let mut guard = guard.lock().unwrap_or_else(|p| p.into_inner());
            guard.allow_host(&domain);
        }

        // Sign in. Import copies the approved domain's cookies from the user's
        // browser; sign-in-inside reads nothing and the person logs in later.
        if path == SignInPath::Import {
            let cookies = import_cookies(browser.signin(), "Default", &domain).map_err(map_import)?;
            {
                let mut guard = guard.lock().unwrap_or_else(|p| p.into_inner());
                for cookie in &cookies {
                    guard.add_secret(&domain, &cookie.value);
                }
            }
            if let Err(error) = self.load_session(&info.id, cookies) {
                self.supervisor.destroy(&info.id)?;
                return Err(error);
            }
        }

        // Bind the agent tool to the agent's runtime process, not the clone.
        self.tool
            .capability_context(session_id, &info.id, runtime_pid, &domain);

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

    /// A fresh redacted frame of the clone, base64 PNG, for the dock's live
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
        self.tool.request_capability_context(session_id, runtime_pid)
    }

    /// The domain a session's agent has asked for, waiting on the person. The
    /// dock turns this into the Allow/Deny card.
    pub fn pending_request(&self, session_id: &str) -> Option<String> {
        self.tool.pending_request(session_id)
    }

    /// The person allowed the agent's request: spawn the clone for the asked
    /// domain (import the sign-in), and approve page actions on it.
    pub fn approve_request(&self, session_id: &str, runtime_pid: u32) -> Result<CloneView, CloneError> {
        let domain = self
            .tool
            .take_pending_request(session_id)
            .ok_or_else(|| CloneError::Launch("no pending clone request".into()))?;
        let view = self.request_clone(
            session_id,
            &domain,
            CloneBrowser::Chrome,
            SignInPath::Import,
            None,
            runtime_pid,
        )?;
        // The person approved the agent acting, so page actions are allowed.
        if let Some(entry) = self.active.lock().unwrap_or_else(|p| p.into_inner()).get_mut(session_id) {
            entry.actions_approved = true;
        }
        self.tool.allow_mutations(session_id);
        Ok(view)
    }

    /// The person denied the request; drop it.
    pub fn deny_request(&self, session_id: &str) {
        self.tool.clear_pending_request(session_id);
    }

    pub fn view(&self, session_id: &str) -> Option<CloneView> {
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        let entry = active.get(session_id)?;
        Some(view_of(session_id, entry, entry.expires_at.saturating_duration_since(Instant::now())))
    }

    /// The person takes control (to sign in, finish 2FA). The agent is paused:
    /// its page actions are refused until the clone is handed back.
    pub fn take_over(&self, session_id: &str) {
        self.set_status(session_id, CloneStatus::TakenOver);
        self.tool.revoke_mutations(session_id);
    }

    /// The person hands control back. The agent's page actions come back only
    /// if the person had approved them.
    pub fn hand_back(&self, session_id: &str) {
        self.set_status(session_id, CloneStatus::Acting);
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
        let clone_id = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(session_id)
            .map(|entry| entry.clone_id);
        if let Some(clone_id) = clone_id {
            let _ = self.supervisor.destroy(&clone_id);
            self.tool.revoke_session(session_id);
        }
    }

    /// Destroy every clone whose lease has run out. The host calls this on a
    /// timer; it is also what a 30-minute default comes to.
    pub fn sweep_expired(&self) {
        let now = Instant::now();
        let expired: Vec<String> = self
            .active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(session, _)| session.clone())
            .collect();
        for session in expired {
            self.destroy(&session);
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
    (!domain.is_empty()
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    .then_some(domain)
}

fn map_import(error: ImportError) -> CloneError {
    CloneError::Launch(format!("sign-in import: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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

            // The agent drives it through its real tool socket: a read command
            // returns, an out-of-contract one is refused.
            let (ok_status, ok_body) = tool_call(&tool, session, r#"{"kind":"screenshot"}"#);
            assert!(ok_status.contains("200"), "{ok_status} {ok_body}");
            assert!(ok_body.contains("\"ok\":true"), "{ok_body}");
            let (bad_status, _) = tool_call(&tool, session, r#"{"kind":"eval","expression":"1"}"#);
            assert!(bad_status.contains("403"), "eval was not refused: {bad_status}");

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
            orchestrator.approve_request(session, pid).expect("approve builds the clone");
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
            let (read_status, _) = tool_call(&tool, session, r#"{"kind":"screenshot"}"#);
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
            orchestrator.approve_request(session, pid).unwrap();
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
