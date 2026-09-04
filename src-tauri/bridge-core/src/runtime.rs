//! The `BridgeCore` runtime: the application state re-homed out of the Tauri
//! shell. It owns both databases, the PTY session runtimes, harness adapters,
//! delegation bookkeeping, worktrees, credentials, and browser supervision.
//! Nothing in this module (or its API) may reference `tauri::` types — host
//! integration happens in the shell crate that embeds [`BridgeCore`].

use crate::events::{CoreEvent, EventBus};
use crate::{
    adapters, agent_config, agent_integration, backend_binding, binary, browser_bridge,
    credential_broker, delegation, model::AdapterDescriptor, session_supervisor, skill_marketplace,
    store, suggestion_engine::SuggestionEngine, verified_catalog, worker_guard, worker_sandbox,
    BridgeError,
};
use portable_pty::{Child, MasterPty};
use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

/// The Bridge version the catalog is read against. The workspace pins one
/// version for every crate precisely so this cannot drift from the application
/// version a snapshot names.
const BRIDGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct RuntimeSession {
    pub writer: Box<dyn Write + Send>,
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    /// Which spawn this entry belongs to. A reader thread draining a dead
    /// shell may outlive a close-and-reopen of the same key; the epoch lets
    /// its cleanup recognise that the entry under the key is no longer its
    /// own and leave the newer shell alone.
    pub epoch: u64,
}

pub struct BridgeCore {
    pub db: Mutex<rusqlite::Connection>,
    pub telemetry_db: Mutex<rusqlite::Connection>,
    pub runtimes: Mutex<HashMap<String, RuntimeSession>>,
    pub adapters: Mutex<HashMap<String, Box<dyn adapters::AdapterRuntime>>>,
    pub reader_launches: Mutex<HashMap<String, Arc<Mutex<bool>>>>,
    pub adapter_registry: Arc<adapters::AdapterRegistry>,
    /// Which backend serves each agent. The registry executes; this decides
    /// what may execute, and what a session recorded last time.
    pub backend_resolver: Arc<backend_binding::BackendResolver>,
    /// The Bridge Verified catalog in force. Loaded once, at boot: it is
    /// authenticated by a signature over exact bytes, and re-verifying that on
    /// every question would be work with no answer to show for it.
    pub catalog: Arc<verified_catalog::Catalog>,
    /// What the catalog contributed to the resolver, and what it could not.
    /// Carried so a skipped entry reaches a caller rather than being a thing
    /// that quietly did not happen at boot.
    pub catalog_registration: agent_integration::CatalogRegistration,
    /// Why a cached snapshot lost to the bundled bootstrap, when one did.
    pub catalog_cache_rejected: Option<String>,
    /// Every integration compiled into this build.
    ///
    /// Empty today, and that is the point of #171: this epic ships the
    /// framework, not an agent. An empty registry means every catalog entry is
    /// skipped for `no_integration`, which is the correct behaviour for a build
    /// that carries no marketplace agent yet.
    pub integrations: Arc<agent_integration::IntegrationRegistry>,
    pub delegations: Mutex<DelegationState>,
    pub worktrees: PathBuf,
    pub database_path: PathBuf,
    pub telemetry_database_path: PathBuf,
    pub snapshot_dir: PathBuf,
    pub skill_store: PathBuf,
    pub skill_consents: Arc<Mutex<HashMap<String, skill_marketplace::SkillConsent>>>,
    pub credential_broker: Arc<credential_broker::CredentialBroker>,
    pub browser_bridge: Arc<browser_bridge::BrowserBridgeSupervisor>,
    /// Read-only `gh` CLI surface. It owns no credentials and is deliberately
    /// separate from model adapters and their sidecars.
    pub github_surface: crate::github_surface::GithubSurface,
    pub github_poller: crate::github_poll::GithubPoller,
    /// Last time each session produced adapter output, used by the worker
    /// stall watchdog to detect a live-but-silent worker. Monotonic, in-memory
    /// only — process death is already handled by the reader-thread EOF path.
    pub worker_activity: Mutex<HashMap<String, std::time::Instant>>,
    /// Last heartbeat copied into `worker_runtime.updated_at` for live UI
    /// visibility. Kept separate so frequent streaming frames only write to
    /// SQLite at a bounded cadence.
    pub worker_activity_persisted: Mutex<HashMap<String, std::time::Instant>>,
    /// The notify-only live event channel; durable history stays in SQLite.
    /// See `events.rs` for the publish-after-commit rules.
    pub events: EventBus,
    /// Sessions with an exclusive lifecycle operation in flight (adapter
    /// start, model switch), mapped to the operation name for error messages.
    /// Lifecycle flows span host-run blocking steps, so this claim — not the
    /// runtimes map — is what keeps a concurrent start from racing a
    /// teardown/commit window and orphaning a live adapter.
    pub lifecycle_claims: Mutex<HashMap<String, &'static str>>,
    /// Serializes filesystem-changing operations with session startup per
    /// workspace. A branch switch must not race an adapter launch or editor
    /// write against the same checkout.
    workspace_operations: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Discoveries this daemon has actually walked, keyed by `discovery_id`.
    /// `imports/preview_external_import` and `imports/commit_external_import`
    /// look approved roots and artifacts up here instead of trusting a
    /// client-supplied `DiscoveryResult` — a caller cannot forge an
    /// approved-root list or an artifact's source path that this process
    /// never discovered on disk.
    pub external_import_discoveries: Mutex<HashMap<String, crate::external_import::DiscoveryResult>>,
    /// The composer typeahead's warm hidden session and fallback cooldowns.
    /// See `suggestion_engine` for why this lives on `BridgeCore` rather than
    /// being started fresh per request: process-start latency on every
    /// keystroke pause would make the feature unusable.
    pub suggestion_engine: SuggestionEngine,
}

