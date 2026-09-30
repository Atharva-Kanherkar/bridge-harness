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

/// Kinds that act on the page. The person must approve them. Takeover pauses
/// all page access, including reads. Images remain in the person's dock.
const MUTATING_KINDS: [&str; 5] = ["click", "type", "scroll", "navigate", "focus"];

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
        Some(format!("Bridge clone browser tool: {}. Use one JSON argument. Kinds: status (poll while paused), inspect, click(x,y), type(text), scroll(x,y), navigate(url), focus(nodeId), result(commandId). Approved domain: {domain}. Page images stay in the person’s dock. The browser is destroyed when your turn ends.", path.display()))
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
        Some(format!("To test in a signed-in browser, ask for a throwaway clone (the person approves it): {tool} '{{\"kind\":\"request\",\"domain\":\"example.com\"}}'. The result includes requestId. Poll this same tool with kind=request_status and requestId until awaiting=false; it returns the approved browser tool instructions in this turn. Approval may need several minutes, so keep polling every two seconds without ending your turn. Optional additionalDomains lists CDN or sign-in domains needed by the site; those are shown for explicit approval. Optional extensionPath must be an absolute directory for the extension under test and is shown for approval. Use the returned browser tool, then finish your turn to destroy the browser. Do not ask unless the task needs a signed-in site.", tool = path.display()))
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
        let safe: String = session.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let _ = fs::remove_file(self.directory.join(format!("clone-browser-{safe}")));
    }

    /// Approve page actions for a session's clone: mutating kinds are allowed
    /// from now on.
    pub fn allow_mutations(&self, session: &str) {
        if let Ok(mut set) = self.mutable.lock() {
            set.insert(session.to_owned());
        }
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
                let mut blocked = json!(guard.blocked().iter().map(|request| request.host.clone()).collect::<Vec<_>>());
                guard.scrub_response(&mut blocked);
                blocked
            }).unwrap_or_else(|| json!([]));
            return Ok(json!({"ok":true,"paused":paused,"blockedHosts":blocked}));
        }
        if self.paused.lock().map_err(|_| "clone unavailable")?.contains(session) {
            return Err("the person controls the browser; all agent access is paused".into());
        }
        if kind == "screenshot" {
            return Err("browser images stay in the person's dock; use inspect for scrubbed page text".into());
        }
        let guard = self
            .supervisor
            .clone_guard(&clone_id)
            .ok_or("clone unavailable")?;
        if request.get("kind").and_then(Value::as_str) == Some("result") {
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
        let kind = request.get("kind").and_then(Value::as_str).unwrap_or("");
        let approved = self.mutable.lock().map(|set| set.contains(session)).unwrap_or(false);
        if MUTATING_KINDS.contains(&kind) && !approved {
            return Err("page actions are not approved for this clone".into());
        }
        let (method, params) = command(&request, &domain)?;
        let mut result = self
            .supervisor
            .page_call(&clone_id, method, params)
            .map_err(|_| "browser command failed".to_owned())?;
        if request.get("kind").and_then(Value::as_str) == Some("click") {
            let release = json!({"type":"mouseReleased","x":request["x"],"y":request["y"],"button":"left","clickCount":1});
            self.supervisor
                .page_call(&clone_id, "Input.dispatchMouseEvent", release)
                .map_err(|_| "browser command failed".to_owned())?;
        }
        if kind == "inspect" { redact_editable_values(&mut result); }
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

fn redact_editable_values(result: &mut Value) {
    if let Some(nodes) = result.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            let editable = matches!(node.pointer("/role/value").and_then(Value::as_str), Some("textField" | "textFieldWithComboBox" | "comboBox"));
            if editable && node.get("value").is_some() { node["value"] = json!({"type":"string","value":"[redacted]"}); }
        }
    }
}

fn normalized_domain(domain: &str) -> Option<String> {
    let d = domain.trim().to_ascii_lowercase();
    (d.len() <= 253 && d.contains('.') && d.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })).then_some(d)
}

fn command(request: &Value, domain: &str) -> Result<(&'static str, Value), String> {
    let kind = request
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("missing kind")?;
    let number = |key: &str| {
        request
            .get(key)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("missing {key}"))
    };
    match kind {
        "inspect" => Ok(("Accessibility.getFullAXTree", json!({}))),
        "screenshot" => Err("browser images remain local to the person".into()),
        "click" => Ok((
            "Input.dispatchMouseEvent",
            json!({"type":"mousePressed","x":number("x")?,"y":number("y")?,"button":"left","clickCount":1}),
        )),
        "type" => Ok((
            "Input.insertText",
            json!({"text":request.get("text").and_then(Value::as_str).ok_or("missing text")?}),
        )),
        "scroll" => Ok((
            "Input.dispatchMouseEvent",
            json!({"type":"mouseWheel","x":number("x")?,"y":number("y")?,"deltaX":0,"deltaY":number("deltaY")?}),
        )),
        "focus" => Ok((
            "DOM.focus",
            json!({"nodeId":request.get("nodeId").and_then(Value::as_i64).ok_or("missing nodeId")?}),
        )),
        "navigate" => {
            let url = reqwest::Url::parse(
                request
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or("missing url")?,
            )
            .map_err(|_| "invalid URL")?;
            let host = url.host_str().ok_or("invalid host")?.to_ascii_lowercase();
            if !matches!(url.scheme(), "http" | "https")
                || !(host == domain || host.ends_with(&format!(".{domain}")))
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err("navigation outside approved domain".into());
            }
            Ok(("Page.navigate", json!({"url":url.as_str()})))
        }
        "result" => Err("result is available only for queued commands".into()),
        _ => Err("browser command kind is not allowed".into()),
    }
}

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
    fn refuses_out_of_contract_kinds() {
        for kind in ["cookie", "storage", "eval", "Runtime.evaluate"] {
            assert!(command(&json!({"kind":kind}), "example.com").is_err());
        }
    }
    #[test]
    fn mutating_kinds_are_real_commands_gated_by_policy_not_validation() {
        // Each mutating kind is a valid command; the gate in `execute`, not
        // `command`, is what withholds it until approval exists.
        for (kind, extra) in [
            ("click", json!({"x":1.0,"y":2.0})),
            ("type", json!({"text":"hi"})),
            ("scroll", json!({"x":1.0,"y":2.0,"deltaY":3.0})),
            ("focus", json!({"nodeId":5})),
            ("navigate", json!({"url":"https://example.com/"})),
        ] {
            assert!(MUTATING_KINDS.contains(&kind), "{kind} not gated");
            let mut request = json!({"kind": kind});
            request
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            assert!(
                command(&request, "example.com").is_ok(),
                "{kind} not a valid command"
            );
        }
        // Read kinds are never gated.
        for kind in ["inspect", "screenshot"] {
            assert!(!MUTATING_KINDS.contains(&kind));
        }
    }

    #[test]
    fn navigate_refuses_a_foreign_domain() {
        assert!(command(
            &json!({"kind":"navigate","url":"https://evil.test/"}),
            "example.com"
        )
        .is_err());
        assert!(command(
            &json!({"kind":"navigate","url":"https://example.com.evil.test/"}),
            "example.com"
        )
        .is_err());
        assert!(command(
            &json!({"kind":"navigate","url":"https://app.example.com/x"}),
            "example.com"
        )
        .is_ok());
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
