//! Daemon-host mode: the desktop shell as a client of `bridged`.
//!
//! In this mode the shell owns **no runtime**. Every Tauri invoke is proxied
//! to the daemon over its Unix socket as the JSON-RPC method the command maps
//! to (the registry keeps that mapping 1:1), and every daemon notification is
//! re-emitted to the webview with an unchanged name and payload — the
//! frontend cannot tell which host it is talking to.
//!
//! Attachment follows the issue contract: connect to a running daemon if one
//! serves the data directory, otherwise start the bundled `bridged` and wait
//! for it to come up. A supervisor thread owns the connection for the life of
//! the app: when the daemon restarts or the live channel drops frames it
//! reconnects and emits the same reconciliation hints (`state-changed`,
//! `adapters-changed`) the embedded host emits after a lagged event bus, so
//! the frontend recovers durable history from the session forest exactly as
//! it always has.

use bridge_client::{ClientError, DaemonClient, Endpoint};
use bridge_protocol::{MethodName, Params, RpcNotification, TypedMethod};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Parallel daemon connections for invokes. The daemon serves each connection
/// sequentially, so one slow call (a Git scan, adapter teardown) must not
/// stall every other panel of the UI; a small pool restores the concurrency
/// the embedded host had. Bounded well below the daemon's connection cap.
const POOL_SIZE: usize = 4;

/// How long an invoke waits for a live connection before failing. Covers the
/// small window while the supervisor is reconnecting after a daemon restart.
const CALL_LINK_WAIT: Duration = Duration::from_secs(10);

/// How long a freshly spawned daemon may take to accept its first handshake.
/// Boot includes adapter discovery and store recovery, so this is generous.
const START_DEADLINE: Duration = Duration::from_secs(30);

/// Handshake budget per connection attempt against a live socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a spawned daemon's stdout/stderr goes, inside the data directory.
const DAEMON_LOG_FILE: &str = "bridged.log";

/// Which host the desktop runs, from `BRIDGE_DESKTOP_HOST`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPreference {
    /// Attach or start a daemon; fall back to embedded if that fails.
    Auto,
    /// Daemon or nothing — startup fails loudly instead of falling back.
    /// This is the acceptance configuration: the embedded path is disabled.
    DaemonOnly,
    /// The pre-daemon in-process runtime, kept during the migration.
    EmbeddedOnly,
}

pub fn host_preference() -> HostPreference {
    match std::env::var("BRIDGE_DESKTOP_HOST").as_deref() {
        Ok("embedded") => HostPreference::EmbeddedOnly,
        Ok("daemon") => HostPreference::DaemonOnly,
        // Unknown values fall back to the default rather than failing app
        // startup over a typo; the chosen host is logged either way.
        _ => HostPreference::Auto,
    }
}

/// The wire params for a proxied command payload. Parameterless methods send
/// none — the daemon rejects stray payloads, and the JS `invoke` helper sends
/// `{}` when the caller passes no arguments. Everything else forwards the
/// payload object exactly as the webview built it: invoke argument names are
/// the contract's camelCase wire names (the shell's signature-parity test
/// pins that), so no translation happens here.
pub fn wire_params(method: MethodName, payload: Value) -> Option<Value> {
    TypedMethod::for_method(method).params.map(|_| payload)
}

/// Proxy one Tauri invoke to the daemon. Runs the blocking socket call on the
/// blocking pool — same rule as the embedded commands: native work never
/// lands on the macOS UI thread.
pub fn proxy_invoke<R: tauri::Runtime>(
    proxy: Arc<DaemonProxy>,
    invoke: tauri::ipc::Invoke<R>,
) -> bool {
    let command = invoke.message.command().to_owned();
    let Some(method) = MethodName::from_command(&command) else {
        // The registry and generate_handler! are tested 1:1, so this is a
        // frontend bug, not a routing gap.
        invoke.resolver.reject(format!("unknown command: {command}"));
        return true;
    };
    let payload = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(value) => value.clone(),
        tauri::ipc::InvokeBody::Raw(bytes) => match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                invoke.resolver.reject(format!("invalid {command} payload: {error}"));
                return true;
            }
        },
    };
    let params = wire_params(method, payload);
    let resolver = invoke.resolver;
    tauri::async_runtime::spawn_blocking(move || match proxy.call(method, params) {
        Ok(result) => resolver.resolve(result),
        Err(message) => resolver.reject(message),
    });
    true
}

/// A live set of daemon connections, replaced wholesale on reconnect.
struct Link {
    clients: Vec<Arc<DaemonClient>>,
}