/// An exclusive per-session lifecycle claim; released on drop.
pub struct SessionLifecycleClaim<'core> {
    core: &'core BridgeCore,
    session_id: String,
}

impl std::fmt::Debug for SessionLifecycleClaim<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionLifecycleClaim")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

impl Drop for SessionLifecycleClaim<'_> {
    fn drop(&mut self) {
        self.core
            .lifecycle_claims
            .lock()
            .unwrap()
            .remove(&self.session_id);
    }
}

/// A worker actively `working` that produces *no* adapter output at all for this
/// long is treated as hung. The window is deliberately generous: healthy agents
/// stream reasoning/tool frames far more often, so total silence this long is a
/// strong stall signal, while a legitimate long build/test is very unlikely to
/// emit nothing for ten minutes. Death is still caught immediately on EOF; this
/// only covers the alive-but-silent case.
pub const WORKER_STALL_TIMEOUT_SECONDS: u64 = 600;

/// A worker parked in `waiting` on a human approval is deliberately idle, so the
/// stall watchdog skips it. That used to mean it was excluded from *every*
/// watchdog and could sit unreported forever. This is the separate approval
/// deadline: past it, the worker is resolved to a terminal typed result that
/// names the unanswered approval, which unblocks the parent.
pub const WORKER_APPROVAL_TIMEOUT_SECONDS: i64 = 30 * 60;

/// A verification attempt whose planned checks have not reached a terminal state
/// within this window is escalated to a terminal failure. Without a deadline an
/// attempt with an unrunnable check stays `verifying` forever and the parent
/// never becomes ready.
pub const COMPLETION_VERIFY_TIMEOUT_SECONDS: i64 = 45 * 60;

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
    /// A peek is emitted before the provider's separate turn-completed frame;
    /// hold it until that boundary so its reply starts a clean turn.
    pub pending_worker_peeks: HashMap<String, delegation::PeekRequest>,
    /// A steer is held for the same reason a peek is: it arrives on the
    /// assistant frame, and delivering it before the parent's turn completes
    /// would race the reply into a turn that is still running.
    pub pending_worker_steers: HashMap<String, delegation::SteerRequest>,
    /// Read-only worker session → tracked Git state captured before process start.
    pub read_only_baselines: HashMap<String, worker_guard::ReadOnlyBaseline>,
    /// OS-level boundary and output directory retained until the worker exits.
    pub read_only_sandboxes: HashMap<String, worker_sandbox::ReadOnlySandbox>,
}

