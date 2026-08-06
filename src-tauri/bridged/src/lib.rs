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
//!   the per-install token from `<data_dir>/daemon.token`.
//! - **Events**: after the handshake, every core event is pushed to the
//!   connection as a JSON-RPC notification. The channel is bounded and lossy
//!   by contract; on lag the daemon resends the idempotent refetch hints and
//!   clients recover durable history via `sessions/replay_session_events`.
//! - **Limits**: request frames are capped at [`MAX_FRAME_BYTES`]; concurrent
//!   connections at [`MAX_CONNECTIONS`] (excess connections are refused with
//!   the stable `overloaded` code). Requests on a connection are handled
//!   sequentially — overload backpressure is the socket itself.
//! - **Health**: `/healthz` (liveness) and `/readyz` (readiness) on a private
//!   loopback HTTP listener. Startup failures — socket bind, health bind,
//!   lease, boot — are process-fatal and printed, never silently swallowed.

mod dispatch;
mod server;

use bridge_core::events::EventBus;
use bridge_core::ownership::{DataDirLease, OwnerKind};
use bridge_core::{BootConfig, BridgeCore};
use std::io::Write;
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

pub use server::serve;

/// Hard cap on a single request frame (one line). A frame beyond this is
/// answered with `invalid_request` and the connection is closed.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Concurrent connection cap. Connections beyond it receive one `overloaded`
/// error frame and are closed without a handshake.
pub const MAX_CONNECTIONS: usize = 32;

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
}

/// Cross-thread daemon state: the readiness flag `/readyz` reports and the
/// shutdown flag every accept/read loop polls.
pub struct DaemonState {
    pub ready: AtomicBool,
    pub shutting_down: AtomicBool,
    pub connections: AtomicUsize,
    pub auth_token: String,
}

/// The running daemon, as far as `main` is concerned.
pub struct Daemon {
    pub core: Arc<BridgeCore>,
    pub state: Arc<DaemonState>,
    pub socket_path: PathBuf,
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
    SocketBind { path: PathBuf, error: std::io::Error },
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
            StartupError::SocketBind { path, error } => write!(
                formatter,
                "could not bind the daemon socket at {}: {error}",
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
        let events = EventBus::new();
        let core = BridgeCore::boot(BootConfig {
            data_dir: config.data_dir.clone(),
            browser_extension_path: config.browser_extension_path.clone(),
            events: Some(events),
        })
        .map_err(StartupError::Boot)?;
        let core = Arc::new(core);

        let auth_token =
            ensure_token(&config.data_dir.join(TOKEN_FILE_NAME)).map_err(StartupError::Token)?;

        let socket_path = config
            .socket_path
            .clone()
            .unwrap_or_else(|| config.data_dir.join(SOCKET_FILE_NAME));
        // A stale socket file cannot be a live server: the lease proves no
        // other owner exists, so removing it is safe.
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        let listener = std::os::unix::net::UnixListener::bind(&socket_path).map_err(|error| {
            StartupError::SocketBind { path: socket_path.clone(), error }
        })?;
        let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));

        let state = Arc::new(DaemonState {
            ready: AtomicBool::new(false),
            shutting_down: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            auth_token,
        });

        if let Some(addr) = config.health_addr {
            start_health_listener(addr, state.clone())?;
        }

        // Maintenance loops are part of ownership, not of any client.
        bridge_core::live_turn::start_worker_maintenance(core.clone());
        bridge_core::live_turn::start_learning_maintenance(core.clone());
        bridge_core::live_turn::start_history_snapshot_maintenance(core.clone());

        state.ready.store(true, Ordering::SeqCst);
        Ok((
            Daemon { core, state, socket_path, _lease: lease },
            listener,
        ))
    }

    /// Graceful shutdown: refuse new work, stop every live adapter so
    /// sessions land in a recoverable stopped state, and remove the socket.
    /// The lease releases when the daemon drops.
    pub fn shutdown(&self) {
        self.state.ready.store(false, Ordering::SeqCst);
        self.state.shutting_down.store(true, Ordering::SeqCst);
        let sessions: Vec<String> =
            self.core.adapters.lock().unwrap().keys().cloned().collect();
        for session_id in sessions {
            self.core
                .stop_session_adapter(&session_id, bridge_core::adapters::ShutdownReason::AppShutdown);
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Load the per-install token, creating it (0600) on first start. The token
/// authenticates every connection's handshake.
fn ensure_token(path: &Path) -> Result<String, std::io::Error> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim().to_owned();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    options.mode(0o600);
    let mut file = options.open(path)?;
    file.write_all(token.as_bytes())?;
    file.flush()?;
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
    fn token_comparison_requires_exact_match() {
        assert!(token_matches("abc", "abc"));
        assert!(!token_matches("abc", "abd"));
        assert!(!token_matches("abc", "ab"));
        assert!(!token_matches("abc", ""));
    }
}