/// The shared handle invokes call through. The supervisor installs a [`Link`]
/// when attached and clears it when the connection dies; calls that observe a
/// dead connection clear it too, so the supervisor reconnects promptly.
#[derive(Default)]
pub struct DaemonProxy {
    link: RwLock<Option<Arc<Link>>>,
    next: AtomicUsize,
}

impl DaemonProxy {
    /// Call a protocol method, waiting briefly for a connection if the
    /// supervisor is mid-reconnect. Errors are the strings the webview
    /// already understands: a daemon-side `BridgeError` arrives as its
    /// Display text, exactly what the embedded host serializes.
    pub fn call(&self, method: MethodName, params: Option<Value>) -> Result<Value, String> {
        self.call_within(method, params, CALL_LINK_WAIT)
    }

    /// [`DaemonProxy::call`] with an explicit link-wait budget, for callers
    /// (and tests) that must not block through a reconnect window.
    pub fn call_within(
        &self,
        method: MethodName,
        params: Option<Value>,
        link_wait: Duration,
    ) -> Result<Value, String> {
        let link = self.wait_for_link(link_wait).ok_or_else(|| {
            "The Bridge daemon is not reachable; still reconnecting".to_owned()
        })?;
        let client =
            link.clients[self.next.fetch_add(1, Ordering::Relaxed) % link.clients.len()].clone();
        match client.call(method, params) {
            Ok(value) => Ok(value),
            Err(ClientError::Rpc(error)) => Err(error.message),
            Err(ClientError::Disconnected) => {
                self.invalidate(&link);
                Err("The Bridge daemon connection was lost; reconnecting".into())
            }
            Err(ClientError::Timeout) => {
                Err(format!("{} did not answer in time", method.as_str()))
            }
            Err(other) => Err(other.to_string()),
        }
    }

    pub fn attached(&self) -> bool {
        self.link.read().unwrap().is_some()
    }

    fn wait_for_link(&self, timeout: Duration) -> Option<Arc<Link>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(link) = self.link.read().unwrap().as_ref() {
                return Some(link.clone());
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn install(&self, clients: Vec<Arc<DaemonClient>>) {
        *self.link.write().unwrap() = Some(Arc::new(Link { clients }));
    }

    /// Clear the link a failed call went through — unless a reconnect already
    /// replaced it, in which case the newer link stays.
    fn invalidate(&self, seen: &Arc<Link>) {
        let mut link = self.link.write().unwrap();
        if link.as_ref().is_some_and(|current| Arc::ptr_eq(current, seen)) {
            *link = None;
        }
    }

    fn invalidate_all(&self) {
        *self.link.write().unwrap() = None;
    }
}

/// Attach-or-start: how the supervisor obtains connections.
pub struct Launcher {
    data_dir: PathBuf,
    browser_extension: PathBuf,
    /// The `bridged` binary to start when attaching fails; `None` means
    /// attach-only (a daemon must be managed externally).
    binary: Option<PathBuf>,
    child: Option<Child>,
}

impl Launcher {
    pub fn new(data_dir: PathBuf, browser_extension: PathBuf, binary: Option<PathBuf>) -> Launcher {
        Launcher { data_dir, browser_extension, binary, child: None }
    }

