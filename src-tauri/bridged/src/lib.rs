//! `bridged` — the single local owner of Bridge's sessions, SQLite stores,
//! PTYs, worktrees, and provider processes, serving the `bridge-protocol`
//! contract to every client.
//!
//! Shape:
//! - **Ownership**: an exclusive [`bridge_core::ownership::DataDirLease`] on
//!   the data directory, acquired before anything else. A second owner —
//!   daemon or embedded desktop app — fails fast with the holder's identity.
//! - **Transport**: newline-delimited JSON-RPC 2.0 over a Unix-domain socket.
//!   The first request on a connection must be `protocol/handshake`, carrying
//!   the per-install token from `<data_dir>/daemon.token`, and it must arrive
//!   within the handshake deadline — an unauthenticated peer cannot park on a
//!   connection slot.
//! - **Events**: one [`EventHub`] per daemon subscribes to the core bus and
//!   fans out to per-connection bounded queues. A connection that falls
//!   behind gets a `stream-lagged` marker plus the idempotent refetch hints
//!   the moment it drains; durable history recovers via
//!   `sessions/replay_session_events`.
//! - **Limits**: request frames are capped at [`MAX_FRAME_BYTES`]; concurrent
//!   connections at [`MAX_CONNECTIONS`] (excess connections are refused with
//!   the stable `overloaded` code). Requests on a connection are handled
//!   sequentially — overload backpressure is the socket itself.
//! - **Health**: `/healthz` (liveness) and `/readyz` (readiness) on a private
//!   loopback HTTP listener. Startup failures — socket bind, health bind,
//!   lease, token, boot — are process-fatal and printed, never silently
//!   swallowed.

mod dispatch;
mod events;
mod server;

use bridge_core::events::EventBus;
use bridge_core::ownership::{DataDirLease, OwnerKind};
use bridge_core::{BootConfig, BridgeCore};
use std::io::Write;
use std::net::SocketAddr;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use events::EventHub;
pub use server::serve;

/// Hard cap on a single request frame (one line). A frame beyond this is
/// answered with `invalid_request` and the connection is closed.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Concurrent connection cap. Connections beyond it receive one `overloaded`
/// error frame and are closed without a handshake.
pub const MAX_CONNECTIONS: usize = 32;

/// How long a fresh connection may take to complete its handshake before the
/// slot is reclaimed.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a graceful shutdown waits for in-flight connections to finish
/// before proceeding to adapter teardown anyway.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

pub const SOCKET_FILE_NAME: &str = "bridged.sock";
pub const TOKEN_FILE_NAME: &str = "daemon.token";

/// Everything `main` resolves from flags before the daemon starts.
pub struct DaemonConfig {
    pub data_dir: PathBuf,
    /// Socket path; defaults to `<data_dir>/bridged.sock`.
    pub socket_path: Option<PathBuf>,
    /// Loopback address for `/healthz` + `/readyz`; `None` disables them.
    pub health_addr: Option<SocketAddr>,
    /// Browser extension directory handed to the browser supervisor.
    pub browser_extension_path: PathBuf,
    /// See [`DEFAULT_HANDSHAKE_TIMEOUT`]; tests shorten it.
    pub handshake_timeout: Duration,
}

/// Cross-thread daemon state: the readiness flag `/readyz` reports, the
/// shutdown flag every accept/read loop polls, and the live connection count.
pub struct DaemonState {
    pub ready: AtomicBool,
    pub shutting_down: AtomicBool,
    pub connections: AtomicUsize,
    pub auth_token: String,
    pub handshake_timeout: Duration,
    /// This process's executable identity, computed once at startup — while
    /// the file at `current_exe()` is still the binary that is running — and
    /// reported in every handshake so a launcher holding a newer build can
    /// replace this daemon instead of silently running stale code. `None`
    /// when the read failed; the launcher treats that as stale, which errs
    /// toward a restart rather than toward staleness going unnoticed.
    pub build_id: Option<String>,
}

/// The running daemon, as far as `main` is concerned.
pub struct Daemon {
    pub core: Arc<BridgeCore>,
    pub state: Arc<DaemonState>,
    pub socket_path: PathBuf,
    pub events: EventHub,
    // Held for the process lifetime; releasing it is what lets the next
    // owner in.
    _lease: DataDirLease,
}

