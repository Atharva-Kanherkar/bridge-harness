//! The `BridgeCore` runtime: the application state re-homed out of the Tauri
//! shell. It owns both databases, the PTY session runtimes, harness adapters,
//! delegation bookkeeping, worktrees, credentials, and browser supervision.
//! Nothing in this module (or its API) may reference `tauri::` types — host
//! integration happens in the shell crate that embeds [`BridgeCore`].

use crate::{
    adapters, agent_config, binary, browser_bridge, credential_broker, delegation,
    model::AdapterDescriptor, session_supervisor, skill_marketplace, store, worker_guard,
    worker_sandbox, BridgeError,
};
use portable_pty::{Child, MasterPty};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

pub struct RuntimeSession {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
}

pub struct BridgeCore {
    pub db: Mutex<rusqlite::Connection>,
    pub telemetry_db: Mutex<rusqlite::Connection>,
    pub runtimes: Mutex<HashMap<String, RuntimeSession>>,
    pub adapters: Mutex<HashMap<String, Box<dyn adapters::AdapterRuntime>>>,
    pub adapter_registry: Arc<adapters::AdapterRegistry>,
    pub delegations: Mutex<DelegationState>,
    pub worktrees: PathBuf,
    pub database_path: PathBuf,
    pub telemetry_database_path: PathBuf,
    pub snapshot_dir: PathBuf,
    pub skill_store: PathBuf,
    pub skill_consents: Arc<Mutex<HashMap<String, skill_marketplace::SkillConsent>>>,
    pub credential_broker: Arc<credential_broker::CredentialBroker>,
    pub browser_bridge: Arc<browser_bridge::BrowserBridgeSupervisor>,
    /// Last time each session produced adapter output, used by the worker
    /// stall watchdog to detect a live-but-silent worker. Monotonic, in-memory
    /// only — process death is already handled by the reader-thread EOF path.
    pub worker_activity: Mutex<HashMap<String, std::time::Instant>>,
    /// Last heartbeat copied into `worker_runtime.updated_at` for live UI
    /// visibility. Kept separate so frequent streaming frames only write to
    /// SQLite at a bounded cadence.
    pub worker_activity_persisted: Mutex<HashMap<String, std::time::Instant>>,
}

/// A worker actively `working` that produces *no* adapter output at all for this
/// long is treated as hung. The window is deliberately generous: healthy agents
/// stream reasoning/tool frames far more often, so total silence this long is a
/// strong stall signal, while a legitimate long build/test is very unlikely to
/// emit nothing for ten minutes. Death is still caught immediately on EOF; this
/// only covers the alive-but-silent case.
pub const WORKER_STALL_TIMEOUT_SECONDS: u64 = 600;

/// Bookkeeping for the multi-agent delegation tree.
#[derive(Default)]
pub struct DelegationState {
    /// Tracks the single same-session repair allowed for malformed worker output.
    pub result_repairs: delegation::ResultRepairTracker,
    /// Last observed provider turn per session, retained until the next turn
    /// so late usage events keep the originating user-request budget key.
    pub last_turn_by_session: HashMap<String, String>,
    /// Per-session count of automatic corrective turns sent after a rejected
    /// `bridge-delegate` request, so a persistently malformed orchestrator turn
    /// cannot drive an unbounded correction loop.
    pub invalid_request_corrections: HashMap<String, u32>,
    /// Read-only worker session → tracked Git state captured before process start.
    pub read_only_baselines: HashMap<String, worker_guard::ReadOnlyBaseline>,
    /// OS-level boundary and output directory retained until the worker exits.
    pub read_only_sandboxes: HashMap<String, worker_sandbox::ReadOnlySandbox>,
}

/// Host-provided configuration for [`BridgeCore::boot`]. The host resolves
/// platform paths (data directory, bundled browser extension) and supplies
/// notification callbacks; the core owns everything that happens after.
pub struct BootConfig {
    /// Application data directory holding the databases, worktrees, skills,
    /// and history snapshots.
    pub data_dir: PathBuf,
    /// Directory containing the browser extension handed to the browser
    /// bridge supervisor.
    pub browser_extension_path: PathBuf,
    /// Fires once the background OpenCode catalog discovery finishes
    /// (successfully or not), so the host can tell the frontend to re-read
    /// adapter availability.
    pub on_opencode_discovered: Option<Box<dyn FnOnce() + Send>>,
}

