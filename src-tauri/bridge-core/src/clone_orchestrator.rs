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

struct Active {
    clone_id: String,
    domain: String,
    status: CloneStatus,
    sign_in_path: SignInPath,
    expires_at: Instant,
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

    pub fn view(&self, session_id: &str) -> Option<CloneView> {
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        let entry = active.get(session_id)?;
        Some(view_of(session_id, entry, entry.expires_at.saturating_duration_since(Instant::now())))
    }

    pub fn take_over(&self, session_id: &str) {
        self.set_status(session_id, CloneStatus::TakenOver);
    }

    pub fn hand_back(&self, session_id: &str) {
        self.set_status(session_id, CloneStatus::Acting);
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
            let mut stream = UnixStream::connect(tool.socket_path()).unwrap();
            // The capability the orchestrator minted is bound to the session; we
            // do not know the token here, so read it from the wrapper script the
            // tool wrote for this session.
            let script = std::fs::read_to_string(
                tool.socket_path()
                    .parent()
                    .unwrap()
                    .join(format!("clone-browser-{session}")),
            )
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
    }
}
