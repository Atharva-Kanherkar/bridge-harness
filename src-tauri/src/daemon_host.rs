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

use bridge_client::{ClientError, DaemonClient, Endpoint, NotificationSubscription};
use bridge_protocol::{ErrorCode, MethodName, Params, RpcNotification, TypedMethod};
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Parallel daemon connections for invokes. The daemon serves each connection
/// sequentially, so one slow call (a Git scan, adapter teardown) must not
/// stall every other panel of the UI; a small pool restores the concurrency
/// the embedded host had. Bounded well below the daemon's connection cap.
///
/// The pool is partitioned: `github` domain methods shell out to `gh` and can
/// take ten-plus seconds against large repositories, so they get their own
/// lanes. Blind sharing let one slow GitHub read queue sessions, health, and
/// work-board invokes behind it — the whole app appeared to hang.
const GENERAL_LANES: usize = 4;
const GITHUB_LANES: usize = 2;
const POOL_SIZE: usize = GENERAL_LANES + GITHUB_LANES;

/// Bound the blocking-runtime work retained by an invoke burst. A short queue
/// absorbs normal UI fan-out; requests beyond it fail promptly instead of
/// retaining arbitrary payloads and blocking threads for minutes.
const MAX_INVOKE_JOBS: usize = POOL_SIZE * 4;

/// How long an invoke waits for a live connection before failing. Covers the
/// small window while the supervisor is reconnecting after a daemon restart.
const CALL_LINK_WAIT: Duration = Duration::from_secs(10);

/// How long a freshly spawned daemon may take to accept its first handshake.
/// Boot includes adapter discovery and store recovery, so this is generous.
const START_DEADLINE: Duration = Duration::from_secs(30);

/// Handshake budget per connection attempt against a live socket.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Graceful process-group shutdown budget before a forced kill. This exceeds
/// bridged's own drain deadline so adapter settlement and socket cleanup get
/// time to finish after the accept loop observes SIGTERM.
const CHILD_SHUTDOWN_DEADLINE: Duration = Duration::from_secs(10);

/// Where a spawned daemon's stdout/stderr goes, inside the data directory.
const DAEMON_LOG_FILE: &str = "bridged.log";
const MAX_DAEMON_LOG_BYTES: u64 = 4 * 1024 * 1024;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPreferenceError(String);

impl std::fmt::Display for HostPreferenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "invalid BRIDGE_DESKTOP_HOST value {:?}; expected auto, daemon, or embedded",
            self.0
        )
    }
}

impl std::error::Error for HostPreferenceError {}

fn parse_host_preference(value: Option<&str>) -> Result<HostPreference, HostPreferenceError> {
    match value {
        None | Some("auto") => Ok(HostPreference::Auto),
        Some("embedded") => Ok(HostPreference::EmbeddedOnly),
        Some("daemon") => Ok(HostPreference::DaemonOnly),
        Some(other) => Err(HostPreferenceError(other.to_owned())),
    }
}

pub fn host_preference() -> Result<HostPreference, HostPreferenceError> {
    parse_host_preference(std::env::var("BRIDGE_DESKTOP_HOST").ok().as_deref())
}

/// Resolve the desktop's data directory before selecting either host. An
/// explicit directory lets development and release smoke tests use their own
/// state; requiring an absolute path avoids differences in Finder/shell cwd.
pub fn desktop_data_dir(
    default: PathBuf,
    override_path: Option<std::ffi::OsString>,
) -> Result<PathBuf, String> {
    match override_path {
        None => Ok(default),
        Some(path) => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err("BRIDGE_DATA_DIR must be a nonempty absolute path".into());
            }
            Ok(path)
        }
    }
}

/// Atomic desktop exclusion, independent of the backend's owner.lock. The
/// single-instance plugin forwards focus requests, but its socket listener
/// starts asynchronously and can fail open when two copies launch together.
/// Keep this lease in application state before either desktop host starts.
pub struct DesktopLease {
    _file: std::fs::File,
}

impl DesktopLease {
    pub fn acquire(data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|error| format!("Could not prepare the Bridge data directory: {error}"))?;
        let lock_path = data_dir.join("desktop.lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&lock_path)
            .map_err(|error| {
                format!(
                    "Could not open the Bridge desktop lock {}: {error}",
                    lock_path.display()
                )
            })?;
        if !file
            .metadata()
            .map_err(|error| format!("Could not inspect the Bridge desktop lock: {error}"))?
            .is_file()
        {
            return Err("The Bridge desktop lock is not a regular file".into());
        }
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "Bridge is already open for {}. Switch to that window, or quit it before opening another copy.",
                    data_dir.display()
                ));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(format!(
                    "Could not acquire the Bridge desktop lock: {error}"
                ));
            }
        }
        // Change permissions only after owning the lock, through the open
        // handle. A losing copy must not mutate the live owner's lock file.
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not secure the Bridge desktop lock: {error}"))?;
        Ok(Self { _file: file })
    }
}