/// Errors during startup. All of them are fatal and printed by `main` —
/// a daemon that cannot bind must say so, not idle as a no-op.
#[derive(Debug)]
pub enum StartupError {
    Ownership(bridge_core::ownership::OwnershipError),
    Boot(bridge_core::BridgeError),
    Token(std::io::Error),
    /// The token file exists but cannot be trusted (wrong type, permissions,
    /// or content). Refusing beats silently adopting an attacker-supplied or
    /// corrupted secret.
    TokenUntrusted { path: PathBuf, reason: String },
    /// The socket path exists and is not a Unix socket. Never deleted — the
    /// operator pointed `--socket` at the wrong place.
    SocketPathOccupied { path: PathBuf },
    /// The socket path is a Unix socket with a live listener — another
    /// daemon's socket, possibly for a different data directory.
    SocketInUse { path: PathBuf },
    SocketBind { path: PathBuf, error: std::io::Error },
    SocketPermissions { path: PathBuf, error: std::io::Error },
    HealthBind { addr: SocketAddr, error: std::io::Error },
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartupError::Ownership(error) => write!(formatter, "{error}"),
            StartupError::Boot(error) => write!(formatter, "runtime boot failed: {error}"),
            StartupError::Token(error) => {
                write!(formatter, "could not prepare the daemon token file: {error}")
            }
            StartupError::TokenUntrusted { path, reason } => write!(
                formatter,
                "refusing the existing token file at {}: {reason} — delete it to \
                 generate a fresh token",
                path.display()
            ),
            StartupError::SocketPathOccupied { path } => write!(
                formatter,
                "the socket path {} exists and is not a Unix socket; refusing to \
                 delete it — pass a different --socket",
                path.display()
            ),
            StartupError::SocketInUse { path } => write!(
                formatter,
                "another process is listening on {}; stop it or pass a different --socket",
                path.display()
            ),
            StartupError::SocketBind { path, error } => write!(
                formatter,
                "could not bind the daemon socket at {}: {error}",
                path.display()
            ),
            StartupError::SocketPermissions { path, error } => write!(
                formatter,
                "could not restrict the daemon socket at {} to owner-only access: {error}",
                path.display()
            ),
            StartupError::HealthBind { addr, error } => write!(
                formatter,
                "could not bind the health listener at {addr}: {error} \
                 (pass --health-addr none to disable it)"
            ),
        }
    }
}

impl std::error::Error for StartupError {}

