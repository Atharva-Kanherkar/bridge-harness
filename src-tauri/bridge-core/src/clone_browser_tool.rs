//! Narrow agent command surface for one guarded browser clone.

use crate::browser_clone::CloneSupervisor;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};
use uuid::Uuid;

struct Capability {
    token: String,
    clone_id: String,
    runtime_pid: u32,
    domain: String,
}

/// Kinds that act on the page. The person approves them with the clone, and
/// takeover pauses all page access, reads included. Screenshots reach the agent
/// unless the person turned vision off for the clone.
const MUTATING_KINDS: [&str; 13] = [
    "click", "double_click", "triple_click", "right_click", "hover", "drag", "type", "key",
    "scroll", "navigate", "form_input", "focus", "evaluate",
];

/// A session's request capability: the token and agent process allowed to ask
/// for a clone, before any clone exists.
#[derive(Clone)]
struct RequestCapability {
    token: String,
    runtime_pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRequest {
    pub id: String,
    pub domain: String,
    pub runtime_pid: u32,
    pub extension_path: Option<String>,
    pub additional_domains: Vec<String>,
}

pub struct CloneBrowserTool {
    supervisor: Arc<CloneSupervisor>,
    socket: PathBuf,
    directory: PathBuf,
    capabilities: Mutex<HashMap<String, Capability>>,
    results: Mutex<HashMap<String, (String, String, Value)>>,
    /// The agent-facing "ask for a clone" capability, minted every turn so the
    /// agent can request one before any exists.
    request_caps: Mutex<HashMap<String, RequestCapability>>,
    /// A session's outstanding request (the domain the agent asked for), waiting
    /// for the person to approve or deny.
    pending: Mutex<HashMap<String, PendingRequest>>,
    answers: Mutex<HashMap<String, (String, Value)>>,
    paused: Mutex<HashSet<String>>,
    /// Sessions whose clone the person approved for page actions. A mutating
    /// kind is refused until the session is in here.
    mutable: Mutex<HashSet<String>>,
    /// Sessions whose person turned screenshots off. Vision is on otherwise.
    blind: Mutex<HashSet<String>>,
    operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl CloneBrowserTool {
    pub fn new(supervisor: Arc<CloneSupervisor>, directory: PathBuf) -> std::io::Result<Arc<Self>> {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        // A short socket name: the full path must clear SUN_LEN (~104 bytes on
        // macOS), so the caller should pass a short directory and the file name
        // stays small too.
        let socket = directory.join(format!("c{}.sock", &Uuid::new_v4().simple().to_string()[..10]));
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        let tool = Arc::new(Self {
            supervisor,
            socket,
            directory,
            capabilities: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
            request_caps: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            answers: Mutex::new(HashMap::new()),
            paused: Mutex::new(HashSet::new()),
            mutable: Mutex::new(HashSet::new()),
            blind: Mutex::new(HashSet::new()),
            operations: Mutex::new(HashMap::new()),
        });
        // The accept loop must not own the tool: otherwise dropping the core
        // leaves the listener, supervisor and every browser alive forever.
        let server = Arc::downgrade(&tool);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let Some(server) = server.upgrade() else { break };
                thread::spawn(move || server.handle(stream));
            }
        });
        Ok(tool)
    }

    /// Mint an unguessable capability scoped to this session, clone, and agent process.
    pub fn capability_context(
        &self,
        session: &str,
        clone_id: &str,
        runtime_pid: u32,
        domain: &str,
    ) -> Option<String> {
        self.session_operation(session);
        if self.supervisor.clone_guard(clone_id).is_none() {
            return None;
        }
        let domain = normalized_domain(domain)?;
        let token = Uuid::new_v4().to_string();
        self.capabilities.lock().ok()?.insert(
            session.to_owned(),
            Capability {
                token: token.clone(),
                clone_id: clone_id.to_owned(),
                runtime_pid,
                domain: domain.clone(),
            },
        );
        let safe_session: String = session
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let path = self.directory.join(format!("clone-browser-{safe_session}"));
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let script = format!("#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 2\nexec curl --silent --show-error --fail-with-body --unix-socket {} -H {} -H {} -H 'Content-Type: application/json' --data-binary \"$1\" http://localhost/v1/clone-browser\n",
            quote(&self.socket.to_string_lossy()), quote(&format!("Authorization: Bearer {token}")), quote(&format!("X-Bridge-Session: {session}")));
        fs::write(&path, script).ok()?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).ok()?;
        Some(format!(r#"Bridge browser tool: {tool}. A real Chrome, signed in as the person on {domain}. Call it with one JSON argument, e.g. {tool} '{{"kind":"screenshot"}}'.
See: screenshot (saves a PNG and returns its path; open it with your image viewer), read_page (every element with a ref like r12 and its x,y; filter=interactive for controls only), get_text.
Act: click/double_click/right_click/hover (ref, or x,y in screenshot pixels), drag (x,y to toX,toY), type (text into the focused field), key (e.g. "Enter", "cmd+a", "shift+Tab"), form_input (ref, value), scroll (deltaY, or ref to scroll it into view), navigate (url, or back/forward/reload), focus (ref), evaluate (expression: page JavaScript), wait (ms).
Work in a loop: look (screenshot or read_page), act, look again. status tells you if the person has taken over (all calls pause until they hand it back). Sites allowed: {domain} and its approved dependencies. The browser is destroyed when your turn ends."#, tool = path.display()))
    }

    /// The unix socket the tool script talks to. The orchestrator's end-to-end
    /// test connects here directly to exercise the agent-facing path.
    #[cfg(test)]
    pub(crate) fn socket_path(&self) -> &std::path::Path {
        &self.socket
    }

    /// Mint the session's "ask for a clone" capability, injected every turn so
    /// the agent can request one before any exists. Returns the instruction the
    /// agent reads.
    pub fn request_capability_context(&self, session: &str, runtime_pid: u32) -> Option<String> {
        self.session_operation(session);
        let token = Uuid::new_v4().to_string();
        self.request_caps.lock().ok()?.insert(
            session.to_owned(),
            RequestCapability { token: token.clone(), runtime_pid },
        );
        let safe_session: String = session
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let path = self.directory.join(format!("clone-request-{safe_session}"));
        let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
        let script = format!("#!/bin/sh\n[ \"$#\" -eq 1 ] || exit 2\nexec curl --silent --show-error --fail-with-body --unix-socket {} -H {} -H {} -H 'Content-Type: application/json' --data-binary \"$1\" http://localhost/v1/clone-browser\n",
            quote(&self.socket.to_string_lossy()), quote(&format!("Authorization: Bearer {token}")), quote(&format!("X-Bridge-Session: {session}")));
        fs::write(&path, script).ok()?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).ok()?;
        Some(format!(r#"When a task needs a real browser (a signed-in site, a web app to check or use, a UI to test), ask for one; the person approves it: {tool} '{{"kind":"request","domain":"example.com"}}'. It opens signed in as the person, and you can see and drive it. The result includes requestId. Poll the same tool with {{"kind":"request_status","requestId":...}} every two seconds until awaiting=false (approval can take minutes; do not end your turn); it returns the browser tool's instructions. Optional additionalDomains lists CDN or sign-in hosts the site needs; optional extensionPath is an absolute directory of an extension under test. The browser is destroyed when your turn ends."#, tool = path.display()))
    }

    /// The domain a session's agent has asked for, awaiting the person's answer.
    pub fn pending_request(&self, session: &str) -> Option<String> {
        self.pending.lock().ok()?.get(session).map(|request| request.domain.clone())
    }

    /// Serialize state changes and in-flight page commands within one chat.
    /// Different chats retain independent gates.
    pub(crate) fn session_operation(&self, session: &str) -> Arc<Mutex<()>> {
        Arc::clone(self.operations.lock().unwrap_or_else(|p| p.into_inner())
            .entry(session.to_owned()).or_insert_with(|| Arc::new(Mutex::new(()))))
    }

    pub fn pending_requests(&self) -> Vec<bridge_protocol::messages::CloneRequest> {
        self.pending.lock().unwrap_or_else(|p| p.into_inner()).iter().map(|(session, request)| bridge_protocol::messages::CloneRequest {
            session_id: session.clone(), request_id: request.id.clone(), domain: request.domain.clone(), extension_path: request.extension_path.clone(), additional_domains: Some(request.additional_domains.clone()),
        }).collect()
    }

    pub fn pending_details(&self, session: &str) -> Option<PendingRequest> {
        self.pending.lock().ok()?.get(session).cloned()
    }

    pub fn take_pending_request(&self, session: &str, id: &str, runtime_pid: u32) -> Result<PendingRequest, String> {
        let mut pending = self.pending.lock().map_err(|_| "request unavailable")?;
        let request = pending.get(session).ok_or("no pending clone request")?;
        if request.id != id || request.runtime_pid != runtime_pid {
            return Err("clone request changed or its agent restarted; review the current request".into());
        }
        Ok(pending.remove(session).unwrap())
    }

    pub fn answer_request(&self, session: &str, id: &str, answer: Value) {
        self.answers.lock().unwrap_or_else(|p| p.into_inner()).insert(session.to_owned(), (id.to_owned(), answer));
    }

    pub fn clear_pending_request(&self, session: &str) {
        if let Some(request) = self.pending.lock().unwrap_or_else(|p| p.into_inner()).remove(session) {
            self.answer_request(session, &request.id, json!({"ok":false,"awaiting":false,"error":"request denied"}));
        }
    }

    pub fn pause(&self, session: &str) {
        self.paused.lock().unwrap_or_else(|p| p.into_inner()).insert(session.to_owned());
        self.revoke_mutations(session);
        self.results.lock().unwrap_or_else(|p| p.into_inner()).retain(|_, (owner, _, _)| owner != session);
    }

    pub fn resume(&self, session: &str) {
        self.paused.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
    }

    // Replacing a browser preserves the authenticated request channel so its
    // status operation can receive the approval result in the requesting turn.
    pub fn revoke_browser(&self, session: &str) {
        self.capabilities.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
        self.mutable.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
        self.paused.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
        self.results.lock().unwrap_or_else(|p| p.into_inner()).retain(|_, (owner, _, _)| owner != session);
        self.blind.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
        let safe = safe_session(session);
        let _ = fs::remove_file(self.directory.join(format!("clone-browser-{safe}")));
        if let Ok(entries) = fs::read_dir(self.directory.join("shots")) {
            for entry in entries.flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with(&format!("{safe}-"))) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    /// Approve page actions for a session's clone: mutating kinds are allowed
    /// from now on.
    pub fn allow_mutations(&self, session: &str) {
        if let Ok(mut set) = self.mutable.lock() {
            set.insert(session.to_owned());
        }
    }

    /// The person's vision choice for this clone: screenshots reach the agent
    /// when `enabled`.
    pub fn set_vision(&self, session: &str, enabled: bool) {
        let mut blind = self.blind.lock().unwrap_or_else(|p| p.into_inner());
        if enabled { blind.remove(session); } else { blind.insert(session.to_owned()); }
    }

    pub fn vision(&self, session: &str) -> bool {
        !self.blind.lock().unwrap_or_else(|p| p.into_inner()).contains(session)
    }

    /// Pause the agent's page actions for a session (the person took over).
    pub fn revoke_mutations(&self, session: &str) {
        if let Ok(mut set) = self.mutable.lock() {
            set.remove(session);
        }
    }

    pub fn revoke_session(&self, session: &str) {
        self.revoke_browser(session);
        self.answers.lock().unwrap_or_else(|p| p.into_inner()).remove(session);
        let _ = self.request_caps.lock().map(|mut m| m.remove(session));
        let _ = self.pending.lock().map(|mut m| m.remove(session));
        let _ = self.mutable.lock().map(|mut m| m.remove(session));
        let _ = self.results.lock().map(|mut results| results.retain(|_, (owner, _, _)| owner != session));
        let safe: String = session
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let _ = fs::remove_file(self.directory.join(format!("clone-browser-{safe}")));
        let _ = fs::remove_file(self.directory.join(format!("clone-request-{safe}")));
    }

    fn handle(&self, mut stream: UnixStream) {
        let peer = peer_pid(&stream);
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut input = Vec::new();
        let mut chunk = [0u8; 4096];
        while input.len() < 1024 * 1024 {
            let Ok(count) = stream.read(&mut chunk) else {
                break;
            };
            if count == 0 {
                break;
            }
            input.extend_from_slice(&chunk[..count]);
            if let Some(split) = input.windows(4).position(|v| v == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&input[..split]);
                let length = head.lines().find_map(|line| {
                    line.split_once(':').and_then(|(key, value)| {
                        key.eq_ignore_ascii_case("Content-Length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                });
                if length.is_some_and(|length| input.len() >= split + 4 + length) {
                    break;
                }
            }
        }
        let request = String::from_utf8_lossy(&input);
        let (headers, body) = request.split_once("\r\n\r\n").unwrap_or(("", ""));
        let header = |name: &str| {
            headers.lines().find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.trim()
                    .eq_ignore_ascii_case(name)
                    .then_some(value.trim())
            })
        };
        let session = header("X-Bridge-Session").unwrap_or("");
        let token = header("Authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        let reply = serde_json::from_str::<Value>(body)
            .map_err(|_| "invalid request".to_owned())
            .and_then(|request| self.execute(session, token, peer, request));
        let (status, value) = match reply {
            Ok(value) => ("200 OK", value),
            Err(error) => ("403 Forbidden", json!({"ok":false,"error":error})),
        };
        let bytes = serde_json::to_vec(&value).unwrap_or_default();
        let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
        let _ = stream.write_all(&bytes);
    }

    fn execute(
        &self,
        session: &str,
        token: &str,
        peer: Option<u32>,
        request: Value,
    ) -> Result<Value, String> {
        let operation = self.operations.lock().map_err(|_| "capability unavailable")?
            .get(session).cloned().ok_or("capability unavailable")?;
        let _operation = operation.lock().unwrap_or_else(|p| p.into_inner());
        // "request" is the one kind an agent can call before any clone exists:
        // it asks the person for a clone. It rides the session's request
        // capability, not a per-clone one.
        let kind = request.get("kind").and_then(Value::as_str).unwrap_or("");
        if matches!(kind, "request" | "request_status") {
            let cap = self.request_caps.lock().map_err(|_| "capability unavailable")?
                .get(session).cloned().ok_or("capability unavailable")?;
            if cap.token != token || cap.runtime_pid == 0 || !peer.is_some_and(|pid| descendant_of(pid, cap.runtime_pid)) {
                return Err("capability invalid".into());
            }
            if kind == "request_status" {
                let id = request.get("requestId").and_then(Value::as_str).ok_or("missing requestId")?;
                if let Some(pending) = self.pending_details(session) {
                    if pending.id != id { return Err("request changed".into()); }
                    return Ok(json!({"ok":true,"awaiting":true,"requestId":id}));
                }
                let answers = self.answers.lock().map_err(|_| "answer unavailable")?;
                let (answer_id, answer) = answers.get(session).ok_or("request no longer available")?;
                if answer_id != id { return Err("request changed".into()); }
                return Ok(answer.clone());
            }
            let domain = request.get("domain").and_then(Value::as_str)
                .and_then(normalized_domain).ok_or("a valid domain is required")?;
            let extension_path = request.get("extensionPath").and_then(Value::as_str).map(str::to_owned);
            if extension_path.as_ref().is_some_and(|path| !std::path::Path::new(path).is_absolute() || !std::path::Path::new(path).join("manifest.json").is_file()) {
                return Err("extensionPath must be an absolute directory containing manifest.json".into());
            }
            let mut additional_domains = Vec::new();
            if let Some(hosts) = request.get("additionalDomains") {
                let hosts = hosts.as_array().ok_or("additionalDomains must be an array")?;
                if hosts.len() > 16 { return Err("at most 16 additional domains are allowed".into()); }
                for host in hosts {
                    let host = host.as_str().and_then(normalized_domain).ok_or("invalid additional domain")?;
                    if host != domain { additional_domains.push(host); }
                }
                additional_domains.sort(); additional_domains.dedup();
            }
            let mut pending = self.pending.lock().map_err(|_| "request unavailable")?;
            if let Some(existing) = pending.get(session) {
                if existing.domain != domain || existing.extension_path != extension_path || existing.additional_domains != additional_domains || existing.runtime_pid != cap.runtime_pid {
                    return Err("a different browser request is already awaiting approval".into());
                }
                return Ok(json!({"ok":true,"requested":domain,"awaiting":true,"requestId":existing.id}));
            }
            let id = Uuid::new_v4().to_string();
            pending.insert(session.to_owned(), PendingRequest { id: id.clone(), domain: domain.clone(), runtime_pid: cap.runtime_pid, extension_path, additional_domains });
            self.answers.lock().map_err(|_| "answer unavailable")?.remove(session);
            return Ok(json!({"ok":true,"requested":domain,"awaiting":true,"requestId":id}));
        }

        let caps = self
            .capabilities
            .lock()
            .map_err(|_| "capability unavailable")?;
        let cap = caps.get(session).ok_or("capability unavailable")?;
        if cap.token != token || !peer.is_some_and(|pid| descendant_of(pid, cap.runtime_pid)) {
            return Err("capability invalid".into());
        }
        let clone_id = cap.clone_id.clone();
        let domain = cap.domain.clone();
        drop(caps);
        if kind == "status" {
            let paused = self.paused.lock().map_err(|_| "clone unavailable")?.contains(session);
            if paused { return Ok(json!({"ok":true,"paused":true})); }
            let blocked = self.supervisor.clone_guard(&clone_id).map(|guard| {
                let guard = guard.lock().unwrap_or_else(|p| p.into_inner());
                let mut hosts: Vec<String> = guard.blocked().iter().map(|request| request.host.clone()).collect();
                hosts.sort(); hosts.dedup();
                let mut blocked = json!(hosts);
                guard.scrub_response(&mut blocked);
                blocked
            }).unwrap_or_else(|| json!([]));
            return Ok(json!({"ok":true,"paused":paused,"blockedHosts":blocked}));
        }
        if self.paused.lock().map_err(|_| "clone unavailable")?.contains(session) {
            return Err("the person controls the browser; all agent access is paused".into());
        }
        let guard = self
            .supervisor
            .clone_guard(&clone_id)
            .ok_or("clone unavailable")?;
        if kind == "result" {
            let id = request
                .get("commandId")
                .and_then(Value::as_str)
                .ok_or("missing commandId")?;
            let results = self.results.lock().map_err(|_| "result unavailable")?;
            let (owner, target, value) = results.get(id).ok_or("result unavailable")?;
            if owner != session || target != &clone_id {
                return Err("result unavailable".into());
            }
            let mut value = value.clone();
            guard
                .lock()
                .map_err(|_| "clone guard unavailable")?
                .scrub_response(&mut value);
            return Ok(json!({"ok":true,"result":value}));
        }
        let kind = canonical_kind(kind).ok_or("unknown kind; see the tool instructions for the list")?;
        let approved = self.mutable.lock().map(|set| set.contains(session)).unwrap_or(false);
        if MUTATING_KINDS.contains(&kind) && !approved {
            return Err("page actions are not approved for this clone".into());
        }
        if kind == "screenshot" {
            if self.blind.lock().map(|set| set.contains(session)).unwrap_or(true) {
                return Err("the person turned off screenshots for this clone; use read_page".into());
            }
            let mut shot = self.screenshot(session, &clone_id)?;
            guard.lock().map_err(|_| "clone guard unavailable")?.scrub_response(&mut shot);
            return Ok(shot);
        }
        if kind == "navigate" {
            if let Some(url) = request.get("url").and_then(Value::as_str).filter(|url| !matches!(*url, "back" | "forward" | "reload")) {
                let host = navigable_host(url)?;
                if !guard.lock().map_err(|_| "clone guard unavailable")?.allows_host(&host) {
                    return Err(format!("navigation outside the approved domains ({domain} and its approved dependencies)"));
                }
            }
        }
        let mut result = self.perform(&clone_id, kind, &request)?;
        guard
            .lock()
            .map_err(|_| "clone guard unavailable")?
            .scrub_response(&mut result);
        let id = Uuid::new_v4().to_string();
        let mut results = self.results.lock().map_err(|_| "result unavailable")?;
        if results.values().filter(|(owner, _, _)| owner == session).count() >= 128 {
            results.retain(|_, (owner, _, _)| owner != session);
        }
        results.insert(id.clone(), (session.to_owned(), clone_id, result.clone()));
        Ok(json!({"ok":true,"commandId":id,"result":result}))
    }

    fn page(&self, clone_id: &str, method: &str, params: Value) -> Result<Value, String> {
        self.supervisor
            .page_call(clone_id, method, params)
            .map_err(|error| format!("browser command failed: {error}"))
    }

    /// Run page JavaScript and return its JSON value, or the page's exception.
    fn evaluate(&self, clone_id: &str, expression: &str) -> Result<Value, String> {
        let reply = self.page(clone_id, "Runtime.evaluate", json!({
            "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true,
        }))?;
        if let Some(details) = reply.get("exceptionDetails") {
            let message = details.pointer("/exception/description").and_then(Value::as_str)
                .or_else(|| details.get("text").and_then(Value::as_str))
                .unwrap_or("script threw");
            return Err(format!("page script failed: {message}"));
        }
        Ok(reply.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    /// Call one of the helper functions below with a JSON argument.
    fn call_js(&self, clone_id: &str, function: &str, argument: Value) -> Result<Value, String> {
        self.evaluate(clone_id, &format!("({function})({argument})"))
    }

    /// The CSS-pixel point a pointer action targets: `ref` (from read_page),
    /// scrolled into view, or explicit `x`/`y` (the screenshot's pixels).
    fn point(&self, clone_id: &str, request: &Value, x_key: &str, y_key: &str, ref_key: &str) -> Result<(f64, f64), String> {
        if let Some(reference) = request.get(ref_key).and_then(Value::as_str) {
            let point = self.call_js(clone_id, REF_POINT_JS, json!(reference))?;
            let x = point.get("x").and_then(Value::as_f64);
            let y = point.get("y").and_then(Value::as_f64);
            return match (x, y) {
                (Some(x), Some(y)) => Ok((x, y)),
                _ => Err(format!("unknown ref {reference}; call read_page again")),
            };
        }
        let number = |key: &str| request.get(key).and_then(Value::as_f64).filter(|n| n.is_finite())
            .ok_or_else(|| format!("give {ref_key} or {x_key}/{y_key}"));
        Ok((number(x_key)?, number(y_key)?))
    }

    fn mouse(&self, clone_id: &str, kind: &str, (x, y): (f64, f64), button: &str, clicks: i64, modifiers: i64) -> Result<(), String> {
        let buttons = match (kind, button) { ("mouseReleased" | "mouseMoved", _) => 0, (_, "right") => 2, (_, "middle") => 4, _ => 1 };
        self.page(clone_id, "Input.dispatchMouseEvent", json!({
            "type": kind, "x": x, "y": y, "button": if kind == "mouseMoved" { "none" } else { button },
            "buttons": buttons, "clickCount": clicks, "modifiers": modifiers,
        })).map(|_| ())
    }

    fn screenshot(&self, session: &str, clone_id: &str) -> Result<Value, String> {
        use base64::Engine as _;
        let view = self.evaluate(clone_id, "({w:innerWidth,h:innerHeight,dpr:devicePixelRatio||1,sx:scrollX,sy:scrollY,url:location.href,title:document.title})")?;
        let number = |key: &str| view.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        let (width, height, dpr) = (number("w").max(1.0), number("h").max(1.0), number("dpr").max(0.1));
        // Scale by 1/dpr so one screenshot pixel is one CSS pixel: the
        // coordinates the agent reads off the image are the ones it clicks.
        let reply = self.page(clone_id, "Page.captureScreenshot", json!({
            "format": "png",
            "clip": {"x": number("sx"), "y": number("sy"), "width": width, "height": height, "scale": 1.0 / dpr},
        }))?;
        let data = reply.get("data").and_then(Value::as_str).ok_or("no frame")?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|_| "invalid frame")?;
        let shots = self.directory.join("shots");
        fs::create_dir_all(&shots).map_err(|_| "screenshot directory unavailable")?;
        let _ = fs::set_permissions(&shots, fs::Permissions::from_mode(0o700));
        let safe = safe_session(session);
        // Keep only the latest few frames per chat on disk.
        if let Ok(entries) = fs::read_dir(&shots) {
            let mut mine: Vec<_> = entries.flatten().filter(|entry| entry.file_name().to_string_lossy().starts_with(&format!("{safe}-"))).collect();
            mine.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());
            while mine.len() >= 8 { let _ = fs::remove_file(mine.remove(0).path()); }
        }
        let path = shots.join(format!("{safe}-{}.png", &Uuid::new_v4().simple().to_string()[..8]));
        fs::write(&path, bytes).map_err(|_| "screenshot could not be saved")?;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        Ok(json!({
            "ok": true, "path": path.display().to_string(), "width": width.round(), "height": height.round(),
            "url": view.get("url"), "title": view.get("title"),
            "hint": "Open the PNG with your image viewer to see the page. Its pixels are the x/y that click, hover, drag and scroll take.",
        }))
    }

    fn perform(&self, clone_id: &str, kind: &str, request: &Value) -> Result<Value, String> {
        let text = |key: &str| request.get(key).and_then(Value::as_str).ok_or_else(|| format!("missing {key}"));
        match kind {
            "read_page" => {
                let filter = request.get("filter").and_then(Value::as_str).unwrap_or("all");
                Ok(json!(self.call_js(clone_id, READ_PAGE_JS, json!({"filter": filter}))?))
            }
            "get_text" => self.evaluate(clone_id, GET_TEXT_JS),
            "wait" => {
                let ms = request.get("ms").and_then(Value::as_u64).unwrap_or(1000).min(10_000);
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(json!({"waited": ms}))
            }
            "click" | "double_click" | "triple_click" | "right_click" => {
                let point = self.point(clone_id, request, "x", "y", "ref")?;
                let button = if kind == "right_click" { "right" } else { request.get("button").and_then(Value::as_str).unwrap_or("left") };
                if !matches!(button, "left" | "right" | "middle") { return Err("button must be left, right or middle".into()); }
                let clicks = match kind { "double_click" => 2, "triple_click" => 3, _ => request.get("clicks").and_then(Value::as_i64).unwrap_or(1).clamp(1, 3) };
                let modifiers = modifier_bits(request.get("modifiers").and_then(Value::as_str).unwrap_or(""))?;
                self.mouse(clone_id, "mouseMoved", point, button, 0, modifiers)?;
                for count in 1..=clicks {
                    self.mouse(clone_id, "mousePressed", point, button, count, modifiers)?;
                    self.mouse(clone_id, "mouseReleased", point, button, count, modifiers)?;
                }
                Ok(json!({"x": point.0, "y": point.1}))
            }
            "hover" => {
                let point = self.point(clone_id, request, "x", "y", "ref")?;
                self.mouse(clone_id, "mouseMoved", point, "none", 0, 0)?;
                Ok(json!({"x": point.0, "y": point.1}))
            }
            "drag" => {
                let from = self.point(clone_id, request, "x", "y", "ref")?;
                let to = self.point(clone_id, request, "toX", "toY", "toRef")?;
                self.mouse(clone_id, "mouseMoved", from, "left", 0, 0)?;
                self.mouse(clone_id, "mousePressed", from, "left", 1, 0)?;
                for step in 1..=8 {
                    let t = f64::from(step) / 8.0;
                    let point = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                    self.page(clone_id, "Input.dispatchMouseEvent", json!({"type":"mouseMoved","x":point.0,"y":point.1,"button":"left","buttons":1}))?;
                }
                self.mouse(clone_id, "mouseReleased", to, "left", 1, 0)?;
                Ok(json!({"from": [from.0, from.1], "to": [to.0, to.1]}))
            }
            "type" => self.page(clone_id, "Input.insertText", json!({"text": text("text")?})),
            "key" => {
                let repeat = request.get("repeat").and_then(Value::as_u64).unwrap_or(1).clamp(1, 50);
                for combo in text("key")?.split_whitespace() {
                    let (event, commands) = key_event(combo)?;
                    for _ in 0..repeat {
                        let mut down = event.clone();
                        down["type"] = json!(if event.get("text").is_some() { "keyDown" } else { "rawKeyDown" });
                        if !commands.is_empty() { down["commands"] = json!(commands); }
                        self.page(clone_id, "Input.dispatchKeyEvent", down)?;
                        let mut up = event.clone();
                        up["type"] = json!("keyUp");
                        if let Some(map) = up.as_object_mut() { map.remove("text"); map.remove("unmodifiedText"); }
                        self.page(clone_id, "Input.dispatchKeyEvent", up)?;
                    }
                }
                Ok(json!({"pressed": text("key")?}))
            }
            "scroll" => {
                if let Some(reference) = request.get("ref").and_then(Value::as_str) {
                    if self.call_js(clone_id, REF_POINT_JS, json!(reference))?.is_null() {
                        return Err(format!("unknown ref {reference}; call read_page again"));
                    }
                    return Ok(json!({"scrolledTo": reference}));
                }
                let view = self.evaluate(clone_id, "({w:innerWidth,h:innerHeight})")?;
                let x = request.get("x").and_then(Value::as_f64).unwrap_or(view["w"].as_f64().unwrap_or(800.0) / 2.0);
                let y = request.get("y").and_then(Value::as_f64).unwrap_or(view["h"].as_f64().unwrap_or(600.0) / 2.0);
                let delta_x = request.get("deltaX").and_then(Value::as_f64).unwrap_or(0.0);
                let delta_y = request.get("deltaY").and_then(Value::as_f64).unwrap_or(if delta_x == 0.0 { 600.0 } else { 0.0 });
                self.page(clone_id, "Input.dispatchMouseEvent", json!({"type":"mouseWheel","x":x,"y":y,"deltaX":delta_x,"deltaY":delta_y}))?;
                std::thread::sleep(std::time::Duration::from_millis(150));
                self.evaluate(clone_id, "({scrollX,scrollY,pageHeight:document.documentElement.scrollHeight})")
            }
            "navigate" => {
                let target = text("url")?;
                match target {
                    "reload" => { self.page(clone_id, "Page.reload", json!({}))?; }
                    "back" | "forward" => {
                        let history = self.page(clone_id, "Page.getNavigationHistory", json!({}))?;
                        let current = history.get("currentIndex").and_then(Value::as_i64).unwrap_or(0);
                        let index = if target == "back" { current - 1 } else { current + 1 };
                        let entry = history.get("entries").and_then(Value::as_array)
                            .and_then(|entries| usize::try_from(index).ok().and_then(|index| entries.get(index)))
                            .ok_or_else(|| format!("no page to go {target} to"))?;
                        self.page(clone_id, "Page.navigateToHistoryEntry", json!({"entryId": entry["id"]}))?;
                    }
                    url => {
                        let reply = self.page(clone_id, "Page.navigate", json!({"url": url}))?;
                        if let Some(error) = reply.get("errorText").and_then(Value::as_str) {
                            return Err(format!("navigation failed: {error}"));
                        }
                    }
                }
                // Return once the new document has loaded (bounded).
                std::thread::sleep(std::time::Duration::from_millis(200));
                for _ in 0..40 {
                    if self.evaluate(clone_id, "document.readyState").ok().as_ref().and_then(Value::as_str) == Some("complete") { break; }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                self.evaluate(clone_id, "({url:location.href,title:document.title})")
            }
            "form_input" => {
                let value = request.get("value").cloned().ok_or("missing value")?;
                self.call_js(clone_id, FORM_INPUT_JS, json!({"ref": text("ref")?, "value": value}))
            }
            "focus" => self.call_js(clone_id, FOCUS_JS, json!(text("ref")?)),
            "evaluate" => self.evaluate(clone_id, text("expression")?),
            _ => Err("unknown kind".into()),
        }
    }
}

impl Drop for CloneBrowserTool {
    fn drop(&mut self) {
        let sessions: HashSet<String> = self.capabilities.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned()
            .chain(self.request_caps.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned()).collect();
        for session in sessions {
            self.revoke_session(&session);
        }
        // Wake the blocking accept loop; its Weak can no longer be upgraded.
        let _ = UnixStream::connect(&self.socket);
        let _ = fs::remove_file(&self.socket);
    }
}

fn normalized_domain(domain: &str) -> Option<String> {
    let d = domain.trim().to_ascii_lowercase();
    (d.len() <= 253 && d.contains('.') && d.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })).then_some(d)
}

/// Accept the names other browser tools use for the same action.
fn canonical_kind(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "screenshot" => "screenshot",
        "read_page" | "inspect" | "find" => "read_page",
        "get_text" | "get_page_text" | "text" => "get_text",
        "wait" => "wait",
        "click" | "left_click" => "click",
        "double_click" => "double_click",
        "triple_click" => "triple_click",
        "right_click" => "right_click",
        "hover" | "move" => "hover",
        "drag" | "left_click_drag" => "drag",
        "type" => "type",
        "key" | "press" => "key",
        "scroll" | "scroll_to" => "scroll",
        "navigate" | "goto" => "navigate",
        "form_input" | "fill" | "select" => "form_input",
        "focus" => "focus",
        "evaluate" | "javascript" | "eval" => "evaluate",
        _ => return None,
    })
}

fn navigable_host(url: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(url).map_err(|_| "invalid URL; give an absolute http(s) URL, or back, forward, reload")?;
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return Err("only plain http(s) URLs can be opened".into());
    }
    Ok(url.host_str().ok_or("invalid host")?.to_ascii_lowercase())
}

fn safe_session(session: &str) -> String {
    session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

/// CDP modifier bits for "shift+ctrl" style lists.
fn modifier_bits(list: &str) -> Result<i64, String> {
    let mut bits = 0;
    for name in list.split(['+', ',', ' ']).filter(|name| !name.is_empty()) {
        bits |= match name.to_ascii_lowercase().as_str() {
            "alt" | "option" | "opt" => 1,
            "ctrl" | "control" => 2,
            "cmd" | "command" | "meta" | "super" => 4,
            "shift" => 8,
            other => return Err(format!("unknown modifier {other}")),
        };
    }
    Ok(bits)
}

/// One key or chord ("Enter", "a", "cmd+a", "shift+Tab") as a CDP key event,
/// plus the editing commands macOS needs for shortcut chords.
fn key_event(combo: &str) -> Result<(Value, Vec<&'static str>), String> {
    let (modifier_list, key) = match combo.rsplit_once('+') {
        Some((mods, key)) if !key.is_empty() => (mods, key),
        _ => ("", combo),
    };
    let modifiers = modifier_bits(modifier_list)?;
    let named: Option<(&str, &str, i64, Option<&str>)> = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(("Enter", "Enter", 13, Some("\r"))),
        "tab" => Some(("Tab", "Tab", 9, None)),
        "backspace" => Some(("Backspace", "Backspace", 8, None)),
        "delete" | "del" => Some(("Delete", "Delete", 46, None)),
        "escape" | "esc" => Some(("Escape", "Escape", 27, None)),
        "space" => Some((" ", "Space", 32, Some(" "))),
        "arrowup" | "up" => Some(("ArrowUp", "ArrowUp", 38, None)),
        "arrowdown" | "down" => Some(("ArrowDown", "ArrowDown", 40, None)),
        "arrowleft" | "left" => Some(("ArrowLeft", "ArrowLeft", 37, None)),
        "arrowright" | "right" => Some(("ArrowRight", "ArrowRight", 39, None)),
        "home" => Some(("Home", "Home", 36, None)),
        "end" => Some(("End", "End", 35, None)),
        "pageup" => Some(("PageUp", "PageUp", 33, None)),
        "pagedown" => Some(("PageDown", "PageDown", 34, None)),
        _ => None,
    };
    let shortcut = modifiers & (2 | 4) != 0;
    let mut event = if let Some((name, code, vk, text)) = named {
        let mut event = json!({"key": name, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers});
        if let (Some(text), false) = (text, shortcut) { event["text"] = json!(text); event["unmodifiedText"] = json!(text); }
        event
    } else {
        let mut chars = key.chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else { return Err(format!("unknown key {key}")); };
        let upper = ch.to_ascii_uppercase();
        let code = if ch.is_ascii_alphabetic() { format!("Key{upper}") } else if ch.is_ascii_digit() { format!("Digit{ch}") } else { String::new() };
        let shown = if modifiers & 8 != 0 { upper.to_string() } else { ch.to_string() };
        let mut event = json!({"key": shown, "code": code, "windowsVirtualKeyCode": upper as i64, "modifiers": modifiers});
        if !shortcut { event["text"] = json!(shown); event["unmodifiedText"] = json!(ch.to_string()); }
        event
    };
    let commands = if shortcut {
        match key.to_ascii_lowercase().as_str() {
            "a" => vec!["selectAll"], "c" => vec!["copy"], "x" => vec!["cut"], "v" => vec!["paste"],
            "z" if modifiers & 8 != 0 => vec!["redo"], "z" => vec!["undo"],
            _ => vec![],
        }
    } else { vec![] };
    if event.get("code").and_then(Value::as_str) == Some("") { if let Some(map) = event.as_object_mut() { map.remove("code"); } }
    Ok((event, commands))
}

/// A compact, ref-addressed outline of the page: every visible interactive
/// element (and, unless filter=interactive, headings and text) with its
/// centre point in CSS pixels. Refs stay stable across calls on one document.
const READ_PAGE_JS: &str = r#"({filter}) => {
  const interactive = 'a[href],button,input:not([type=hidden]),select,textarea,summary,[role=button],[role=link],[role=checkbox],[role=radio],[role=tab],[role=menuitem],[role=option],[role=switch],[role=combobox],[role=textbox],[role=searchbox],[role=slider],[contenteditable=""],[contenteditable=true],[onclick],[tabindex]:not([tabindex="-1"])';
  const textual = 'h1,h2,h3,h4,h5,h6,p,li,label,td,th,figcaption,blockquote,pre,img[alt]';
  const selector = filter === 'interactive' ? interactive : interactive + ',' + textual;
  const vw = innerWidth, vh = innerHeight;
  window.__bridgeRef = window.__bridgeRef || 0;
  const clip = (s, n) => { s = (s || '').replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n) + '…' : s; };
  const lines = [`url: ${location.href}`, `title: ${document.title}`, `viewport: ${vw}x${vh} css px, scrolled to ${Math.round(scrollY)} of ${document.documentElement.scrollHeight}`, ''];
  let size = 0;
  for (const el of document.querySelectorAll(selector)) {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || cs.opacity === '0') continue;
    const isInteractive = el.matches(interactive);
    if (!isInteractive && el.querySelector(interactive)) continue;
    const tag = el.tagName.toLowerCase();
    const role = el.getAttribute('role') || (tag === 'input' ? `input:${el.type}` : tag);
    const labelled = el.getAttribute('aria-labelledby');
    const labelText = labelled ? labelled.split(/\s+/).map(id => document.getElementById(id)?.innerText || '').join(' ') : '';
    const forLabel = el.id && el.labels && el.labels[0] ? el.labels[0].innerText : '';
    const name = clip(el.getAttribute('aria-label') || labelText || forLabel || el.getAttribute('placeholder') || el.innerText || el.getAttribute('title') || el.getAttribute('alt') || (tag === 'input' && ['button','submit'].includes(el.type) ? el.value : ''), isInteractive ? 100 : 240);
    if (!isInteractive && !name) continue;
    if (!el.dataset.bridgeRef) el.dataset.bridgeRef = 'r' + (++window.__bridgeRef);
    const x = Math.round(r.left + r.width / 2), y = Math.round(r.top + r.height / 2);
    const off = y < 0 || y > vh || x < 0 || x > vw ? ' offscreen' : '';
    let extra = '';
    if (tag === 'a') extra += ` href=${clip(el.getAttribute('href'), 80)}`;
    if (['input','textarea','select'].includes(tag)) {
      if (el.type === 'checkbox' || el.type === 'radio') extra += el.checked ? ' checked' : ' unchecked';
      else if (el.type === 'password') extra += el.value ? ' value=[hidden]' : '';
      else if (el.value) extra += ` value="${clip(el.value, 80)}"`;
      if (tag === 'select') extra += ` options=[${[...el.options].slice(0, 12).map(o => clip(o.text, 30)).join('|')}]`;
    }
    if (el.disabled) extra += ' disabled';
    const line = `[${el.dataset.bridgeRef}] ${role} "${name}"${extra} @${x},${y}${off}`;
    size += line.length;
    if (size > 40000) { lines.push('… truncated; scroll or use filter=interactive'); break; }
    lines.push(line);
  }
  return lines.join('\n');
}"#;