impl Drop for DesktopLease {
    fn drop(&mut self) {
        // Release explicitly before close. A concurrent fork/exec can briefly
        // inherit this descriptor despite CLOEXEC, retaining its flock after
        // our close until the child's exec. Releasing the shared lock avoids
        // a false conflict during an immediate desktop restart.
        let _ = self._file.unlock();
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
        invoke
            .resolver
            .reject(format!("unknown command: {command}"));
        return true;
    };
    let payload = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(value) => value.clone(),
        tauri::ipc::InvokeBody::Raw(bytes) => match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(error) => {
                invoke
                    .resolver
                    .reject(format!("invalid {command} payload: {error}"));
                return true;
            }
        },
    };
    let params = wire_params(method, payload);
    let resolver = invoke.resolver;
    let Some(permit) = proxy.reserve_invoke() else {
        resolver.reject("The Bridge daemon is busy; try again");
        return true;
    };
    tauri::async_runtime::spawn_blocking(move || {
        let result = proxy.call(method, params);
        drop(permit);
        match result {
            Ok(value) => resolver.resolve(value),
            Err(message) => resolver.reject(message),
        }
    });
    true
}

/// A live set of daemon connections, replaced wholesale on reconnect.
struct Link {
    clients: Vec<Arc<DaemonClient>>,
}

/// The connection indexes `method` may use. GitHub methods are confined to
/// the reserved lanes and everything else stays off them, so a slow `gh`
/// call can never occupy a connection a session or health invoke needs. A
/// degraded pool (reconnect built fewer connections) falls back to sharing.
fn lane_range(method: MethodName, total: usize) -> std::ops::Range<usize> {
    if total <= GENERAL_LANES {
        return 0..total;
    }
    if method.domain() == "github" {
        GENERAL_LANES..total
    } else {
        0..GENERAL_LANES
    }
}

/// Prefer an idle lane, scanning from the rotation point so load still
/// spreads; only when every lane is mid-call does blind rotation apply.
fn pick_lane(
    lanes: &std::ops::Range<usize>,
    rotation: usize,
    busy: impl Fn(usize) -> bool,
) -> usize {
    let width = lanes.len().max(1);
    (0..width)
        .map(|offset| lanes.start + (rotation + offset) % width)
        .find(|index| !busy(*index))
        .unwrap_or(lanes.start + rotation % width)
}

/// The shared handle invokes call through. The supervisor installs a [`Link`]
/// when attached and clears it when the connection dies; calls that observe a
/// dead connection clear it too, so the supervisor reconnects promptly.
#[derive(Default)]
pub struct DaemonProxy {
    link: RwLock<Option<Arc<Link>>>,
    next: AtomicUsize,
    active_invokes: AtomicUsize,
}

struct InvokePermit {
    proxy: Arc<DaemonProxy>,
}

impl Drop for InvokePermit {
    fn drop(&mut self) {
        self.proxy.active_invokes.fetch_sub(1, Ordering::Release);
    }
}

impl DaemonProxy {
    fn reserve_invoke(self: &Arc<Self>) -> Option<InvokePermit> {
        self.active_invokes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_INVOKE_JOBS).then_some(active + 1)
            })
            .ok()?;
        Some(InvokePermit {
            proxy: self.clone(),
        })
    }

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
        let link = self
            .wait_for_link(link_wait)
            .ok_or_else(|| "The Bridge daemon is not reachable; still reconnecting".to_owned())?;
        let lanes = lane_range(method, link.clients.len());
        let rotation = self.next.fetch_add(1, Ordering::Relaxed);
        let index = pick_lane(&lanes, rotation, |index| link.clients[index].busy());
        let client = link.clients[index].clone();
        match client.call(method, params) {
            Ok(value) => Ok(value),
            // The code is the whole point of the 3000-range contract, and this
            // path used to drop it — leaving daemon-mode clients string-matching
            // the very messages the codes exist to replace. Both hosts now hand
            // the webview the same `{code,kind,message}` envelope.
            Err(ClientError::Rpc(error)) => Err(host_error_envelope(error)),
            Err(ClientError::Disconnected) => {
                self.invalidate(&link);
                Err("The Bridge daemon connection was lost; reconnecting".into())
            }
            Err(ClientError::Timeout) => Err(format!("{} did not answer in time", method.as_str())),
            Err(other) => Err(other.to_string()),
        }
    }

    pub fn attached(&self) -> bool {
        self.link.read().unwrap().is_some()
    }

    pub fn disconnect(&self) {
        self.invalidate_all();
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
        if link
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, seen))
        {
            *link = None;
        }
    }

    fn invalidate_all(&self) {
        *self.link.write().unwrap() = None;
    }
}