impl Daemon {
    /// Acquire ownership, boot the runtime, prepare the token, and bind the
    /// socket and health listeners. On return the daemon is ready; call
    /// [`serve`] to run the accept loop.
    pub fn start(config: DaemonConfig) -> Result<(Daemon, std::os::unix::net::UnixListener), StartupError> {
        let lease = DataDirLease::acquire(&config.data_dir, OwnerKind::Daemon)
            .map_err(StartupError::Ownership)?;

        // Subscribe-before-boot is the host contract: boot-time events
        // (adapter discovery) must be observable by early connections.
        let bus = EventBus::new();
        let core = BridgeCore::boot(BootConfig {
            data_dir: config.data_dir.clone(),
            browser_extension_path: config.browser_extension_path.clone(),
            events: Some(bus),
        })
        .map_err(StartupError::Boot)?;
        let core = Arc::new(core);
        let events = EventHub::start(&core);

        let auth_token = ensure_token(&config.data_dir.join(TOKEN_FILE_NAME))?;

        let socket_path = config
            .socket_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join(SOCKET_FILE_NAME));
        prepare_socket_path(&socket_path)?;
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).map_err(|error| {
            StartupError::SocketBind { path: socket_path.clone(), error }
        })?;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| StartupError::SocketPermissions { path: socket_path.clone(), error },
        )?;

        let state = Arc::new(DaemonState {
            ready: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            auth_token,
            handshake_timeout: config.handshake_timeout,
            build_id: bridge_core::binary::self_identity().ok(),
        });

        if let Some(addr) = config.health_addr {
            start_health_listener(addr, state.clone())?;
        }

        // Maintenance loops are part of ownership, not of any client.
        bridge_core::live_turn::start_worker_maintenance(core.clone());
        bridge_core::live_turn::start_completion_check_maintenance(core.clone());
        bridge_core::work_observation::start_work_fact_maintenance(core.clone());
        bridge_core::live_turn::start_learning_maintenance(core.clone());
        bridge_core::work_briefing_live::start_briefing_maintenance(core.clone());
        bridge_core::github_poll::start_github_poll_maintenance(core.clone());
        bridge_core::memory_extraction_live::start_extraction_maintenance(core.clone());
        bridge_core::routing_evaluation_live::start_evaluation_maintenance(core.clone());
        bridge_core::live_turn::start_queued_input_maintenance(core.clone());
        bridge_core::live_turn::start_history_snapshot_maintenance(core.clone());

        state.ready.store(true, Ordering::SeqCst);
        Ok((
            Daemon { core, state, socket_path, events, _lease: lease },
            listener,
        ))
    }

    /// Graceful shutdown: refuse new work, drain in-flight connections (their
    /// read loops poll the flag and close), then stop every live adapter so
    /// sessions land in a recoverable stopped state, and remove the socket.
    /// The lease releases when the daemon drops.
    pub fn shutdown(&self, drain_timeout: Duration) {
        self.state.ready.store(false, Ordering::SeqCst);
        self.state.shutting_down.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + drain_timeout;
        while self.state.connections.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        let stragglers = self.state.connections.load(Ordering::SeqCst);
        if stragglers > 0 {
            eprintln!(
                "bridged: proceeding with shutdown while {stragglers} connection(s) \
                 are still mid-request"
            );
        }
        let sessions: Vec<String> =
            self.core.adapters.lock().unwrap().keys().cloned().collect();
        for session_id in sessions {
            self.core
                .stop_session_adapter(&session_id, bridge_core::adapters::ShutdownReason::AppShutdown);
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Refuse to bind over anything that is not provably a stale socket of a dead
/// daemon: a live listener is someone else's socket, and any non-socket file
/// is a path the operator mistyped — deleting either would be destructive.
fn prepare_socket_path(path: &Path) -> Result<(), StartupError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(StartupError::SocketBind { path: path.to_path_buf(), error })
        }
        Ok(metadata) => metadata,
    };
    if !metadata.file_type().is_socket() {
        return Err(StartupError::SocketPathOccupied { path: path.to_path_buf() });
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(StartupError::SocketInUse { path: path.to_path_buf() });
    }
    std::fs::remove_file(path)
        .map_err(|error| StartupError::SocketBind { path: path.to_path_buf(), error })
}

/// Load the per-install token, creating it (0600) on first start. An existing
/// file is trusted only when it is a regular, non-symlink, owner-only file
/// holding exactly the 64 hex characters this daemon generates.
fn ensure_token(path: &Path) -> Result<String, StartupError> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(StartupError::Token(error)),
        Ok(metadata) => {
            let untrusted = |reason: &str| StartupError::TokenUntrusted {
                path: path.to_path_buf(),
                reason: reason.to_owned(),
            };
            if !metadata.file_type().is_file() {
                return Err(untrusted("it is not a regular file"));
            }
            let mode = metadata.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(untrusted(&format!(
                    "its permissions are {mode:03o}, not owner-only (0600)"
                )));
            }
            let existing = std::fs::read_to_string(path).map_err(StartupError::Token)?;
            let existing = existing.trim();
            if existing.len() != 64 || !existing.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(untrusted("its content is not a 64-character hex token"));
            }
            return Ok(existing.to_owned());
        }
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut options = std::fs::OpenOptions::new();
    // create_new: never follow a symlink planted between the check and here.
    options.write(true).create_new(true);
    options.mode(0o600);
    let mut file = options.open(path).map_err(StartupError::Token)?;
    file.write_all(token.as_bytes()).map_err(StartupError::Token)?;
    file.flush().map_err(StartupError::Token)?;
    Ok(token)
}