const GET_TEXT_JS: &str = r#"(() => { const t = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n'); return { url: location.href, title: document.title, text: t.length > 50000 ? t.slice(0, 50000) + '\n… truncated' : t }; })()"#;

/// Scroll a ref into view and return its centre, or null when it is gone.
const REF_POINT_JS: &str = r#"(ref) => {
  const el = document.querySelector(`[data-bridge-ref="${CSS.escape(ref)}"]`);
  if (!el) return null;
  el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}"#;

const FOCUS_JS: &str = r#"(ref) => {
  const el = document.querySelector(`[data-bridge-ref="${CSS.escape(ref)}"]`);
  if (!el) throw new Error(`unknown ref ${ref}; call read_page again`);
  el.scrollIntoView({ block: 'center', behavior: 'instant' }); el.focus();
  return { focused: ref };
}"#;

/// Set a field's value the way a person would, so frameworks see the change.
const FORM_INPUT_JS: &str = r#"({ref, value}) => {
  const el = document.querySelector(`[data-bridge-ref="${CSS.escape(ref)}"]`);
  if (!el) throw new Error(`unknown ref ${ref}; call read_page again`);
  el.scrollIntoView({ block: 'center', behavior: 'instant' }); el.focus();
  if (el.tagName === 'SELECT') {
    const option = [...el.options].find(o => o.value === String(value) || o.text.trim() === String(value));
    if (!option) throw new Error(`no option ${value}`);
    el.value = option.value;
  } else if (el.type === 'checkbox' || el.type === 'radio') {
    el.checked = value === true || value === 'true' || value === 'on' || value === 1;
  } else if (el.isContentEditable) {
    el.textContent = String(value);
  } else {
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, String(value));
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { set: ref };
}"#;