/// Whether an attached daemon is running different code than the binary this
/// launcher would spawn.
///
/// Attach-only clients have no expected binary identity. A launcher with a
/// bundled binary must not silently run against another build, but a mismatch
/// does not grant it ownership of the running daemon or permission to stop it.
fn daemon_is_stale(ours: Option<&str>, theirs: Option<&str>) -> bool {
    match (ours, theirs) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(ours), Some(theirs)) => ours != theirs,
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
        Launcher {
            data_dir,
            browser_extension,
            binary,
            child: None,
        }
    }

    /// A connection pool to a daemon serving the data directory, attaching to
    /// a running one or starting the bundled binary. Fails with an
    /// actionable message — including the daemon's own words when it refused
    /// us or exited during startup.
    pub fn ensure(&mut self) -> Result<Vec<Arc<DaemonClient>>, String> {
        self.ensure_with_deadline(START_DEADLINE)
    }

    fn ensure_with_deadline(
        &mut self,
        start_deadline: Duration,
    ) -> Result<Vec<Arc<DaemonClient>>, String> {
        self.ensure_with_stop(start_deadline, None)
    }

    fn ensure_with_stop(
        &mut self,
        start_deadline: Duration,
        stop: Option<&AtomicBool>,
    ) -> Result<Vec<Arc<DaemonClient>>, String> {
        bridge_client::socket_path_for_data_dir(&self.data_dir)
            .map_err(|error| error.to_string())?;
        let initial: Result<Vec<Arc<DaemonClient>>, ClientError> = match self.attach() {
            Ok(clients) => {
                self.validate_build(&clients)?;
                return Ok(clients);
            }
            Err(error) => Err(error),
        };
        match &initial {
            Ok(_) => unreachable!("the Ok case returned above"),
            // A live daemon answered and said no (bad token, incompatible
            // protocol). Starting a second one cannot help — the socket is
            // owned. Surface its refusal verbatim.
            Err(ClientError::Handshake(error)) if !retryable_handshake(error) => {
                return Err(format!(
                    "a running bridged daemon refused this app ({}): {}",
                    error.code, error.message
                ));
            }
            Err(_) => {}
        }
        let mut retrying_live_daemon = matches!(
            initial,
            Err(ClientError::Handshake(ref error)) if retryable_handshake(error)
        );
        let binary = match (self.binary.clone(), retrying_live_daemon) {
            (Some(binary), _) => Some(binary),
            (None, true) => None,
            (None, false) => {
                return Err(format!(
                    "no bridged daemon is serving {} and no daemon binary is available to start",
                    self.data_dir.display()
                ));
            }
        };
        let already_running = matches!(self.child.as_mut().map(Child::try_wait), Some(Ok(None)));
        if !already_running && !retrying_live_daemon {
            self.spawn(binary.as_deref().expect("binary checked above"))?;
        }
        let deadline = Instant::now() + start_deadline;
        loop {
            if stop.is_some_and(|stop| stop.load(Ordering::SeqCst)) {
                self.terminate_child();
                return Err("daemon startup cancelled".into());
            }
            std::thread::sleep(Duration::from_millis(200));
            match self.attach() {
                Ok(clients) => {
                    // Another desktop may have won the startup race, or a
                    // daemon may have restarted during the handshake retry.
                    // Apply the same identity check on every successful attach.
                    self.validate_build(&clients)?;
                    return Ok(clients);
                }
                Err(ClientError::Handshake(error)) if !retryable_handshake(&error) => {
                    self.terminate_child();
                    return Err(format!(
                        "the bridged daemon refused this app ({}): {}",
                        error.code, error.message
                    ));
                }
                Err(ClientError::Handshake(_)) => {}
                Err(_) if retrying_live_daemon && self.child.is_none() && binary.is_some() => {
                    self.spawn(binary.as_deref().unwrap())?;
                    retrying_live_daemon = false;
                    continue;
                }
                Err(_) => {}
            }
            // A startup failure (say, another owner holds the data-dir
            // lease) exits the child; report its last words instead of
            // timing out in silence.
            if let Some(Ok(Some(status))) = self.child.as_mut().map(Child::try_wait) {
                if retrying_live_daemon && binary.is_some() {
                    self.reap_child();
                    self.spawn(binary.as_deref().unwrap())?;
                    retrying_live_daemon = false;
                    continue;
                }
                self.reap_child();
                return Err(format!(
                    "bridged exited during startup ({status}): {}",
                    self.log_tail()
                ));
            }
            if Instant::now() >= deadline {
                self.terminate_child();
                return Err(format!(
                    "bridged did not become reachable within {start_deadline:?}: {}",
                    self.log_tail()
                ));
            }
        }
    }

    /// The identity of the binary this launcher would spawn. Recomputed per
    /// call on purpose: the whole point is noticing that the file changed
    /// under a long-running app, so memoizing it would rebuild the bug this
    /// check exists to kill. `None` for attach-only launchers and for a
    /// binary that cannot be read.
    fn binary_identity(&self) -> Option<String> {
        bridge_core::binary::file_identity(self.binary.as_deref()?).ok()
    }

    fn validate_build(&self, clients: &[Arc<DaemonClient>]) -> Result<(), String> {
        let ours = self.binary_identity();
        let theirs = clients
            .iter()
            .map(|client| client.handshake().build_id.as_deref())
            .find(|theirs| daemon_is_stale(ours.as_deref(), *theirs));
        if let Some(theirs) = theirs {
            return Err(format!(
                "A different Bridge build is already using {} (running backend {}, bundled backend {}). \
                 Quit the other Bridge copies, or stop a separately managed bridged daemon, then reopen this app. \
                 The running backend was left untouched so its active work can finish.",
                self.data_dir.display(),
                theirs.unwrap_or("unknown"),
                ours.as_deref().unwrap_or("unknown"),
            ));
        }
        Ok(())
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
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.log_path())
            .map_err(|error| format!("could not open the daemon log: {error}"))?;
        if !log
            .metadata()
            .map_err(|error| format!("could not inspect the daemon log: {error}"))?
            .is_file()
        {
            return Err("the daemon log is not a regular file".into());
        }
        std::fs::set_permissions(self.log_path(), std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("could not secure the daemon log: {error}"))?;
        if log.metadata().map(|metadata| metadata.len()).unwrap_or(0) > MAX_DAEMON_LOG_BYTES {
            log.set_len(0)
                .map_err(|error| format!("could not rotate the daemon log: {error}"))?;
        }
        let log_err = log
            .try_clone()
            .map_err(|error| format!("could not open the daemon log: {error}"))?;
        let child = daemon_command(binary, &self.data_dir, &self.browser_extension)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .spawn()
            .map_err(|error| format!("could not start {}: {error}", binary.display()))?;
        eprintln!(
            "bridge: started bridged (pid {}) for {}",
            child.id(),
            self.data_dir.display()
        );
        // Reap the previous child, if any, now that it has provably exited
        // (already_running was false) — never leave zombies behind.
        if let Some(mut old) = self.child.replace(child) {
            let _ = old.wait();
        }
        Ok(())
    }

    fn terminate_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            // try_wait caches an exited child's status and releases its PID.
            // Never signal that PID/group again: the group can still contain
            // surviving processes, and the numeric PID may have been reused.
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {}
                Err(error) => {
                    eprintln!(
                        "bridge: could not verify owned daemon process {} before shutdown: {error}",
                        child.id()
                    );
                    return;
                }
            }
            let group = -(child.id() as libc::pid_t);
            if unsafe { libc::kill(group, libc::SIGTERM) } != 0 {
                let _ = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
            }
            let deadline = Instant::now() + CHILD_SHUTDOWN_DEADLINE;
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    _ if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    _ => {
                        let _ = unsafe { libc::kill(group, libc::SIGKILL) };
                        let _ = child.kill();
                        break;
                    }
                }
            }
            let _ = child.wait();
        }
    }

    fn reap_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }

    fn log_path(&self) -> PathBuf {
        self.data_dir.join(DAEMON_LOG_FILE)
    }

    fn log_tail(&self) -> String {
        const TAIL_BYTES: u64 = 64 * 1024;
        let Ok(mut file) = std::fs::File::open(self.log_path()) else {
            return format!("no daemon log at {}", self.log_path().display());
        };
        let length = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        let start = length.saturating_sub(TAIL_BYTES);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return format!("could not seek daemon log {}", self.log_path().display());
        }
        let mut contents = Vec::new();
        if (&mut file)
            .take(TAIL_BYTES)
            .read_to_end(&mut contents)
            .is_err()
        {
            return format!("could not read daemon log {}", self.log_path().display());
        }
        let contents = String::from_utf8_lossy(&contents);
        let lines: Vec<&str> = contents.lines().rev().take(5).collect();
        if lines.is_empty() {
            return format!("daemon log {} is empty", self.log_path().display());
        }
        let mut tail: Vec<&str> = lines.into_iter().rev().collect();
        tail.insert(0, "last daemon log lines:");
        tail.join("\n")
    }
}

