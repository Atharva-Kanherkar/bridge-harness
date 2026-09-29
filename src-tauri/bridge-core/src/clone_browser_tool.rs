//! Narrow agent command surface for one guarded browser clone.

use crate::browser_clone::CloneSupervisor;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
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

/// Kinds that act on the page rather than read it. Until the approval card
/// (a later part) exists, these are refused: the epic's rule is that an agent
/// click, keystroke, or navigation waits for the user. Read kinds (inspect,
/// screenshot, result) never mutate and are always allowed.
const MUTATING_KINDS: [&str; 5] = ["click", "type", "scroll", "navigate", "focus"];

pub struct CloneBrowserTool {
    supervisor: Arc<CloneSupervisor>,
    socket: PathBuf,
    directory: PathBuf,
    capabilities: Mutex<HashMap<String, Capability>>,
    results: Mutex<HashMap<String, (String, String, Value)>>,
    /// Whether page-mutating kinds are permitted. Off until an approval path
    /// gates them; see [`MUTATING_KINDS`].
    allow_mutation: bool,
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
            // Read-only until an approval path can gate page mutations.
            allow_mutation: false,
        });
        let server = Arc::clone(&tool);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let server = Arc::clone(&server);
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
        Some(format!("Bridge clone browser tool: {}. Use one JSON argument. Kinds: inspect, screenshot, click(x,y), type(text), scroll(x,y), navigate(url), focus(nodeId), result(commandId). Approved domain: {domain}.", path.display()))
    }

    /// The unix socket the tool script talks to. The orchestrator's end-to-end
    /// test connects here directly to exercise the agent-facing path.
    #[cfg(test)]
    pub(crate) fn socket_path(&self) -> &std::path::Path {
        &self.socket
    }

    pub fn revoke_session(&self, session: &str) {
        self.capabilities.lock().unwrap().remove(session);
        let safe: String = session
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let _ = fs::remove_file(self.directory.join(format!("clone-browser-{safe}")));
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
        if MUTATING_KINDS.contains(&kind) && !self.allow_mutation {
            return Err("page actions await user approval and are not yet available".into());
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
        guard
            .lock()
            .map_err(|_| "clone guard unavailable")?
            .scrub_response(&mut result);
        let id = Uuid::new_v4().to_string();
        let mut results = self.results.lock().map_err(|_| "result unavailable")?;
        if results.len() >= 128 {
            results.clear();
        }
        results.insert(id.clone(), (session.to_owned(), clone_id, result.clone()));
        Ok(json!({"ok":true,"commandId":id,"result":result}))
    }
}

fn normalized_domain(domain: &str) -> Option<String> {
    let d = domain.trim().to_ascii_lowercase();
    (d.contains('.')
        && d.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    .then_some(d)
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
        "screenshot" => Ok(("Page.captureScreenshot", json!({"format":"png"}))),
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