/// Constant-time token comparison — an attacker on a shared machine must not
/// learn the token byte-by-byte from response timing.
pub(crate) fn token_matches(expected: &str, presented: &str) -> bool {
    let expected = expected.as_bytes();
    let presented = presented.as_bytes();
    if expected.len() != presented.len() {
        return false;
    }
    expected
        .iter()
        .zip(presented)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// `/healthz` and `/readyz` — the daemon's only plain-HTTP surface. Port 4317
/// (health + credential proxy) is deliberately not extended.
fn start_health_listener(addr: SocketAddr, state: Arc<DaemonState>) -> Result<(), StartupError> {
    let server = tiny_http::Server::http(addr)
        .map_err(|error| StartupError::HealthBind {
            addr,
            error: std::io::Error::other(error.to_string()),
        })?;
    std::thread::Builder::new()
        .name("bridged-health".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let (status, body) = match request.url() {
                    "/healthz" => (
                        200,
                        serde_json::json!({"ok": true, "version": env!("CARGO_PKG_VERSION")}),
                    ),
                    "/readyz" => {
                        if state.ready.load(Ordering::SeqCst) {
                            (200, serde_json::json!({"ready": true}))
                        } else {
                            (503, serde_json::json!({"ready": false}))
                        }
                    }
                    _ => (404, serde_json::json!({"ok": false, "error": "not found"})),
                };
                let mut response =
                    tiny_http::Response::from_string(body.to_string()).with_status_code(status);
                if let Ok(header) =
                    tiny_http::Header::from_bytes("Content-Type", "application/json")
                {
                    response.add_header(header);
                }
                let _ = request.respond(response);
            }
        })
        .expect("health listener thread spawns");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_persist_across_starts_and_are_owner_readable_only() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join(TOKEN_FILE_NAME);
        let first = ensure_token(&path).unwrap();
        assert_eq!(first.len(), 64, "two v4 UUIDs, hex, no hyphens");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "the token must not be group/world readable");
        let second = ensure_token(&path).unwrap();
        assert_eq!(first, second, "restarts keep the install's token");
    }

    #[test]
    fn untrustworthy_token_files_are_refused_not_adopted() {
        let fixture = tempfile::tempdir().unwrap();

        // Group/world-readable file.
        let readable = fixture.path().join("readable.token");
        std::fs::write(&readable, "a".repeat(64)).unwrap();
        std::fs::set_permissions(&readable, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = ensure_token(&readable).unwrap_err();
        assert!(error.to_string().contains("owner-only"), "{error}");

        // Malformed content.
        let malformed = fixture.path().join("malformed.token");
        std::fs::write(&malformed, "hello\n").unwrap();
        std::fs::set_permissions(&malformed, std::fs::Permissions::from_mode(0o600)).unwrap();
        let error = ensure_token(&malformed).unwrap_err();
        assert!(error.to_string().contains("64-character hex"), "{error}");

        // A symlink, even to a valid token file.
        let target = fixture.path().join("target.token");
        std::fs::write(&target, "b".repeat(64)).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = fixture.path().join("link.token");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let error = ensure_token(&link).unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");
    }

    #[test]
    fn socket_paths_are_never_deleted_unless_provably_stale_sockets() {
        let fixture = tempfile::tempdir().unwrap();

        // A regular file at the socket path is refused, not replaced.
        let file = fixture.path().join("not-a-socket");
        std::fs::write(&file, "precious data").unwrap();
        let error = prepare_socket_path(&file).unwrap_err();
        assert!(matches!(error, StartupError::SocketPathOccupied { .. }), "{error}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "precious data");

        // A socket with a live listener is refused.
        let live = fixture.path().join("live.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
        let error = prepare_socket_path(&live).unwrap_err();
        assert!(matches!(error, StartupError::SocketInUse { .. }), "{error}");

        // A dead daemon's socket is stale and reclaimed.
        let stale = fixture.path().join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        prepare_socket_path(&stale).unwrap();
        assert!(!stale.exists());

        // A missing path is fine.
        prepare_socket_path(&fixture.path().join("fresh.sock")).unwrap();
    }

    #[test]
    fn token_comparison_requires_exact_match() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", "ab"));
        assert!(!token_matches("abc", ""));
    }
}