fn peer_pid(stream: &UnixStream) -> Option<u32> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let ok = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&mut pid as *mut libc::pid_t).cast(),
            &mut len,
        )
    };
    (ok == 0 && pid > 0).then_some(pid as u32)
}

fn descendant_of(mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..32 {
        if pid == ancestor {
            return true;
        }
        let output = std::process::Command::new("/bin/ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output();
        let Some(parent) = output
            .ok()
            .and_then(|v| String::from_utf8(v.stdout).ok())
            .and_then(|v| v.trim().parse::<u32>().ok())
        else {
            return false;
        };
        if parent == 0 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn consent_is_immutable_and_stale_answers_fail_closed() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = CloneSupervisor::guarded(dir.path().join("ledger.json"));
        let tool = CloneBrowserTool::new(supervisor, dir.path().to_owned()).unwrap();
        let pid = std::process::id();
        tool.request_capability_context("chat", pid).unwrap();
        let token = tool.request_caps.lock().unwrap()["chat"].token.clone();
        let asked = tool.execute("chat", &token, Some(pid), json!({"kind":"request","domain":"example.test"})).unwrap();
        let id = asked["requestId"].as_str().unwrap();
        assert!(tool.execute("chat", &token, Some(pid), json!({"kind":"request","domain":"other.test"})).is_err());
        assert!(tool.execute("chat", &token, Some(pid), json!({"kind":"request","domain":"example.test","additionalDomains":["cdn.other.test"]})).is_err());
        assert_eq!(tool.pending_request("chat").as_deref(), Some("example.test"));
        assert!(tool.take_pending_request("chat", "old-request", pid).is_err());
        assert!(tool.take_pending_request("chat", id, pid + 1).is_err());
        assert!(tool.pending_details("chat").is_some());
        tool.clear_pending_request("chat");
        let answer = tool.execute("chat", &token, Some(pid), json!({"kind":"request_status","requestId":id})).unwrap();
        assert_eq!(answer["awaiting"], false);
        assert_eq!(answer["ok"], false);
        assert!(tool.execute("chat", &token, Some(pid), json!({"kind":"request_status","requestId":"wrong"})).is_err());
    }

    #[test]
    fn dropping_the_tool_releases_its_listener_and_supervisor() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = CloneSupervisor::guarded(dir.path().join("ledger.json"));
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().to_owned()).unwrap();
        let socket = tool.socket.clone();
        tool.request_capability_context("s1", std::process::id()).unwrap();
        let weak = Arc::downgrade(&tool);
        drop(tool);
        assert!(weak.upgrade().is_none(), "listener retained its owner");
        assert!(!socket.exists());
        assert!(!dir.path().join("clone-request-s1").exists());
        assert_eq!(Arc::strong_count(&supervisor), 1);
    }

    #[test]
    fn destroy_revokes_a_request_even_before_a_browser_exists() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = CloneSupervisor::guarded(dir.path().join("ledger.json"));
        let tool = CloneBrowserTool::new(Arc::clone(&supervisor), dir.path().to_owned()).unwrap();
        let pid = std::process::id();
        tool.request_capability_context("s1", pid).unwrap();
        let token = tool.request_caps.lock().unwrap()["s1"].token.clone();
        tool.execute("s1", &token, Some(pid), json!({"kind":"request","domain":"example.test"})).unwrap();
        let orchestrator = crate::clone_orchestrator::CloneOrchestrator::new(supervisor, Arc::clone(&tool));
        orchestrator.destroy("s1");
        assert!(tool.pending_request("s1").is_none());
        assert!(tool.execute("s1", &token, Some(pid), json!({"kind":"request","domain":"example.test"})).is_err());
    }
    #[test]
    fn unknown_kinds_are_refused_and_aliases_resolve() {
        for kind in ["cookie", "storage", "Runtime.evaluate", ""] {
            assert!(canonical_kind(kind).is_none(), "{kind}");
        }
        assert_eq!(canonical_kind("inspect"), Some("read_page"));
        assert_eq!(canonical_kind("left_click"), Some("click"));
        assert_eq!(canonical_kind("javascript"), Some("evaluate"));
    }

    #[test]
    fn every_action_is_gated_and_every_look_is_not() {
        for kind in ["click", "double_click", "triple_click", "right_click", "hover", "drag", "type", "key", "scroll", "navigate", "form_input", "focus", "evaluate"] {
            assert!(MUTATING_KINDS.contains(&kind), "{kind} not gated");
            assert_eq!(canonical_kind(kind), Some(kind));
        }
        for kind in ["screenshot", "read_page", "get_text", "wait"] {
            assert!(!MUTATING_KINDS.contains(&kind), "{kind} gated");
        }
    }

    #[test]
    fn vision_is_on_until_the_person_turns_it_off() {
        let dir = tempfile::tempdir_in("/tmp").unwrap();
        let supervisor = CloneSupervisor::guarded(dir.path().join("ledger.json"));
        let tool = CloneBrowserTool::new(supervisor, dir.path().join("t")).unwrap();
        assert!(tool.vision("s1"));
        tool.set_vision("s1", false);
        assert!(!tool.vision("s1"));
        assert!(tool.vision("s2"));
        tool.revoke_browser("s1");
        assert!(tool.vision("s1"));
    }

    #[test]
    fn navigation_takes_only_plain_web_urls() {
        assert_eq!(navigable_host("https://App.Example.com/x").unwrap(), "app.example.com");
        for url in ["file:///etc/passwd", "javascript:alert(1)", "https://user:pw@example.com/", "not a url"] {
            assert!(navigable_host(url).is_err(), "{url}");
        }
    }

    #[test]
    fn keys_and_chords_map_to_cdp_events() {
        let (enter, commands) = key_event("Enter").unwrap();
        assert_eq!(enter["windowsVirtualKeyCode"], 13);
        assert_eq!(enter["text"], "\r");
        assert!(commands.is_empty());
        let (select_all, commands) = key_event("cmd+a").unwrap();
        assert_eq!(select_all["modifiers"], 4);
        assert!(select_all.get("text").is_none());
        assert_eq!(commands, vec!["selectAll"]);
        let (shift_tab, _) = key_event("shift+Tab").unwrap();
        assert_eq!(shift_tab["modifiers"], 8);
        let (letter, _) = key_event("q").unwrap();
        assert_eq!(letter["text"], "q");
        assert_eq!(letter["code"], "KeyQ");
        assert!(key_event("hyper+q").is_err());
        assert!(key_event("NotAKey").is_err());
    }

    #[test]
    fn scrubs_registered_values() {
        let mut guard = crate::browser_clone_guard::GuardState::new();
        guard.add_secret("example.com", "long-secret-value");
        let mut reply = json!({"content":"long-secret-value", "nested":["long-secret-value"]});
        guard.scrub_response(&mut reply);
        assert!(!reply.to_string().contains("long-secret-value"));
    }
}