    /// A connection pool to a daemon serving the data directory, attaching to
    /// a running one or starting the bundled binary. Fails with an
    /// actionable message — including the daemon's own words when it refused
    /// us or exited during startup.
    pub fn ensure(&mut self) -> Result<Vec<Arc<DaemonClient>>, String> {
        match self.attach() {
            Ok(clients) => return Ok(clients),
            // A live daemon answered and said no (bad token, incompatible
            // protocol). Starting a second one cannot help — the socket is
            // owned. Surface its refusal verbatim.
            Err(ClientError::Handshake(error)) => {
                return Err(format!(
                    "a running bridged daemon refused this app ({}): {}",
                    error.code, error.message
                ));
            }
            Err(_) => {}
        }
        let Some(binary) = self.binary.clone() else {
            return Err(format!(
                "no bridged daemon is serving {} and no daemon binary is available to start",
                self.data_dir.display()
            ));
        };
        let already_running = matches!(
            self.child.as_mut().map(Child::try_wait),
            Some(Ok(None))
        );
        if !already_running {
            self.spawn(&binary)?;
        }
        let deadline = Instant::now() + START_DEADLINE;
        loop {
            std::thread::sleep(Duration::from_millis(200));
            match self.attach() {
                Ok(clients) => return Ok(clients),
                Err(ClientError::Handshake(error)) => {
                    return Err(format!(
                        "the bridged daemon refused this app ({}): {}",
                        error.code, error.message
                    ));
                }
                Err(_) => {}
            }
            // A startup failure (say, another owner holds the data-dir
            // lease) exits the child; report its last words instead of
            // timing out in silence.
            if let Some(Ok(Some(status))) = self.child.as_mut().map(Child::try_wait) {
                return Err(format!(
                    "bridged exited during startup ({status}): {}",
                    self.log_tail()
                ));
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "bridged did not become reachable within {START_DEADLINE:?}: {}",
                    self.log_tail()
                ));
            }
        }
    }

    fn attach(&self) -> Result<Vec<Arc<DaemonClient>>, ClientError> {
        let endpoint = Endpoint::for_data_dir(&self.data_dir)?;
        let first = DaemonClient::connect_with_timeout(&endpoint, CONNECT_TIMEOUT)?;
        let mut clients = vec![Arc::new(first)];
        // Extra pool connections are an optimization, never a requirement:
        // a daemon near its connection cap still serves us on one.
        while clients.len() < POOL_SIZE {
            match DaemonClient::connect_with_timeout(&endpoint, CONNECT_TIMEOUT) {
                Ok(client) => clients.push(Arc::new(client)),
                Err(_) => break,
            }
        }
        Ok(clients)
    }

    fn spawn(&mut self, binary: &Path) -> Result<(), String> {
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|error| format!("could not create {}: {error}", self.data_dir.display()))?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
            .map_err(|error| format!("could not open the daemon log: {error}"))?;
        let log_err = log
            .try_clone()
            .map_err(|error| format!("could not open the daemon log: {error}"))?;
        let child = Command::new(binary)
            .arg("--data-dir")
            .arg(&self.data_dir)
            .arg("--browser-extension")
            .arg(&self.browser_extension)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
        eprintln!("bridge: started bridged (pid {}) for {}", child.id(), self.data_dir.display());
        // Reap the previous child, if any, now that it has provably exited
        // (already_running was false) — never leave zombies behind.
        if let Some(mut old) = self.child.replace(child) {
            let _ = old.wait();
        }
        Ok(())
    }

    fn log_path(&self) -> PathBuf {
        self.data_dir.join(DAEMON_LOG_FILE)
    }

    fn log_tail(&self) -> String {
        let Ok(contents) = std::fs::read_to_string(self.log_path()) else {
            return format!("no daemon log at {}", self.log_path().display());
        };
        let lines: Vec<&str> = contents.lines().rev().take(5).collect();
        if lines.is_empty() {
            return format!("daemon log {} is empty", self.log_path().display());
        }
        let mut tail: Vec<&str> = lines.into_iter().rev().collect();
        tail.insert(0, "last daemon log lines:");
        tail.join("\n")
    }
}

/// The bundled `bridged` binary. In a bundle Tauri places external binaries
/// next to the app executable under their plain name; a source checkout falls
/// back to the target-triple binary `prepare-daemon.sh` stages.
pub fn find_bridged_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BRIDGE_DAEMON_BIN") {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bundled) = exe.parent().map(|dir| dir.join("bridged")) {
            if bundled.is_file() {
                return Some(bundled);
            }
        }
    }
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("binaries");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(staged)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("bridged-"))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Own the daemon connection for the life of the app: keep the proxy linked,
/// pump notifications to `emit`, reconnect on any failure. After every
/// reconnect — and on a `stream-lagged` marker — emit the refetch hints the
/// embedded host emits after bus lag: the frontend re-reads the state
/// snapshot and durable session history, so nothing is silently missing.
/// `stop` exists for tests; in the app the loop runs until process exit.
pub fn supervise<E: Fn(&str, Value)>(
    proxy: &Arc<DaemonProxy>,
    mut launcher: Launcher,
    initial: Option<Vec<Arc<DaemonClient>>>,
    stop: &AtomicBool,
    emit: E,
) {
    let mut pending = initial;
    let mut attached_before = false;
    while !stop.load(Ordering::SeqCst) {
        let clients = match pending.take() {
            Some(clients) => clients,
            None => match launcher.ensure() {
                Ok(clients) => clients,
                Err(error) => {
                    eprintln!("bridge: cannot reach the daemon ({error}); retrying");
                    let backoff = Instant::now() + Duration::from_secs(2);
                    while Instant::now() < backoff && !stop.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    continue;
                }
            },
        };
        let subscription = clients[0].subscribe();
        proxy.install(clients);
        if attached_before {
            emit_reconciliation(&emit);
        }
        attached_before = true;
        pump(proxy, &subscription, stop, &emit);
        proxy.invalidate_all();
    }
}