impl Drop for Launcher {
    fn drop(&mut self) {
        // A desktop-owned daemon is intentionally process-scoped. Do not
        // leave it behind when setup falls back or the app exits.
        self.terminate_child();
    }
}

fn retryable_handshake(error: &bridge_protocol::RpcError) -> bool {
    matches!(
        ErrorCode::from_code(error.code),
        Some(ErrorCode::Overloaded | ErrorCode::ShuttingDown)
    )
}

fn daemon_command(binary: &Path, data_dir: &Path, browser_extension: &Path) -> Command {
    bridge_core::adapters::supervised_command(
        binary,
        [
            std::ffi::OsStr::new("--data-dir"),
            data_dir.as_os_str(),
            std::ffi::OsStr::new("--health-addr"),
            std::ffi::OsStr::new("none"),
            std::ffi::OsStr::new("--browser-extension"),
            browser_extension.as_os_str(),
        ],
    )
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
    staged_binary(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .as_path(),
    )
}

fn staged_binary(directory: &Path) -> Option<PathBuf> {
    let candidate = directory.join(format!("bridged-{}", env!("TAURI_ENV_TARGET_TRIPLE")));
    candidate.is_file().then_some(candidate)
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
            None => match launcher.ensure_with_stop(START_DEADLINE, Some(stop)) {
                Ok(clients) => clients,
                Err(error) => {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
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
    subscription: &NotificationSubscription,
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
    if notification.method
        == bridge_protocol::notifications::NotificationName::StreamLagged.as_str()
    {
        emit_reconciliation(emit);
        return;
    }
    let payload = notification
        .params
        .map(Params::into_value)
        .unwrap_or(Value::Null);
    emit(&notification.method, payload);
}

fn emit_reconciliation<E: Fn(&str, Value)>(emit: &E) {
    use bridge_protocol::notifications::NotificationName;
    emit(NotificationName::StateChanged.as_str(), Value::Null);
    emit(NotificationName::AdaptersChanged.as_str(), Value::Null);
    // Terminals reconcile their sequenced VT checkpoints after lag or reconnect.
    emit(NotificationName::StreamLagged.as_str(), Value::Null);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    fn desktop_data_directory_defaults_to_the_platform_directory() {
        let default = PathBuf::from("/platform/Bridge");
        assert_eq!(desktop_data_dir(default.clone(), None).unwrap(), default);
    }

    #[test]
    fn desktop_data_directory_accepts_an_explicit_absolute_path() {
        let explicit = PathBuf::from("/isolated release test/Bridge");
        assert_eq!(
            desktop_data_dir(
                PathBuf::from("/platform/Bridge"),
                Some(explicit.clone().into_os_string())
            )
            .unwrap(),
            explicit
        );
    }

    #[test]
    fn desktop_data_directory_rejects_empty_and_relative_overrides() {
        for path in ["", "relative/Bridge", "~/Bridge"] {
            let error =
                desktop_data_dir(PathBuf::from("/platform/Bridge"), Some(path.into())).unwrap_err();
            assert!(error.contains("BRIDGE_DATA_DIR"), "{error}");
            assert!(error.contains("absolute path"), "{error}");
        }
    }

    #[test]
    fn simultaneous_desktop_starts_have_exactly_one_owner() {
        let fixture = tempfile::tempdir().unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let contenders = (0..8)
            .map(|_| {
                let directory = fixture.path().to_path_buf();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let lease = DesktopLease::acquire(&directory);
                    // Hold the winner until every contender has tried, so
                    // sequential reacquisition cannot mask an exclusion bug.
                    barrier.wait();
                    lease
                })
            })
            .collect::<Vec<_>>();
        let leases = contenders
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(leases.iter().filter(|lease| lease.is_ok()).count(), 1);
        for error in leases.iter().filter_map(|lease| lease.as_ref().err()) {
            assert!(error.contains("already open"), "{error}");
        }
        drop(leases);
        assert!(DesktopLease::acquire(fixture.path()).is_ok());
    }

    #[test]
    fn a_losing_desktop_preserves_the_existing_lock_file() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("desktop.lock");
        std::fs::write(&path, "existing lock contents").unwrap();
        let lease = DesktopLease::acquire(fixture.path()).unwrap();
        // Detect any chmod by the losing attempt as well as truncation.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(DesktopLease::acquire(fixture.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "existing lock contents"
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        drop(lease);
        assert!(DesktopLease::acquire(fixture.path()).is_ok());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn desktop_release_unlocks_a_descriptor_temporarily_inherited_by_a_child() {
        let fixture = tempfile::tempdir().unwrap();
        let lease = DesktopLease::acquire(fixture.path()).unwrap();
        // dup shares the open-file description just as fork does before exec.
        let inherited = lease._file.try_clone().unwrap();
        drop(lease);
        assert!(DesktopLease::acquire(fixture.path()).is_ok());
        drop(inherited);
    }

    #[test]
    fn desktop_lock_does_not_follow_a_symlink() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("other-file");
        std::fs::write(&target, "preserve this file").unwrap();
        std::os::unix::fs::symlink(&target, fixture.path().join("desktop.lock")).unwrap();
        assert!(DesktopLease::acquire(fixture.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            "preserve this file"
        );
    }

    #[test]
    fn a_long_socket_path_is_rejected_before_spawning_a_daemon() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path().join("long-directory-".repeat(10));
        let binary = fixture.path().join("fake-bridged");
        std::fs::write(&binary, "#!/bin/sh\nexit 99\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut launcher = Launcher::new(
            data_dir.clone(),
            fixture.path().join("extension"),
            Some(binary),
        );
        let error = match launcher.ensure() {
            Ok(_) => panic!("an overlong daemon path was accepted"),
            Err(error) => error,
        };
        assert!(error.contains("shorter absolute directory"), "{error}");
        assert!(launcher.child.is_none());
        assert!(
            !data_dir.exists(),
            "preflight must not spawn or create a daemon log"
        );
    }

    #[test]
    fn parameterless_methods_send_no_params() {
        // The JS invoke helper sends `{}` for argument-free calls; the daemon
        // contract requires their absence.
        assert_eq!(
            wire_params(MethodName::GetState, serde_json::json!({})),
            None
        );
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
            RpcNotification::new(
                "stream-lagged",
                Params::new(serde_json::json!({"missed": 3})).ok(),
            ),
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
            seen.iter()
                .map(|(kind, _)| kind.as_str())
                .collect::<Vec<_>>(),
            vec!["state-changed", "adapters-changed", "stream-lagged", "session-output"],
        );
        assert_eq!(seen[0].1, Value::Null);
        assert_eq!(seen[3].1["data"], "x");
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
        assert_eq!(parse_host_preference(None), Ok(HostPreference::Auto));
        assert_eq!(
            parse_host_preference(Some("auto")),
            Ok(HostPreference::Auto)
        );
        assert_eq!(
            parse_host_preference(Some("daemon")),
            Ok(HostPreference::DaemonOnly)
        );
        assert_eq!(
            parse_host_preference(Some("embedded")),
            Ok(HostPreference::EmbeddedOnly)
        );
        assert!(parse_host_preference(Some("deamon")).is_err());
    }

    #[test]
    fn daemon_command_disables_the_global_health_listener() {
        let command = daemon_command(
            Path::new("bridged"),
            Path::new("data"),
            Path::new("extension"),
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            &args[args.len() - 6..],
            [
                "--data-dir",
                "data",
                "--health-addr",
                "none",
                "--browser-extension",
                "extension"
            ]
        );
    }

    #[test]
    fn child_shutdown_budget_exceeds_the_daemon_drain_deadline() {
        assert!(CHILD_SHUTDOWN_DEADLINE > bridged::DEFAULT_DRAIN_TIMEOUT);
    }

    #[test]
    fn staged_binary_requires_the_current_target_triple() {
        let fixture = tempfile::tempdir().unwrap();
        let expected = fixture
            .path()
            .join(format!("bridged-{}", env!("TAURI_ENV_TARGET_TRIPLE")));
        let stale = fixture.path().join("bridged-aarch64-stale-target");
        std::fs::write(&stale, b"stale").unwrap();
        assert_eq!(staged_binary(fixture.path()), None);
        std::fs::write(&expected, b"current").unwrap();
        assert_eq!(staged_binary(fixture.path()), Some(expected));
    }

    #[test]
    fn github_methods_are_confined_to_their_reserved_lanes() {
        // Full pool: github stays on the reserved lanes, the rest stays off.
        assert_eq!(
            lane_range(MethodName::GithubPullRequests, POOL_SIZE),
            GENERAL_LANES..POOL_SIZE
        );
        assert_eq!(lane_range(MethodName::Health, POOL_SIZE), 0..GENERAL_LANES);
        // A degraded pool (reconnect built fewer connections) shares.
        assert_eq!(lane_range(MethodName::GithubPullRequests, 2), 0..2);
        assert_eq!(lane_range(MethodName::Health, 2), 0..2);
    }

    #[test]
    fn an_idle_lane_is_preferred_over_a_busy_rotation_target() {
        // Rotation points at lane 1, which is busy; lane 2 is idle.
        assert_eq!(pick_lane(&(0..4), 1, |index| index == 1), 2);
        // Nothing busy: pure rotation.
        assert_eq!(pick_lane(&(0..4), 5, |_| false), 1);
        // Everything busy: fall back to rotation instead of stalling.
        assert_eq!(pick_lane(&(0..4), 6, |_| true), 2);
        // Offsets apply within the partition, not the whole pool.
        assert_eq!(pick_lane(&(4..6), 0, |index| index == 4), 5);
    }

    #[test]
    fn invoke_jobs_are_bounded() {
        let proxy = Arc::new(DaemonProxy::default());
        let permits = (0..MAX_INVOKE_JOBS)
            .map(|_| proxy.reserve_invoke().expect("slot available"))
            .collect::<Vec<_>>();
        assert!(proxy.reserve_invoke().is_none());
        drop(permits);
        assert!(proxy.reserve_invoke().is_some());
    }

    /// The decision table. `ours` is the launcher's binary identity, `theirs`
    /// the daemon's handshake report.
    #[test]
    fn staleness_is_a_mismatch_and_attach_only_launchers_never_see_one() {
        // Attach-only: nothing better to offer, so nothing is stale.
        assert!(!daemon_is_stale(None, None));
        assert!(!daemon_is_stale(None, Some("aaaa")));
        // A daemon that reports no identity predates the check: stale.
        assert!(daemon_is_stale(Some("aaaa"), None));
        // The actual comparison.
        assert!(!daemon_is_stale(Some("aaaa"), Some("aaaa")));
        assert!(daemon_is_stale(Some("aaaa"), Some("bbbb")));
    }

    /// Runs a real second process that owns a data-directory lease and serves
    /// handshakes. Keeping it outside the test process catches accidental
    /// SIGTERM/SIGKILL of a foreign owner without endangering the test runner.
    struct ForeignDaemon {
        fixture: tempfile::TempDir,
        child: Child,
        data_dir: PathBuf,
        binary: PathBuf,
    }

    impl ForeignDaemon {
        fn start(build_id: Option<&str>, refuse_first: bool) -> Self {
            let fixture = tempfile::tempdir().unwrap();
            let data_dir = fixture.path().join("data");
            std::fs::create_dir_all(&data_dir).unwrap();
            let binary = fixture.path().join("fake-bridged");
            std::fs::write(
                &binary,
                format!(
                    "#!/bin/sh\necho spawned > '{}'\nexec sleep 30\n",
                    fixture.path().join("spawned").display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "daemon_host::tests::helper_foreign_daemon",
                    "--nocapture",
                ])
                .env("BRIDGE_FOREIGN_DAEMON_TEST_DIR", &data_dir)
                .env("BRIDGE_FOREIGN_DAEMON_TEST_BUILD", build_id.unwrap_or(""))
                .env(
                    "BRIDGE_FOREIGN_DAEMON_TEST_REFUSE_FIRST",
                    if refuse_first { "1" } else { "0" },
                )
                .stdout(Stdio::null());
            let child = command.spawn().unwrap();
            let mut daemon = Self {
                fixture,
                child,
                data_dir,
                binary,
            };
            let deadline = Instant::now() + Duration::from_secs(15);
            while !daemon.data_dir.join("ready").exists() {
                assert!(
                    daemon.child.try_wait().unwrap().is_none(),
                    "helper exited before ready"
                );
                assert!(Instant::now() < deadline, "helper did not become ready");
                std::thread::sleep(Duration::from_millis(20));
            }
            daemon
        }

        fn launcher(&self, attach_only: bool) -> Launcher {
            Launcher::new(
                self.data_dir.clone(),
                self.fixture.path().join("extension"),
                (!attach_only).then(|| self.binary.clone()),
            )
        }

        fn assert_untouched(&mut self) {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "foreign daemon was stopped"
            );
            let owner = bridge_core::ownership::DataDirLease::current_holder(&self.data_dir)
                .expect("foreign daemon must retain its lease");
            assert_eq!(owner.pid, self.child.id());
            assert!(
                !self.fixture.path().join("spawned").exists(),
                "replacement was started"
            );
        }
    }

    impl Drop for ForeignDaemon {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn mismatched_foreign_daemons_are_refused_without_interrupting_the_owner() {
        // Missing identities are mismatches too. Neither case permits this
        // app to signal another desktop's backend or start a replacement.
        for build_id in [Some("0000000000000000"), None] {
            let mut daemon = ForeignDaemon::start(build_id, false);
            let mut launcher = daemon.launcher(false);
            let error = match launcher.ensure_with_deadline(Duration::from_secs(2)) {
                Ok(_) => panic!("mismatched backend was attached to"),
                Err(error) => error,
            };
            assert!(error.contains("different Bridge build"), "{error}");
            assert!(error.contains("left untouched"), "{error}");
            drop(launcher);
            daemon.assert_untouched();
        }
    }

    #[test]
    fn a_mismatch_after_a_retryable_handshake_also_leaves_the_owner_running() {
        let mut daemon = ForeignDaemon::start(Some("0000000000000000"), true);
        let mut launcher = daemon.launcher(false);
        let error = match launcher.ensure_with_deadline(Duration::from_secs(2)) {
            Ok(_) => panic!("retry attached to a mismatched backend"),
            Err(error) => error,
        };
        assert!(error.contains("different Bridge build"), "{error}");
        drop(launcher);
        daemon.assert_untouched();
    }

    #[test]
    fn dropping_an_attach_only_launcher_leaves_the_foreign_daemon_running() {
        let mut daemon = ForeignDaemon::start(Some("0000000000000000"), false);
        let mut launcher = daemon.launcher(true);
        let clients = launcher
            .ensure()
            .expect("attach-only clients accept the running build");
        assert_eq!(clients.len(), POOL_SIZE);
        drop(clients);
        drop(launcher);
        daemon.assert_untouched();
    }

    /// Re-exec target for ForeignDaemon. Without the marker it is a no-op.
    #[test]
    fn helper_foreign_daemon() {
        let Some(data_dir) = std::env::var_os("BRIDGE_FOREIGN_DAEMON_TEST_DIR") else {
            return;
        };
        let data_dir = PathBuf::from(data_dir);
        let _lease = bridge_core::ownership::DataDirLease::acquire(
            &data_dir,
            bridge_core::ownership::OwnerKind::Daemon,
        )
        .unwrap();
        std::fs::write(data_dir.join(bridge_client::TOKEN_FILE_NAME), "test-token").unwrap();
        let listener = UnixListener::bind(data_dir.join(bridge_client::SOCKET_FILE_NAME)).unwrap();
        let build_id = std::env::var("BRIDGE_FOREIGN_DAEMON_TEST_BUILD").unwrap();
        let mut refuse_next =
            std::env::var("BRIDGE_FOREIGN_DAEMON_TEST_REFUSE_FIRST").unwrap() == "1";
        let mut connections = Vec::new();
        std::fs::write(data_dir.join("ready"), b"ready").unwrap();
        for socket in listener.incoming() {
            let mut socket = socket.unwrap();
            let mut request = String::new();
            BufReader::new(socket.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            let request: bridge_protocol::RpcRequest = serde_json::from_str(&request).unwrap();
            assert_eq!(request.method, bridge_protocol::HANDSHAKE_METHOD);
            let response = if refuse_next {
                refuse_next = false;
                bridge_protocol::RpcResponse::error(
                    request.id,
                    bridge_protocol::RpcError::new(ErrorCode::ShuttingDown, "retry the daemon"),
                )
            } else {
                bridge_protocol::RpcResponse::result(
                    request.id,
                    serde_json::json!({
                        "protocolVersion": bridge_protocol::PROTOCOL_VERSION,
                        "server": {"name": "bridge", "version": "0.1.0"},
                        "capabilities": ["sessions"],
                        "buildId": (!build_id.is_empty()).then_some(&build_id),
                    }),
                )
            };
            serde_json::to_writer(&mut socket, &response).unwrap();
            socket.write_all(b"\n").unwrap();
            connections.push(socket);
        }
    }

    #[test]
    fn launcher_drop_kills_and_reaps_an_owned_child() {
        let fixture = tempfile::tempdir().unwrap();
        let mut launcher = Launcher::new(
            fixture.path().join("data"),
            fixture.path().join("extension"),
            None,
        );
        launcher.child = Some(Command::new("sleep").arg("30").spawn().unwrap());
        let pid = launcher.child.as_ref().unwrap().id();
        drop(launcher);
        let alive = unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        assert!(!alive, "owned child {pid} survived launcher drop");
    }

    #[test]
    fn dropping_a_reaped_child_does_not_signal_its_former_process_group() {
        let fixture = tempfile::tempdir().unwrap();
        let mut leader = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let mut survivor = Command::new("sleep")
            .arg("30")
            .process_group(leader.id() as libc::pid_t)
            .spawn()
            .unwrap();
        leader.kill().unwrap();
        leader.wait().unwrap();

        let mut launcher = Launcher::new(
            fixture.path().join("data"),
            fixture.path().join("extension"),
            None,
        );
        launcher.child = Some(leader);
        drop(launcher);
        std::thread::sleep(Duration::from_millis(50));
        let survived = survivor.try_wait().unwrap().is_none();
        let _ = survivor.kill();
        let _ = survivor.wait();
        assert!(
            survived,
            "an already reaped child caused a process-group signal"
        );
    }

    #[test]
    fn startup_timeout_kills_and_reaps_an_unreachable_child() {
        let fixture = tempfile::tempdir().unwrap();
        let binary = fixture.path().join("fake-bridged");
        std::fs::write(&binary, b"#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let data_dir = fixture.path().join("data");
        let mut launcher = Launcher::new(
            data_dir.clone(),
            fixture.path().join("extension"),
            Some(binary),
        );
        let binary = launcher.binary.clone().unwrap();
        launcher.spawn(&binary).unwrap();
        let pid = launcher.child.as_ref().unwrap().id() as libc::pid_t;
        let error = match launcher.ensure_with_deadline(Duration::from_millis(250)) {
            Ok(_) => panic!("unreachable child unexpectedly accepted connections"),
            Err(error) => error,
        };
        assert!(error.contains("did not become reachable"), "{error}");
        assert!(launcher.child.is_none());
        assert_ne!(
            unsafe { libc::kill(pid, 0) },
            0,
            "child {pid} survived timeout"
        );
    }

    #[test]
    fn a_disappeared_shutting_down_daemon_allows_replacement_spawn() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join(bridge_client::TOKEN_FILE_NAME), "test-token").unwrap();
        let socket_path = data_dir.join(bridge_client::SOCKET_FILE_NAME);
        let listener = UnixListener::bind(&socket_path).unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(socket.try_clone().unwrap())
                .read_line(&mut request)
                .unwrap();
            assert!(!request.is_empty());
            let refusal = bridge_protocol::RpcResponse::error(
                bridge_protocol::ResponseId::Null,
                bridge_protocol::RpcError::new(
                    ErrorCode::ShuttingDown,
                    "the daemon is shutting down",
                ),
            );
            serde_json::to_writer(&mut socket, &refusal).unwrap();
            socket.write_all(b"\n").unwrap();
        });

        let spawned = fixture.path().join("spawned");
        let binary = fixture.path().join("fake-bridged");
        std::fs::write(
            &binary,
            format!(
                "#!/bin/sh\necho spawned > '{}'\nexec sleep 30\n",
                spawned.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut launcher = Launcher::new(data_dir, fixture.path().join("extension"), Some(binary));
        let error = match launcher.ensure_with_deadline(Duration::from_millis(1500)) {
            Ok(_) => panic!("fake replacement unexpectedly accepted connections"),
            Err(error) => error,
        };
        server.join().unwrap();
        assert!(error.contains("did not become reachable"), "{error}");
        assert!(
            spawned.is_file(),
            "replacement binary was never started: {error}"
        );
        assert!(launcher.child.is_none());
    }
}

/// Render an `RpcError` as the same envelope an embedded command produces, so a
/// client branches on `code` in either host mode instead of parsing prose.
fn host_error_envelope(error: bridge_protocol::RpcError) -> String {
    let code = bridge_protocol::ErrorCode::from_code(error.code);
    serde_json::to_string(&serde_json::json!({
        "code": error.code,
        "kind": code.map(|code| code.name()),
        "message": error.message,
    }))
    .unwrap_or(error.message)
}