impl BridgeCore {
    /// Open the stores, run recovery, build the adapter registry, and start
    /// browser supervision — everything the runtime needs before a host can
    /// serve requests against it.
    pub fn boot(config: BootConfig) -> Result<Self, BridgeError> {
        let db_path = config.data_dir.join("bridge.db");
        let telemetry_db_path = config.data_dir.join("bridge-telemetry.db");
        let snapshot_dir = config.data_dir.join("history-snapshots");
        let connection = store::open(&db_path)?;
        let telemetry_connection = store::open_telemetry(&telemetry_db_path)?;
        session_supervisor::SessionSupervisor::recover_tracked_adapter_processes(&connection)?;
        session_supervisor::SessionSupervisor::recover_orphaned_workers(&connection)?;
        session_supervisor::SessionSupervisor::reconcile_workspace_statuses(&connection)?;
        let _ = store::export_history_snapshot(&connection, &snapshot_dir);
        let opencode_config = agent_config::state(&connection)?
            .harnesses
            .into_iter()
            .find(|config| config.id == "opencode");
        let opencode_settings = agent_config::opencode_settings(opencode_config.as_ref())?;
        let adapter_registry = Arc::new(adapters::AdapterRegistry::built_in_with_opencode_notify(
            opencode_settings,
            config.on_opencode_discovered,
        )?);
        let credential_broker = Arc::new(credential_broker::CredentialBroker::openai()?);
        let browser_bridge = browser_bridge::BrowserBridgeSupervisor::start(
            config.browser_extension_path,
            config.data_dir.join("browser-site-metrics.json"),
        );
        Ok(Self {
            db: Mutex::new(connection),
            telemetry_db: Mutex::new(telemetry_connection),
            runtimes: Mutex::new(HashMap::new()),
            adapters: Mutex::new(HashMap::new()),
            adapter_registry,
            delegations: Mutex::new(DelegationState::default()),
            worktrees: config.data_dir.join("worktrees"),
            database_path: db_path,
            telemetry_database_path: telemetry_db_path,
            snapshot_dir,
            skill_store: config.data_dir.join("skills"),
            skill_consents: Arc::new(Mutex::new(HashMap::new())),
            credential_broker,
            browser_bridge,
            worker_activity: Mutex::new(HashMap::new()),
            worker_activity_persisted: Mutex::new(HashMap::new()),
        })
    }
}

pub fn start_health_server(
    database: PathBuf,
    adapters: Vec<AdapterDescriptor>,
    credential_broker: Arc<credential_broker::CredentialBroker>,
) {
    thread::spawn(move || {
        let Ok(server) = tiny_http::Server::http("127.0.0.1:4317") else {
            return;
        };
        for request in server.incoming_requests() {
            if request.url() == "/health" {
                let body = serde_json::json!({
                    "ok": true,
                    "version": env!("CARGO_PKG_VERSION"),
                    "database": database,
                    "adapters": adapters,
                    "harnesses": {
                        "claude": binary::resolve("claude").is_some(),
                        "codex": binary::resolve("codex").is_some(),
                        "opencode": binary::resolve("opencode").is_some(),
                        "shell": true
                    }
                })
                .to_string();
                let mut response = tiny_http::Response::from_string(body).with_status_code(200);
                if let Ok(header) =
                    tiny_http::Header::from_bytes("Content-Type", "application/json")
                {
                    response.add_header(header);
                }
                let _ = request.respond(response);
                continue;
            }
            if let Some(route) = request.url().strip_prefix(credential_broker::PROXY_PREFIX) {
                // Handle each proxy call on its own thread so a slow (or
                // deliberately slow-drip) upstream request cannot block /health
                // liveness or serialize other agents behind the single accept loop.
                let route = route.to_owned();
                let method = request.method().as_str().to_owned();
                let token = request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv(credential_broker::PROXY_AUTH_HEADER))
                    .map(|header| header.value.as_str().to_owned())
                    .unwrap_or_default();
                let headers: Vec<(String, String)> = request
                    .headers()
                    .iter()
                    .map(|header| (header.field.to_string(), header.value.as_str().to_owned()))
                    .collect();
                let broker = credential_broker.clone();
                thread::spawn(move || {
                    let mut request = request;
                    let mut parts = route.splitn(3, '/');
                    let session_id = parts.next().unwrap_or_default().to_owned();
                    let reference = parts.next().unwrap_or_default().to_owned();
                    let path_and_query = format!("/{}", parts.next().unwrap_or_default());
                    let mut body = Vec::new();
                    let result = request
                        .as_reader()
                        .take((credential_broker::MAX_BODY_BYTES + 1) as u64)
                        .read_to_end(&mut body)
                        .map_err(BridgeError::Io)
                        .and_then(|_| {
                            broker.proxy(credential_broker::ProxyRequest {
                                session_id,
                                reference,
                                method,
                                path_and_query,
                                headers,
                                token,
                                body,
                            })
                        });
                    let response = match result {
                        Ok(proxied) => {
                            let mut response = tiny_http::Response::from_data(proxied.body)
                                .with_status_code(proxied.status);
                            if let Some(header) = proxied.content_type.and_then(|value| {
                                tiny_http::Header::from_bytes("Content-Type", value).ok()
                            }) {
                                response.add_header(header);
                            }
                            response
                        }
                        Err(error) => {
                            let body = serde_json::json!({"ok": false, "error": error.to_string()})
                                .to_string();
                            tiny_http::Response::from_string(body).with_status_code(400)
                        }
                    };
                    let _ = request.respond(response);
                });
                continue;
            }
            let body = serde_json::json!({"ok": false, "error": "not found"}).to_string();
            let mut response = tiny_http::Response::from_string(body).with_status_code(404);
            if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
                response.add_header(header);
            }
            let _ = request.respond(response);
        }
    });
}