/// Host-provided configuration for [`BridgeCore::boot`]. The host resolves
/// platform paths (data directory, bundled browser extension) and optionally
/// injects a pre-subscribed event bus; the core owns everything after that.
pub struct BootConfig {
    /// Application data directory holding the databases, worktrees, skills,
    /// and history snapshots.
    pub data_dir: PathBuf,
    /// Directory containing the browser extension handed to the browser
    /// bridge supervisor.
    pub browser_extension_path: PathBuf,
    /// The live event channel the runtime publishes to. Hosts subscribe
    /// before calling [`BridgeCore::boot`] and inject the bus here so
    /// boot-time events (adapter discovery completion) cannot be missed;
    /// `None` creates a fresh bus.
    pub events: Option<EventBus>,
}

impl BridgeCore {
    /// The aggregate application snapshot the frontend renders.
    pub fn state_snapshot(&self) -> Result<crate::model::BridgeState, BridgeError> {
        store::state(&self.db.lock().unwrap())
    }

    /// Flip the reader-launch gate for `session_id` so its reader thread stops
    /// processing new lines. Idempotent and safe to call from teardown paths.
    pub fn deactivate_reader_launch(&self, session_id: &str) {
        if let Some(gate) = self
            .reader_launches
            .lock()
            .unwrap()
            .get(session_id)
            .cloned()
        {
            *gate.lock().unwrap() = false;
        }
    }