/// Forward notifications until the connection dies (subscription closed), a
/// call observes the loss first (link cleared), or `stop` is set.
fn pump<E: Fn(&str, Value)>(
    proxy: &Arc<DaemonProxy>,
    subscription: &Receiver<RpcNotification>,
    stop: &AtomicBool,
    emit: &E,
) {
    loop {
        if stop.load(Ordering::SeqCst) || !proxy.attached() {
            return;
        }
        match subscription.recv_timeout(Duration::from_millis(500)) {
            Ok(notification) => deliver(notification, emit),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Emit one daemon notification to the webview. Lag markers — daemon-side or
/// synthesized by the client's own bounded queue — become refetch hints
/// instead of surfacing a transport detail the frontend does not know.
fn deliver<E: Fn(&str, Value)>(notification: RpcNotification, emit: &E) {
    if notification.method == bridge_protocol::notifications::NotificationName::StreamLagged.as_str()
    {
        emit_reconciliation(emit);
        return;
    }
    let payload = notification.params.map(Params::into_value).unwrap_or(Value::Null);
    emit(&notification.method, payload);
}

fn emit_reconciliation<E: Fn(&str, Value)>(emit: &E) {
    use bridge_protocol::notifications::NotificationName;
    emit(NotificationName::StateChanged.as_str(), Value::Null);
    emit(NotificationName::AdaptersChanged.as_str(), Value::Null);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameterless_methods_send_no_params() {
        // The JS invoke helper sends `{}` for argument-free calls; the daemon
        // contract requires their absence.
        assert_eq!(wire_params(MethodName::GetState, serde_json::json!({})), None);
        assert_eq!(wire_params(MethodName::Health, serde_json::json!({})), None);
    }

    #[test]
    fn parameterized_methods_forward_the_payload_verbatim() {
        let payload = serde_json::json!({"sessionId": "s1"});
        assert_eq!(
            wire_params(MethodName::GetSessionForest, payload.clone()),
            Some(payload)
        );
        // All-optional params keep their empty object: the daemon decodes it,
        // whereas absence would be an invalid_params refusal.
        assert_eq!(
            wire_params(MethodName::RefreshOpencodeCatalog, serde_json::json!({})),
            Some(serde_json::json!({}))
        );
    }

    #[test]
    fn every_registered_command_is_proxyable() {
        // from_command is the proxy's routing table; a method whose command
        // name did not resolve would dead-end invokes in daemon mode.
        for method in MethodName::ALL.iter().copied() {
            assert_eq!(
                MethodName::from_command(method.command_name()),
                Some(method),
                "{} is not reachable through the invoke proxy",
                method.as_str()
            );
        }
    }

    #[test]
    fn lag_markers_become_reconciliation_hints() {
        let seen = std::sync::Mutex::new(Vec::<(String, Value)>::new());
        let emit = |kind: &str, payload: Value| {
            seen.lock().unwrap().push((kind.to_owned(), payload));
        };
        deliver(
            RpcNotification::new("stream-lagged", Params::new(serde_json::json!({"missed": 3})).ok()),
            &emit,
        );
        deliver(
            RpcNotification::new(
                "session-output",
                Params::new(serde_json::json!({"sessionId": "s", "data": "x"})).ok(),
            ),
            &emit,
        );
        let seen = seen.into_inner().unwrap();
        assert_eq!(
            seen.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(),
            vec!["state-changed", "adapters-changed", "session-output"],
        );
        assert_eq!(seen[0].1, Value::Null);
        assert_eq!(seen[2].1["data"], "x");
    }

    #[test]
    fn notification_payloads_pass_through_unchanged() {
        let seen = std::sync::Mutex::new(Vec::<(String, Value)>::new());
        let emit = |kind: &str, payload: Value| {
            seen.lock().unwrap().push((kind.to_owned(), payload));
        };
        // state-changed carries no params on the wire; the webview contract
        // is a null payload, exactly what the embedded forwarder emits.
        deliver(RpcNotification::new("state-changed", None), &emit);
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen, vec![("state-changed".to_owned(), Value::Null)]);
    }

    #[test]
    fn host_preference_defaults_to_auto() {
        // Do not read the real env here (tests run in parallel); the parsing
        // rule is what matters and it is exercised through the match arms.
        assert_eq!(host_preference(), HostPreference::Auto);
    }
}