    /// Claim exclusive lifecycle access to a session for the duration of the
    /// returned guard. Every flow that starts, replaces, or tears down a
    /// session's adapter runtime must hold this across its whole
    /// plan → blocking-step → commit window; a concurrent claim fails fast
    /// with the name of the operation already in flight.
    pub fn claim_session_lifecycle(
        &self,
        session_id: &str,
        operation: &'static str,
    ) -> Result<SessionLifecycleClaim<'_>, BridgeError> {
        let mut claims = self.lifecycle_claims.lock().unwrap();
        if let Some(in_flight) = claims.get(session_id) {
            return Err(BridgeError::Invalid(format!(
                "Another operation ({in_flight}) is already in progress for this session; try again once it finishes"
            )));
        }
        claims.insert(session_id.to_owned(), operation);
        Ok(SessionLifecycleClaim {
            core: self,
            session_id: session_id.to_owned(),
        })
    }

    pub(crate) fn workspace_operation(&self, workspace_id: &str) -> Arc<Mutex<()>> {
        Arc::clone(
            self.workspace_operations
                .lock()
                .unwrap()
                .entry(workspace_id.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    /// A runtime around in-memory stores with no adapters, no discovery, and
    /// a dormant browser supervisor — for exercising domain methods in tests.
    #[cfg(test)]
    pub(crate) fn for_tests(scratch: &std::path::Path) -> BridgeCore {
        BridgeCore {
            db: Mutex::new(store::open(std::path::Path::new(":memory:")).unwrap()),
            telemetry_db: Mutex::new(
                store::open_telemetry(std::path::Path::new(":memory:")).unwrap(),
            ),
            runtimes: Mutex::new(HashMap::new()),
            adapters: Mutex::new(HashMap::new()),
            reader_launches: Mutex::new(HashMap::new()),
            adapter_registry: Arc::new(adapters::AdapterRegistry::empty()),
            backend_resolver: Arc::new(backend_binding::BackendResolver::built_in()),
            catalog: Arc::new(
                verified_catalog::Catalog::bundled(BRIDGE_VERSION)
                    .expect("the bundled bootstrap catalog must load"),
            ),
            catalog_registration: agent_integration::CatalogRegistration::default(),
            catalog_cache_rejected: None,
            integrations: Arc::new(agent_integration::IntegrationRegistry::empty()),
            delegations: Mutex::new(DelegationState::default()),
            worktrees: scratch.join("worktrees"),
            database_path: scratch.join("bridge.db"),
            telemetry_database_path: scratch.join("bridge-telemetry.db"),
            snapshot_dir: scratch.join("history-snapshots"),
            skill_store: scratch.join("skills"),
            skill_consents: Arc::new(Mutex::new(HashMap::new())),
            credential_broker: Arc::new(credential_broker::CredentialBroker::openai().unwrap()),
            browser_bridge: browser_bridge::BrowserBridgeSupervisor::dormant(
                scratch.join("no-extension"),
                scratch.join("browser-site-metrics.json"),
            ),
            github_surface: crate::github_surface::GithubSurface::unavailable_for_tests(),
            github_poller: crate::github_poll::GithubPoller::default(),
            worker_activity: Mutex::new(HashMap::new()),
            worker_activity_persisted: Mutex::new(HashMap::new()),
            events: EventBus::new(),
            lifecycle_claims: Mutex::new(HashMap::new()),
            workspace_operations: Mutex::new(HashMap::new()),
            external_import_discoveries: Mutex::new(HashMap::new()),
            suggestion_engine: SuggestionEngine::new(),
        }
    }

    /// Open the stores, run recovery, build the adapter registry, and start
    /// browser supervision — everything the runtime needs before a host can
    /// serve requests against it.
    pub fn boot(config: BootConfig) -> Result<Self, BridgeError> {
        // Managed payloads live under the leased data directory, registered here
        // rather than in each host so a future host cannot forget to do it. Until
        // this runs, every adapter's managed tier resolves to nothing and the
        // agents behave exactly as they did before managed payloads existed.
        crate::managed_runtime::register_managed_root(config.data_dir.join("managed-runtimes"));
        let db_path = config.data_dir.join("bridge.db");
        let telemetry_db_path = config.data_dir.join("bridge-telemetry.db");
        let snapshot_dir = config.data_dir.join("history-snapshots");
        let connection = store::open(&db_path)?;
        let telemetry_connection = store::open_telemetry(&telemetry_db_path)?;
        session_supervisor::SessionSupervisor::recover_tracked_adapter_processes(&connection)?;
        // Children beyond the per-session claims — discovery and control
        // servers above all — are reaped from the durable launch ledger, and
        // pre-ledger opencode orphans by the one-time sweep. Both fail closed
        // and neither may abort boot: a stuck foreign process is not this
        // instance's failure.
        let ledger_root = config.data_dir.join("process-ledger");
        crate::process_ledger::register_ledger_root(&ledger_root);
        crate::claude_adapter::register_node_compile_cache_root(
            config.data_dir.join("node-compile-cache"),
        );
        let _ = crate::process_ledger::recover_in_dir(&connection, &ledger_root);
        let _ = crate::process_ledger::sweep_legacy_opencode_orphans(&connection);
        session_supervisor::SessionSupervisor::recover_orphaned_workers(&connection)?;
        // Adoption state must survive restart: a pending row whose worktree is
        // gone would otherwise block its parent forever.
        crate::worker_adoption::recover(&connection)?;
        session_supervisor::SessionSupervisor::reconcile_workspace_statuses(&connection)?;
        // No history snapshot here on purpose. `VACUUM INTO` plus a full-file
        // hash grows with total history, and running it before the daemon
        // bound its socket pushed readiness past the desktop shell's start
        // deadline on a large history, so the app fell back to the embedded
        // host. `live_turn::start_history_snapshot_maintenance` takes the
        // first, staleness-gated export as soon as the host is serving.
        let opencode_config = agent_config::state(&connection)?
            .harnesses
            .into_iter()
            .find(|config| config.id == "opencode");
        let opencode_settings = agent_config::opencode_settings(opencode_config.as_ref())?;
        let events = config.events.unwrap_or_default();
        // OpenCode discovery finishes after boot returns; publish the refetch
        // hint so subscribed hosts re-read adapter availability.
        let discovery_events = events.clone();
        let adapter_registry = Arc::new(adapters::AdapterRegistry::built_in_with_opencode_notify(
            opencode_settings,
            Some(Box::new(move || {
                discovery_events.publish(CoreEvent::AdaptersChanged)
            })),
        )?);
        let credential_broker = Arc::new(credential_broker::CredentialBroker::openai()?);

        // The catalog, and what it contributes to resolution. `load` never
        // fails for a bad cache — a corrupt or withdrawn snapshot loses to the
        // bundled bootstrap — so the only error here is the compiled-in
        // bootstrap failing its own validation, which is a build defect rather
        // than anything a user or a publisher can cause.
        let catalog_store =
            verified_catalog::CatalogStore::new(config.data_dir.join("verified-catalog"));
        let loaded = catalog_store
            .load(&verified_catalog::TrustRoot::production(), BRIDGE_VERSION)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let integrations = Arc::new(agent_integration::IntegrationRegistry::empty());
        let mut backend_resolver = backend_binding::BackendResolver::built_in();
        let catalog_registration =
            integrations.offer_catalog(&loaded.catalog, &mut backend_resolver);

        let browser_bridge = browser_bridge::BrowserBridgeSupervisor::start(
            config.browser_extension_path,
            config.data_dir.join("browser-site-metrics.json"),
        );
        Ok(Self {
            db: Mutex::new(connection),
            telemetry_db: Mutex::new(telemetry_connection),
            runtimes: Mutex::new(HashMap::new()),
            adapters: Mutex::new(HashMap::new()),
            reader_launches: Mutex::new(HashMap::new()),
            adapter_registry,
            backend_resolver: Arc::new(backend_resolver),
            catalog: Arc::new(loaded.catalog),
            catalog_registration,
            catalog_cache_rejected: loaded.cache_rejected.map(|error| error.code().to_owned()),
            integrations,
            delegations: Mutex::new(DelegationState::default()),
            worktrees: config.data_dir.join("worktrees"),
            database_path: db_path,
            telemetry_database_path: telemetry_db_path,
            snapshot_dir,
            skill_store: config.data_dir.join("skills"),
            skill_consents: Arc::new(Mutex::new(HashMap::new())),
            credential_broker,
            browser_bridge,
            github_surface: crate::github_surface::GithubSurface::discover(),
            github_poller: crate::github_poll::GithubPoller::default(),
            worker_activity: Mutex::new(HashMap::new()),
            worker_activity_persisted: Mutex::new(HashMap::new()),
            events,
            lifecycle_claims: Mutex::new(HashMap::new()),
            workspace_operations: Mutex::new(HashMap::new()),
            external_import_discoveries: Mutex::new(HashMap::new()),
            suggestion_engine: SuggestionEngine::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_config;
    use std::path::Path;
    use std::time::Duration;

    /// Point OpenCode at a nonexistent executable so background discovery
    /// fails immediately instead of spawning a real OpenCode server; the
    /// completion callback still fires on the failure path.
    fn seed_fast_failing_opencode(db: &rusqlite::Connection, data_dir: &Path) {
        let mut opencode = agent_config::state(db)
            .unwrap()
            .harnesses
            .into_iter()
            .find(|harness| harness.id == "opencode")
            .unwrap();
        opencode.advanced = serde_json::json!({
            "executablePath": data_dir.join("missing-opencode").to_string_lossy(),
        });
        agent_config::save_harness(db, opencode).unwrap();
    }

    fn seeded_config(data_dir: &Path) -> BootConfig {
        let db = crate::store::open(&data_dir.join("bridge.db")).unwrap();
        seed_fast_failing_opencode(&db, data_dir);
        BootConfig {
            data_dir: data_dir.to_path_buf(),
            browser_extension_path: data_dir.join("no-extension"),
            events: None,
        }
    }

    #[test]
    fn boot_prepares_stores_and_derived_paths_under_the_data_dir() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        let core = BridgeCore::boot(seeded_config(data_dir)).unwrap();

        assert!(data_dir.join("bridge.db").is_file());
        assert!(data_dir.join("bridge-telemetry.db").is_file());
        assert_eq!(core.database_path, data_dir.join("bridge.db"));
        assert_eq!(
            core.telemetry_database_path,
            data_dir.join("bridge-telemetry.db")
        );
        assert_eq!(core.worktrees, data_dir.join("worktrees"));
        assert_eq!(core.snapshot_dir, data_dir.join("history-snapshots"));
        assert_eq!(core.skill_store, data_dir.join("skills"));
        assert!(core.runtimes.lock().unwrap().is_empty());
        assert!(core.adapters.lock().unwrap().is_empty());
        assert!(core
            .delegations
            .lock()
            .unwrap()
            .last_turn_by_session
            .is_empty());
        // Both stores must be usable connections, not just files on disk.
        let sessions: i64 = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sessions, 0);
    }

    #[test]
    fn boot_leaves_the_history_snapshot_export_to_the_maintenance_thread() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        let _core = BridgeCore::boot(seeded_config(data_dir)).unwrap();
        // Readiness must never wait on an O(history) `VACUUM INTO`: the first
        // export belongs to `live_turn::start_history_snapshot_maintenance`.
        assert!(!data_dir.join("history-snapshots").exists());
    }

    #[test]
    fn boot_serves_the_bundled_catalog_and_leaves_the_built_ins_resolving() {
        let fixture = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::boot(seeded_config(fixture.path())).unwrap());
        let catalog = crate::api::verified_catalog(&core);

        assert!(catalog.provenance.bundled);
        assert_eq!(catalog.generation, 1);
        assert!(
            catalog.entries.is_empty(),
            "#171 says this epic ships the framework, not an agent"
        );
        assert!(
            catalog.cache_rejected.is_none(),
            "there is no cache to refuse"
        );
        assert!(catalog.registration.registered.is_empty());

        // And the three agents Bridge already had still resolve. A catalog that
        // contributes nothing must also take nothing away.
        for agent in ["claude", "codex", "opencode"] {
            let agent = bridge_protocol::messages::AgentId::parse(agent).unwrap();
            assert!(
                core.backend_resolver.preferred(&agent).is_ok(),
                "{agent:?} must still resolve"
            );
        }
    }

    #[test]
    fn boot_refuses_a_cache_this_build_cannot_authenticate_and_serves_the_bootstrap() {
        // What a shipped build does today: the trust root is empty, so no
        // cached snapshot can be authenticated and the bootstrap is what
        // serves. Fail-closed, exercised through the real boot path rather
        // than asserted about it.
        let fixture = tempfile::tempdir().unwrap();
        let cache = fixture.path().join("verified-catalog");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(
            cache.join(verified_catalog::SNAPSHOT_FILE),
            br#"{"schemaVersion":1,"generation":9,"publishedAt":"2026-08-15T00:00:00Z","minimumBridgeVersion":"0.1.0","entries":[]}"#,
        )
        .unwrap();
        std::fs::write(
            cache.join(verified_catalog::SIGNATURE_FILE),
            br#"{"keyId":"whoever","signature":"AAAA","installedAt":"2026-08-15T00:00:00Z"}"#,
        )
        .unwrap();

        let core = Arc::new(BridgeCore::boot(seeded_config(fixture.path())).unwrap());
        let catalog = crate::api::verified_catalog(&core);
        assert!(
            catalog.provenance.bundled,
            "an unauthenticated cache must lose to the compiled-in bootstrap"
        );
        assert_eq!(
            catalog.cache_rejected,
            Some("unknown_key"),
            "and the reason must reach a caller rather than vanish at boot"
        );
    }

    #[test]
    fn boot_fails_when_the_data_dir_is_unusable() {
        let fixture = tempfile::tempdir().unwrap();
        let not_a_dir = fixture.path().join("occupied");
        std::fs::write(&not_a_dir, b"file, not a directory").unwrap();
        let result = BridgeCore::boot(BootConfig {
            data_dir: not_a_dir,
            browser_extension_path: fixture.path().join("no-extension"),
            events: None,
        });
        assert!(result.is_err());
    }

    /// The boot wiring for the launch ledger: a record whose supervisor is
    /// dead is reaped during `BridgeCore::boot`, exactly as an interrupted
    /// discovery or SIGKILLed daemon leaves it.
    #[cfg(unix)]
    #[test]
    fn boot_reaps_ledgered_children_of_dead_supervisors() {
        // Boot re-registers the process-wide managed root; hold the shared
        // lock so tests that count walks under that root are not perturbed.
        let _managed_root_guard = crate::managed_runtime::MANAGED_ROOT_TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        {
            let db = crate::store::open(&data_dir.join("bridge.db")).unwrap();
            seed_fast_failing_opencode(&db, data_dir);
        }
        let ledger_root = data_dir.join("process-ledger");
        let mut command = std::process::Command::new("sleep");
        command.arg("30");
        crate::adapters::configure_process_group(&mut command);
        let mut child = command.spawn().unwrap();
        let guard = crate::process_ledger::record_launch_in_dir(
            &ledger_root,
            "opencode.control",
            "boot test",
            child.id(),
        );
        std::mem::forget(guard);
        let record_path = std::fs::read_dir(&ledger_root)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        let mut record: crate::process_ledger::LaunchRecord =
            serde_json::from_slice(&std::fs::read(&record_path).unwrap()).unwrap();
        record.supervisor_pid = u32::MAX - 1;
        record.supervisor_identity = "a supervisor that no longer exists".into();
        std::fs::write(&record_path, serde_json::to_vec(&record).unwrap()).unwrap();

        let core = BridgeCore::boot(BootConfig {
            data_dir: data_dir.to_path_buf(),
            browser_extension_path: data_dir.join("no-extension"),
            events: None,
        })
        .unwrap();

        let status = child.wait().unwrap();
        assert!(!status.success(), "boot must terminate the abandoned child");
        assert!(!record_path.exists(), "the handled record is cleared");
        let db = core.db.lock().unwrap();
        let killed: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM events WHERE kind='process.orphan_killed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(killed, 1, "recovery evidence is durable");
    }

    #[cfg(unix)]
    #[test]
    fn boot_recovers_orphaned_adapter_processes_then_reconciles_workspaces() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        let mut orphan = {
            let db = crate::store::open(&data_dir.join("bridge.db")).unwrap();
            seed_fast_failing_opencode(&db, data_dir);
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/boot-test','now')", []).unwrap();
            db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/boot-test-w','working','now')", []).unwrap();
            db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,active_turn_id) VALUES('s','w','codex','Session','working','reported','turn')", []).unwrap();
            let mut command = std::process::Command::new("sleep");
            command.arg("30");
            crate::adapters::configure_process_group(&mut command);
            let child = command.spawn().unwrap();
            crate::session_supervisor::SessionSupervisor::track_adapter_process(
                &db,
                "s",
                child.id(),
            )
            .unwrap();
            child
        };

        let core = BridgeCore::boot(BootConfig {
            data_dir: data_dir.to_path_buf(),
            browser_extension_path: data_dir.join("no-extension"),
            events: None,
        })
        .unwrap();
        let _ = orphan.wait();

        let db = core.db.lock().unwrap();
        let (pid, status): (Option<i64>, String) = db
            .query_row(
                "SELECT adapter_pid,status FROM sessions WHERE id='s'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(pid, None, "boot must clear tracked orphan PIDs");
        // store::open already marks live sessions stopped before recovery runs,
        // so the orphaned session lands on 'stopped' rather than 'failed'.
        assert_eq!(status, "stopped");
        assert!(db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE entity_id='s' AND kind='adapter.orphan_killed')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        // Reconciliation runs last: the workspace seeded as 'working' must end
        // 'ready' because its only session is stopped, not live.
        let workspace: String = db
            .query_row("SELECT status FROM workspaces WHERE id='w'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(workspace, "ready");
    }

    #[test]
    fn boot_publishes_adapters_changed_once_discovery_completes() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        let mut config = seeded_config(data_dir);
        // Hosts subscribe before boot so the discovery event cannot be missed.
        let bus = crate::events::EventBus::new();
        let mut receiver = bus.subscribe();
        config.events = Some(bus);
        let _core = BridgeCore::boot(config).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match receiver.try_recv() {
                Ok(event) => {
                    assert!(matches!(event, CoreEvent::AdaptersChanged), "{:?}", event.kind());
                    break;
                }
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!(
                    "discovery completion must publish adapters-changed even when discovery fails: {error}"
                ),
            }
        }
    }

    #[test]
    fn boot_reopens_an_existing_database_without_disturbing_rows() {
        let fixture = tempfile::tempdir().unwrap();
        let data_dir = fixture.path();
        {
            let db = crate::store::open(&data_dir.join("bridge.db")).unwrap();
            seed_fast_failing_opencode(&db, data_dir);
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/boot-reopen','now')", []).unwrap();
        }
        let core = BridgeCore::boot(BootConfig {
            data_dir: data_dir.to_path_buf(),
            browser_extension_path: data_dir.join("no-extension"),
            events: None,
        })
        .unwrap();
        let name: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT name FROM projects WHERE id='p'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(name, "Demo");
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
                        "codex": crate::codex_adapter::resolve_runtime().is_some(),
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
