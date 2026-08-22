//! The host-agnostic body of every protocol method.
//!
//! One function per method in the `bridge-protocol` registry, all blocking and
//! all Tauri-free. Hosts own only their transport concerns: the Tauri shell
//! decodes invoke arguments and places calls on its blocking pool so nothing
//! runs on the macOS UI thread; the `bridged` daemon deserializes contracted
//! params and calls these on its per-connection threads. Neither host may
//! carry method logic of its own — a body that exists twice will drift.
//!
//! Events: every function publishes on `core.events` (after the corresponding
//! DB commit, per the event contract) — never through a host event system.

use crate::events::CoreEvent;
use crate::model::{
    AdapterDescriptor, AgentEvent, BridgeState, CapabilityTier, Harness, SessionForestSnapshot,
};
use crate::{
    adapters, agent, agent_config, agent_integration, binary, browser_bridge, completion, git,
    learning_job, learning_router, live_turn, marketplace, memory_ledger, model_profiles, opencode_adapter,
    secret_interception, session_recall, session_supervisor, sessions, skill_marketplace, slash, store,
    verification_pipeline, verified_catalog, work, work_actions, work_observation, work_reconcile,
    work_task_state, worker_adoption,
    worker_lifecycle, workspace_files, BridgeCore, BridgeError, RuntimeSession,
};
use bridge_protocol::messages as wire;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

// --- health / state ----------------------------------------------------------

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    pub version: &'static str,
    pub harnesses: HashMap<&'static str, bool>,
    pub database: String,
    pub telemetry_database: String,
    pub snapshot_directory: String,
    pub adapters: Vec<AdapterDescriptor>,
}

pub fn health(core: &Arc<BridgeCore>) -> Result<Health, BridgeError> {
    let adapters = core.adapter_registry.descriptors();
    let opencode_available = adapters
        .iter()
        .find(|adapter| adapter.id == "opencode")
        .is_some_and(|adapter| adapter.available);
    Ok(Health {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
        harnesses: HashMap::from([
            ("claude", binary::resolve("claude").is_some()),
            ("codex", crate::codex_adapter::resolve_runtime().is_some()),
            ("opencode", opencode_available),
            ("shell", true),
        ]),
        database: core.database_path.to_string_lossy().into(),
        telemetry_database: core.telemetry_database_path.to_string_lossy().into(),
        snapshot_directory: core.snapshot_dir.to_string_lossy().into(),
        adapters,
    })
}

pub fn get_state(core: &Arc<BridgeCore>) -> Result<BridgeState, BridgeError> {
    core.state_snapshot()
}

// --- verified catalog --------------------------------------------------------

/// The Bridge Verified catalog in force, and how it came to be trusted.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifiedCatalog<'core> {
    pub generation: u64,
    pub provenance: &'core verified_catalog::Provenance,
    pub entries: &'core [verified_catalog::VerifiedEntry],
    /// What the catalog contributed to backend resolution, and which entries
    /// this build could not serve.
    pub registration: &'core agent_integration::CatalogRegistration,
    /// The refusal code if a cached snapshot lost to the bundled bootstrap at
    /// boot. `None` when the cache was used or there was none.
    pub cache_rejected: Option<&'core str>,
}

/// Read the catalog this build is serving.
///
/// Deliberately **not** a protocol method. #164 lands the catalog itself, and
/// putting it on the wire belongs with the marketplace screen that consumes it —
/// a DTO shaped now, with no reader, would be a contract written against a
/// caller nobody has seen. Its absence from the registry is a decision, and this
/// function is where that decision stops costing anything: the day the screen
/// exists, its method body is one line.
pub fn verified_catalog(core: &Arc<BridgeCore>) -> VerifiedCatalog<'_> {
    VerifiedCatalog {
        generation: core.catalog.generation(),
        provenance: core.catalog.provenance(),
        entries: core.catalog.entries(),
        registration: &core.catalog_registration,
        cache_rejected: core.catalog_cache_rejected.as_deref(),
    }
}

/// What the current conformance suite checks, and which revision it is.
///
/// The desktop never runs the suite — #168's pipeline runs where a candidate
/// can be installed in a clean environment and where vendor credentials live.
/// What the desktop can usefully answer is *what a verdict means*: a UI showing
/// "Bridge Verified" should be able to say what was checked to earn it, without
/// the marketplace screen restating a list that would then drift from the one
/// the suite actually runs.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSuite {
    pub suite_version: u32,
    pub checks: Vec<&'static str>,
    /// How many times the required set must agree with itself before a verdict
    /// is trusted.
    pub determinism_runs: usize,
}

pub fn verification_suite() -> VerificationSuite {
    VerificationSuite {
        suite_version: verification_pipeline::SUITE_VERSION,
        checks: verification_pipeline::CheckId::ALL
            .iter()
            .map(|check| check.as_str())
            .collect(),
        determinism_runs: verification_pipeline::DETERMINISM_RUNS,
    }
}

// --- projects / workspaces ---------------------------------------------------

pub fn add_project(core: &Arc<BridgeCore>, path: &str) -> Result<BridgeState, BridgeError> {
    core.add_project(path)
}

pub fn create_workspace(core: &Arc<BridgeCore>, title: &str) -> Result<BridgeState, BridgeError> {
    core.create_workspace(title)
}

pub fn connect_workspace_folder(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    path: &str,
) -> Result<BridgeState, BridgeError> {
    core.connect_workspace_folder(workspace_id, path)
}

/// List the current chat's workspace files for the composer's `@file`
/// autocomplete. Returns an empty list for chats with no connected folder.
pub fn list_workspace_files(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<Vec<String>, BridgeError> {
    match core.session_workspace_root(session_id) {
        Some(root) => workspace_files::list_files(&root),
        None => Ok(Vec::new()),
    }
}

/// List a workspace's files for the editor's tree and file palette. Same
/// `git ls-files` view as the composer's `@file` autocomplete, so the two
/// surfaces never disagree about what "the project" is.
pub fn list_workspace_tree(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<Vec<String>, BridgeError> {
    let path = core.workspace_path(workspace_id)?;
    workspace_files::list_files(Path::new(&path))
}

/// Read one workspace file for the editor. Resolves the path under the lock,
/// then reads outside it, same as [`workspace_changes`].
pub fn read_workspace_file(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    path: &str,
) -> Result<workspace_files::FileContents, BridgeError> {
    let root = core.workspace_path(workspace_id)?;
    workspace_files::read_file(Path::new(&root), path)
}

/// Write one workspace file, refusing the write if it changed on disk since
/// the editor read it. Returns the new content hash.
pub fn write_workspace_file(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    path: &str,
    content: &str,
    base_sha256: Option<&str>,
) -> Result<workspace_files::WriteOutcome, BridgeError> {
    let root = core.workspace_path(workspace_id)?;
    workspace_files::write_file(Path::new(&root), path, content, base_sha256)
}

pub fn refresh_workspace(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<BridgeState, BridgeError> {
    // Resolve the path under the lock, but run Git entirely outside it so a
    // slow status scan cannot delay message submission or streaming writes.
    let path = core.workspace_path(workspace_id)?;
    let stats = git::stats(Path::new(&path))?;
    core.record_workspace_git_stats(workspace_id, stats)
}

/// The workspace's uncommitted changeset for the importance-first review UI:
/// every path that differs from `HEAD`, tracked or not, each with a unified
/// diff and an importance badge. Resolves the path under the lock, then runs
/// Git entirely outside it, same as [`refresh_workspace`].
pub fn workspace_changes(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<git::WorkspaceChangeset, BridgeError> {
    let path = core.workspace_path(workspace_id)?;
    git::workspace_changeset(Path::new(&path))
}

pub fn archive_workspace(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<BridgeState, BridgeError> {
    // The core publishes state-changed as soon as the archive commits, so a
    // snapshot failure below cannot leave listeners unaware of it.
    core.archive_workspace(workspace_id)?;
    core.state_snapshot()
}

// --- sessions ----------------------------------------------------------------

pub fn get_session_forest(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    // Git may be slow on large repositories or during index contention. Never
    // call this while holding the global SQLite lock.
    let repository_path = core.session_repository_path(session_id)?;
    let repository_state = match repository_path {
        Some(path) => store::repository_state_for_path(&path),
        None => serde_json::json!({"status":"unavailable"}),
    };
    core.session_forest_snapshot_with_repository_state(session_id, repository_state)
}

/// Replay durable session events after a cursor — the recovery half of the
/// notify-then-replay event contract.
pub fn replay_session_events(
    core: &Arc<BridgeCore>,
    session_id: &str,
    after_sequence: i64,
    limit: Option<u32>,
) -> Result<Vec<AgentEvent>, BridgeError> {
    core.replay_session_events(session_id, after_sequence, limit)
}

pub fn activate_session_entry(
    core: &Arc<BridgeCore>,
    session_id: &str,
    entry_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    // The core publishes state-changed once the head move is recorded.
    core.activate_session_entry(session_id, entry_id)
}

pub fn create_chat(
    core: &Arc<BridgeCore>,
    harness: &Harness,
    model: Option<&str>,
    title: Option<&str>,
) -> Result<BridgeState, BridgeError> {
    core.create_chat(harness, model, title)
}

/// Create an orchestrator session inside a workspace (the classic Bridge agent
/// that plans and delegates to workers). Multiple are allowed per workspace.
pub fn create_workspace_session(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    create_worktree: bool,
) -> Result<BridgeState, BridgeError> {
    let plan = core.plan_workspace_session(workspace_id, create_worktree)?;
    let worktree = match plan.worktree_source().map(str::to_owned) {
        Some(source) => Some(sessions::prepare_orchestrator_worktree(
            &core.worktrees,
            plan.workspace_title(),
            Path::new(&source),
            plan.session_id(),
        )?),
        None => None,
    };
    core.persist_workspace_session(plan, worktree)
}

pub fn start_session(
    core: &Arc<BridgeCore>,
    workspace_id: String,
    harness: Option<Harness>,
    model: Option<String>,
) -> Result<BridgeState, BridgeError> {
    live_turn::start_session(core, workspace_id, harness, model)
}

/// Start (or hot-return) a session by id. A `direct` chat runs the stored
/// harness/model with no briefing; an `orchestrator` session runs codex with
/// the routing briefing + delegation protocol.
pub fn start_chat(core: &Arc<BridgeCore>, session_id: String) -> Result<BridgeState, BridgeError> {
    live_turn::start_chat(core, session_id)
}

/// Change a root chat's provider/model. Stops any running adapter so the next
/// message starts a fresh provider session with the explicit user selection.
pub fn update_chat_model(
    core: &Arc<BridgeCore>,
    session_id: &str,
    harness: &Harness,
    model: Option<&str>,
) -> Result<BridgeState, BridgeError> {
    // Exclusive for the whole plan -> teardown -> commit window: a concurrent
    // start would otherwise slip in after teardown and be orphaned by the
    // commit clearing its process and turn state.
    let _lifecycle = core.claim_session_lifecycle(session_id, "model switch")?;
    let Some(change) = core.plan_chat_model_change(session_id, harness, model)? else {
        return core.state_snapshot();
    };
    core.stop_session_adapter(session_id, adapters::ShutdownReason::Replaced);
    // The core publishes the durable agent event when the commit lands.
    core.commit_chat_model_change(change)?;
    core.state_snapshot()
}

pub fn prepare_turn(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<secret_interception::SanitizedTurn, BridgeError> {
    live_turn::prepare_turn(core, session_id, text)
}

pub fn send_turn(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<(), BridgeError> {
    live_turn::send_turn(core, session_id, text)
}

/// Submit user input and let Bridge decide what to do with it: start a turn,
/// steer the one already running, or durably queue it for the next phase
/// boundary. The disposition comes back so the client can say which happened.
pub fn submit_input(
    core: &Arc<BridgeCore>,
    session_id: String,
    text: String,
) -> Result<wire::SubmitInputResult, BridgeError> {
    live_turn::submit_input(core, session_id, text)
}

pub fn compact_session(core: &Arc<BridgeCore>, session_id: &str) -> Result<(), BridgeError> {
    let prompt = core.begin_manual_compaction(session_id)?;
    live_turn::send_internal_checkpoint_turn(core, session_id, &prompt)
}

pub fn search_session_entries(
    core: &Arc<BridgeCore>,
    session_id: &str,
    query: &str,
    limit: Option<u32>,
) -> Result<bridge_protocol::messages::SearchSessionEntriesResult, BridgeError> {
    let db = core.db.lock().unwrap();
    session_recall::search(&db, session_id, query, limit)
}

pub fn save_memory_record(
    core: &Arc<BridgeCore>,
    body: &str,
    kind: Option<&str>,
    session_id: Option<&str>,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::save(&db, body, kind, session_id)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

pub fn list_memory_records(
    core: &Arc<BridgeCore>,
    scope_key: &str,
) -> Result<bridge_protocol::messages::ListMemoryRecordsResult, BridgeError> {
    let db = core.db.lock().unwrap();
    memory_ledger::list(&db, scope_key)
}

pub fn delete_memory_record(
    core: &Arc<BridgeCore>,
    record_id: &str,
) -> Result<bridge_protocol::messages::MemoryRecord, BridgeError> {
    let record = {
        let db = core.db.lock().unwrap();
        memory_ledger::forget(&db, record_id)?
    };
    core.events.publish(CoreEvent::MemoryChanged {
        scope_key: record.scope_key.clone(),
    });
    Ok(record)
}

/// What memory exists here, so the UI can be honest about what it does not
/// own. The provider half is derived from the slash catalog: an unavailable
/// adapter contributes nothing, and no harness name is compared in this body.
pub fn get_memory_capabilities(
    core: &Arc<BridgeCore>,
) -> Result<wire::MemoryCapabilities, BridgeError> {
    let provider_native = slash::provider_memory_commands(&available_adapter_ids(core))
        .into_iter()
        .map(|command| wire::ProviderMemoryCommand {
            harness: command.harness,
            command: command.name,
            description: command.description,
        })
        .collect();
    Ok(wire::MemoryCapabilities {
        ledger: wire::MemoryLedgerCapability {
            exists: true,
            scope_key: wire::ACCOUNT_MEMORY_SCOPE.to_string(),
            max_body_chars: wire::MAX_MEMORY_BODY_CHARS as u32,
            kinds: wire::MEMORY_KINDS.iter().map(|kind| kind.to_string()).collect(),
        },
        provider_native,
    })
}

/// Run a finished worker's objective again because the user asked. Goes through
/// the ordinary launch path, so every policy limit applies as it did the first
/// time.
pub fn retry_worker_task(
    core: &Arc<BridgeCore>,
    child_session_id: &str,
) -> Result<(), BridgeError> {
    live_turn::retry_worker_task(core, child_session_id)
}

pub fn interrupt_turn(core: &Arc<BridgeCore>, session_id: &str) -> Result<(), BridgeError> {
    core.interrupt_turn(session_id)
}

/// Refresh subscription usage for every provider, independent of which session
/// is on screen. Results are broadcast on the `account-usage` channel.
pub fn refresh_account_usage(core: &Arc<BridgeCore>) -> Result<(), BridgeError> {
    core.refresh_account_usage()
}

pub fn stop_session(
    core: &Arc<BridgeCore>,
    session_id: String,
) -> Result<BridgeState, BridgeError> {
    live_turn::stop_session(core, session_id)
}

// --- approvals ---------------------------------------------------------------

pub fn resolve_approval(
    core: &Arc<BridgeCore>,
    session_id: &str,
    event_id: i64,
    decision: &str,
) -> Result<(), BridgeError> {
    if !matches!(
        decision,
        "accept" | "acceptForSession" | "decline" | "cancel"
    ) {
        return Err(BridgeError::Invalid("Unsupported approval decision".into()));
    }
    let db = core.db.lock().unwrap();
    let (data, adapter_id): (String, String) = db.query_row(
        "SELECT e.payload,s.harness FROM session_entries e
         JOIN sessions s ON s.id=e.session_id
         WHERE e.session_id=?1 AND e.sequence=?2 AND e.kind='approval.requested'",
        params![session_id, event_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let data: Value = serde_json::from_str(&data)
        .map_err(|e| BridgeError::Invalid(format!("Approval metadata is invalid: {e}")))?;
    // Two storage shapes reach this reader: adapter approvals persist through
    // the agent-event envelope (details nested under `data`), while policy
    // approvals are typed forest entries with top-level fields. Look in the
    // envelope's `data` first and fall back to the top level.
    let detail = |name: &str| {
        data.get("data")
            .and_then(|nested| nested.get(name))
            .or_else(|| data.get(name))
            .cloned()
    };
    if detail("approvalType").as_ref().and_then(Value::as_str) == Some("delegation_path_scope") {
        let resolved = live_turn::resolve_policy_delegation_approval(
            &db, session_id, event_id, decision, &data,
        )?;
        drop(db);
        if !resolved.accepted {
            live_turn::report_delegation_approval_declined(
                core,
                session_id,
                &resolved.turn_id,
                &resolved.approval_id,
                decision,
                &resolved.request,
            );
            return Ok(());
        }
        match live_turn::launch_worker_outcome(
            core,
            session_id,
            &resolved.turn_id,
            &resolved.request,
            true,
        ) {
            live_turn::WorkerLaunchOutcome::Launched(child_session_id) => {
                live_turn::report_approved_launch_adopted(
                    core,
                    session_id,
                    &resolved.turn_id,
                    &resolved.approval_id,
                    Some(&child_session_id),
                    false,
                );
            }
            live_turn::WorkerLaunchOutcome::Queued => {
                live_turn::report_approved_launch_adopted(
                    core,
                    session_id,
                    &resolved.turn_id,
                    &resolved.approval_id,
                    None,
                    true,
                );
            }
            // An approved scope that still routes to approval would loop the
            // user; treat it as a launch failure so the turn terminates.
            live_turn::WorkerLaunchOutcome::AwaitingApproval
            | live_turn::WorkerLaunchOutcome::Failed => {
                let db = core.db.lock().unwrap();
                live_turn::record_approved_launch_failure(
                    &db,
                    session_id,
                    &resolved.turn_id,
                    &resolved.request,
                )?;
                drop(db);
                core.events.publish(CoreEvent::StateChanged);
                return Err(BridgeError::Invalid(
                    "Write scope was approved, but the worker could not launch; the delegation may be retried for this turn".into(),
                ));
            }
        }
        core.events.publish(CoreEvent::StateChanged);
        return Ok(());
    }
    let request_id = detail("requestId")
        .ok_or_else(|| BridgeError::Invalid("Approval has no adapter request id".into()))?;
    let is_worker = store::worker_runtime(&db, session_id)?.is_some();
    if is_worker {
        session_supervisor::SessionSupervisor::transition(
            &db,
            session_id,
            worker_lifecycle::WorkerLifecycleState::Working,
            Some("approval_resolved"),
        )?;
    }
    drop(db);
    let adapters = core.adapters.lock().unwrap();
    let runtime = adapters
        .get(session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    if let Err(error) = runtime.respond(request_id, decision) {
        drop(adapters);
        if is_worker {
            let _ = session_supervisor::SessionSupervisor::transition(
                &core.db.lock().unwrap(),
                session_id,
                worker_lifecycle::WorkerLifecycleState::Waiting,
                Some("approval_delivery_failed"),
            );
        }
        return Err(error);
    }
    drop(adapters);
    let mut normalized = agent::NormalizedEvent {
        kind: "approval.resolved".into(),
        item_id: None,
        role: None,
        status: Some(decision.to_owned()),
        title: Some("Approval resolved".into()),
        text: None,
        data: serde_json::json!({"requestEventId":event_id,"decision":decision}),
    };
    normalized.item_id = detail("itemId")
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_owned);
    let db = core.db.lock().unwrap();
    let event = store::session_event(
        &db,
        session_id,
        &normalized,
        &serde_json::json!({"adapter":adapter_id}),
    )?;
    if !is_worker {
        db.execute(
            "UPDATE sessions SET status='working' WHERE id=?1",
            params![session_id],
        )?;
    }
    db.execute(
        "UPDATE workspaces SET status=CASE
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='waiting') THEN 'waiting'
            WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=workspaces.id AND status='working') THEN 'working'
            ELSE 'ready' END
         WHERE id=(SELECT workspace_id FROM sessions WHERE id=?1)",
        params![session_id],
    )?;
    drop(db);
    core.events.publish(CoreEvent::Agent(event));
    // Resolving from any surface — the child conversation or the parent's
    // mirrored card — must update the parent's view and the worker lifecycle
    // together, so the two never disagree about whether the child is blocked.
    if is_worker {
        live_turn::notify_parent_child_left_waiting(core, session_id, decision);
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(())
}

// --- terminal ----------------------------------------------------------------

pub fn open_terminal(core: &Arc<BridgeCore>, workspace_id: &str) -> Result<(), BridgeError> {
    let runtime_id = format!("terminal:{workspace_id}");
    // Exclusive across the whole check → spawn → insert window: two
    // concurrent opens would otherwise both pass the check, and the second
    // insert would overwrite the first entry and orphan its PTY child.
    let _lifecycle = core.claim_session_lifecycle(&runtime_id, "terminal open")?;
    if core.runtimes.lock().unwrap().contains_key(&runtime_id) {
        return Ok(());
    }
    let db = core.db.lock().unwrap();
    let path: String = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get(0),
    )?;
    drop(db);
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 32,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let mut command = CommandBuilder::new("zsh");
    command.args(["-l"]);
    command.cwd(&path);
    command.env("TERM", "xterm-256color");
    command.env("BRIDGE_WORKSPACE_ID", workspace_id);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| BridgeError::Pty(e.to_string()))?;
    core.runtimes.lock().unwrap().insert(
        runtime_id.clone(),
        RuntimeSession {
            writer,
            master: pair.master,
            child,
        },
    );
    let core_reader = Arc::clone(core);
    let workspace_reader = workspace_id.to_owned();
    let runtime_reader = runtime_id;
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    core_reader.events.publish(CoreEvent::SessionOutput {
                        session_id: workspace_reader.clone(),
                        data,
                    });
                }
            }
        }
        core_reader.runtimes.lock().unwrap().remove(&runtime_reader);
    });
    Ok(())
}

pub fn write_terminal(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    data: &str,
) -> Result<(), BridgeError> {
    let mut sessions = core.runtimes.lock().unwrap();
    let runtime = sessions
        .get_mut(&format!("terminal:{workspace_id}"))
        .ok_or_else(|| BridgeError::Invalid("Workspace terminal is not open".into()))?;
    runtime.writer.write_all(data.as_bytes())?;
    runtime.writer.flush()?;
    Ok(())
}

pub fn resize_terminal(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    rows: u16,
    cols: u16,
) -> Result<(), BridgeError> {
    if let Some(runtime) = core
        .runtimes
        .lock()
        .unwrap()
        .get_mut(&format!("terminal:{workspace_id}"))
    {
        runtime
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| BridgeError::Pty(e.to_string()))?
    }
    Ok(())
}

// --- slash commands ------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandResolve {
    pub name: String,
    pub harness: String,
    pub kind: String,
    /// When true, the frontend should switch the direct chat to `harness`
    /// before sending.
    pub switch_harness: bool,
}

fn available_adapter_ids(core: &BridgeCore) -> HashSet<String> {
    core.adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect()
}

/// Enumerate slash commands + skills from every signed-in provider, so the UI
/// can offer a labeled `/` menu.
pub fn list_slash_commands(
    core: &Arc<BridgeCore>,
) -> Result<Vec<slash::SlashCommand>, BridgeError> {
    Ok(slash::list_commands(&available_adapter_ids(core)))
}

/// Resolve a composer `/command` against the catalog so the UI can auto-switch
/// harness before sending.
pub fn resolve_slash_command(
    core: &Arc<BridgeCore>,
    text: &str,
    session_id: &str,
) -> Result<Option<SlashCommandResolve>, BridgeError> {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Ok(None);
    };
    let name = rest
        .split_whitespace()
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BridgeError::Invalid("Empty slash command".into()))?;
    let (kind, session_harness): (String, String) = {
        let db = core.db.lock().unwrap();
        db.query_row(
            "SELECT kind, harness FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };
    if slash::is_bridge_local(name) {
        return Ok(Some(SlashCommandResolve {
            name: name.to_string(),
            harness: session_harness,
            kind: "builtin".into(),
            switch_harness: false,
        }));
    }
    let available = available_adapter_ids(core);
    let catalog = slash::list_commands(&available);
    let matches: Vec<_> = catalog
        .iter()
        .filter(|command| command.name.eq_ignore_ascii_case(name))
        .collect();
    if matches.is_empty() {
        return Ok(None);
    }
    let chosen = matches
        .iter()
        .find(|command| command.harness == session_harness)
        .or_else(|| {
            // Prefer the command's own harness when the name is unique to one provider.
            if matches.len() == 1 {
                matches.first()
            } else {
                None
            }
        })
        .or_else(|| matches.first())
        .map(|command| (*command).clone())
        .expect("matches non-empty");
    let switch_harness = kind == "direct" && chosen.harness != session_harness;
    Ok(Some(SlashCommandResolve {
        name: chosen.name.clone(),
        harness: chosen.harness.clone(),
        kind: chosen.kind.clone(),
        switch_harness,
    }))
}

// --- completion / verification -------------------------------------------------

fn completion_repository_stamp(
    db: &Connection,
    session_id: &str,
) -> Result<completion::RepositoryStamp, BridgeError> {
    let state = store::repository_state_for_session(db, session_id)?;
    let head = state.get("head").and_then(Value::as_str).ok_or_else(|| {
        BridgeError::Invalid("completion proof requires a Git repository HEAD".into())
    })?;
    let dirty = state
        .get("dirtyHash")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid(
                "completion proof requires a deterministic dirty-tree digest".into(),
            )
        })?;
    Ok(completion::RepositoryStamp {
        head: head.into(),
        dirty_digest: dirty.into(),
    })
}

fn completion_attempt_repository(
    db: &Connection,
    attempt_id: &str,
) -> Result<(String, completion::RepositoryStamp), BridgeError> {
    let (session_id, repository_path, stored_head, stored_dirty): (String, String, String, String) = db.query_row(
        "SELECT session_id,repository_path,repository_head,dirty_digest FROM eval_attempts WHERE id=?1",
        params![attempt_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let state = store::repository_state_for_path(Path::new(&repository_path));
    let head = state
        .get("head")
        .and_then(Value::as_str)
        .unwrap_or(&stored_head);
    let dirty = state
        .get("dirtyHash")
        .and_then(Value::as_str)
        .unwrap_or(&stored_dirty);
    Ok((
        session_id,
        completion::RepositoryStamp {
            head: head.into(),
            dirty_digest: dirty.into(),
        },
    ))
}

/// Every capability a verifier can currently rely on: live adapters plus the
/// installed skill catalog.
fn live_available_capabilities(core: &BridgeCore) -> HashSet<String> {
    let mut capabilities = core
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .flat_map(|descriptor| descriptor.capabilities)
        .collect::<HashSet<_>>();
    if let Ok(skills) = skill_marketplace::available_capabilities(&user_home(), &core.skill_store) {
        capabilities.extend(skills);
    }
    capabilities
}

pub fn create_completion_plan(
    core: &Arc<BridgeCore>,
    session_id: &str,
    acceptance_criteria: Vec<String>,
    changed_paths: Vec<String>,
    repository_commands: Vec<String>,
    markdown_projection: Option<String>,
    markdown_committed: bool,
) -> Result<completion::CompletionSummary, BridgeError> {
    let (workspace_id, implementer_family): (String, Option<String>) = {
        let db = core.db.lock().unwrap();
        let workspace_id = db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let implementer_family = db.query_row(
            "SELECT s.harness FROM worker_runtime r JOIN worker_leases l ON l.session_id=r.session_id JOIN sessions s ON s.id=r.session_id WHERE r.parent_session_id=?1 AND l.role='implementation' ORDER BY r.updated_at DESC LIMIT 1",
            params![session_id],
            |row| row.get(0),
        ).optional()?;
        (workspace_id, implementer_family)
    };
    let contract = completion::CompletionContract {
        id: Uuid::new_v4().to_string(),
        workspace_id,
        session_id: session_id.to_owned(),
        schema_version: completion::COMPLETION_SCHEMA_VERSION,
        acceptance_criteria: acceptance_criteria.clone(),
        markdown_projection,
        markdown_committed,
    };
    let available_capabilities = live_available_capabilities(core);
    let change_labels = completion::labels_for_paths(&changed_paths);
    let db = core.db.lock().unwrap();
    let plan = completion::plan_with_registered_manifests(
        &db,
        completion::PlanInput {
            contract_id: contract.id.clone(),
            acceptance_criteria,
            changed_paths,
            repository_commands,
        },
        &change_labels,
        &available_capabilities,
    )?;
    let repository_path: String = db.query_row("SELECT COALESCE(s.cwd,w.path) FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1", params![session_id], |row| row.get(0))?;
    let repository = completion_repository_stamp(&db, session_id)?;
    completion::create_flow(
        &db,
        &contract,
        &plan,
        session_id,
        &repository_path,
        &repository,
        implementer_family.as_deref(),
    )?;
    let summary = completion::latest_summary(&db, session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion plan was not persisted".into()))?;
    drop(db);
    core.events.publish(CoreEvent::StateChanged);
    Ok(summary)
}

pub fn record_completion_check(
    core: &Arc<BridgeCore>,
    attempt_id: &str,
    run: &completion::CheckRun,
) -> Result<completion::CompletionSummary, BridgeError> {
    let db = core.db.lock().unwrap();
    let (session_id, repository) = completion_attempt_repository(&db, attempt_id)?;
    completion::record_check(&db, attempt_id, run)?;
    completion::finalize(&db, attempt_id, &repository)?;
    completion::reconcile_parent_readiness(&db, &session_id)?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion summary disappeared".into()))?;
    drop(db);
    core.events.publish(CoreEvent::StateChanged);
    Ok(summary)
}

pub fn waive_completion(
    core: &Arc<BridgeCore>,
    attempt_id: &str,
    check_ids: &[String],
    reason: &str,
) -> Result<completion::CompletionSummary, BridgeError> {
    let db = core.db.lock().unwrap();
    let (session_id, repository) = completion_attempt_repository(&db, attempt_id)?;
    completion::waive(
        &db,
        attempt_id,
        check_ids,
        reason,
        "local_user",
        &repository,
    )?;
    completion::finalize(&db, attempt_id, &repository)?;
    completion::reconcile_parent_readiness(&db, &session_id)?;
    let summary = completion::latest_summary(&db, &session_id)?
        .ok_or_else(|| BridgeError::Invalid("completion summary disappeared".into()))?;
    drop(db);
    core.events.publish(CoreEvent::StateChanged);
    Ok(summary)
}

pub fn register_verifier_manifest(
    core: &Arc<BridgeCore>,
    source: &str,
    manifest: &completion::VerifierManifest,
) -> Result<(), BridgeError> {
    completion::register_verifier_manifest(&core.db.lock().unwrap(), source, manifest)
}

pub fn verifier_candidates(
    core: &Arc<BridgeCore>,
    change_labels: &[String],
    available_capabilities: Vec<String>,
) -> Result<Vec<completion::VerifierCandidate>, BridgeError> {
    completion::verifier_candidates(
        &core.db.lock().unwrap(),
        change_labels,
        &available_capabilities.into_iter().collect(),
    )
}

// --- work ---------------------------------------------------------------------

/// The Work board. Store-only: it takes the connection and nothing else, so no
/// git command, provider start, connector call, or network request can happen on
/// the way to a rendered board.
pub fn get_work_board(core: &Arc<BridgeCore>) -> Result<wire::WorkBoard, BridgeError> {
    work::board(&core.db.lock().unwrap())
}

/// Apply a human's decision to a suggested task.
///
/// Local only. Marking a task done does not close the thread it came from, and dismissing
/// it does not archive anything — this writes one row in Bridge's own database and stops.
/// The state machine decides whether the transition is legal; an illegal one is refused
/// rather than quietly ignored, so a caller cannot believe something happened.
pub fn work_task_action(
    core: &Arc<BridgeCore>,
    params: &wire::TaskActionParams,
) -> Result<(), BridgeError> {
    let action = match params.action {
        wire::WorkTaskActionKind::Done => work_task_state::TaskAction::Complete,
        wire::WorkTaskActionKind::Snooze => work_task_state::TaskAction::Snooze,
        wire::WorkTaskActionKind::Dismiss => work_task_state::TaskAction::Dismiss,
        wire::WorkTaskActionKind::Restore => work_task_state::TaskAction::Restore,
    };
    let now = chrono::Utc::now();
    let now_text = now.to_rfc3339();
    let db = core.db.lock().unwrap();
    let current = work_reconcile::read_task(&db, &params.task_id)?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?;
    let next = work_task_state::apply_action(&current, action, &now_text)
        .map_err(|illegal| BridgeError::Invalid(illegal.reason()))?;
    let snoozed_until = match params.action {
        wire::WorkTaskActionKind::Snooze => {
            let deadline = params.snoozed_until.as_deref()
                .ok_or_else(|| BridgeError::Invalid("snoozing requires a deadline".into()))?;
            let parsed = chrono::DateTime::parse_from_rfc3339(deadline)
                .map_err(|_| BridgeError::Invalid("the snooze deadline is not a timestamp".into()))?;
            if parsed <= now {
                return Err(BridgeError::Invalid("the snooze deadline must be in the future".into()));
            }
            Some(deadline)
        }
        // Leaving a stale deadline on a task that is no longer snoozed would make a later
        // expiry check answer about a snooze nobody set.
        _ => None,
    };
    work_reconcile::write_task_state(&db, &params.task_id, &next, snoozed_until, &now_text)
}

/// Pin or unpin a task.
///
/// Its own call rather than an action variant, because pinning is orthogonal to the five
/// states: folding it in would let a caller send `pin` where a state change is expected.
pub fn work_task_pin(
    core: &Arc<BridgeCore>,
    params: &wire::TaskPinParams,
) -> Result<(), BridgeError> {
    let action = if params.pinned {
        work_task_state::TaskAction::Pin
    } else {
        work_task_state::TaskAction::Unpin
    };
    let now = chrono::Utc::now().to_rfc3339();
    let db = core.db.lock().unwrap();
    let current = work_reconcile::read_task(&db, &params.task_id)?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?;
    let next = work_task_state::apply_action(&current, action, &now)
        .map_err(|illegal| BridgeError::Invalid(illegal.reason()))?;
    work_reconcile::write_task_pin(&db, &params.task_id, next.pinned, &now)
}

/// Prepare a session for working on a task.
///
/// Creates the session and the draft, and sends nothing. The returned shape carries no turn
/// id precisely because none was dispatched: the user edits the draft and presses Send, and
/// until they do, no model has been told anything.
pub fn work_task_prepare_session(
    core: &Arc<BridgeCore>,
    params: &wire::TaskPrepareSessionParams,
) -> Result<wire::WorkTaskDraft, BridgeError> {
    let (title, why, source_kind) = {
        let db = core.db.lock().unwrap();
        db.query_row(
            "SELECT title,why,source_kind FROM work_tasks WHERE id=?1",
            params![params.task_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?
    };
    let draft = work_actions::prepare_draft(&title, &why, &source_kind);
    // create_chat_id inserts a session row and starts no adapter, which is the whole
    // requirement: the session exists to be typed into, and no provider has been spoken to.
    let harness = Harness::parse(params.harness.as_str())
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let session_id = core.create_chat_id(&harness, params.model.as_deref(), Some(&draft.title))?;
    Ok(wire::WorkTaskDraft {
        session_id,
        title: draft.title,
        draft: draft.draft,
    })
}

/// Recheck and return one task's evidence destination at the point the user opens it.
pub fn work_task_open_evidence(
    core: &Arc<BridgeCore>,
    params: &wire::TaskOpenEvidenceParams,
) -> Result<wire::WorkEvidenceTarget, BridgeError> {
    let (stored, source_kind): (Option<String>, String) = core.db.lock().unwrap()
        .query_row(
            "SELECT evidence_target,source_kind FROM work_tasks WHERE id=?1",
            params![params.task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| BridgeError::Invalid("that task is not on the board".into()))?;
    let family = crate::work_connectors::ConnectorFamily::parse(
        source_kind.split('.').next().unwrap_or_default(),
    )
    .ok_or_else(|| BridgeError::Invalid("that task's source is not supported".into()))?;
    match work_actions::open_target(stored.as_deref(), family)
        .map_err(|refused| BridgeError::Invalid(refused.reason()))?
    {
        work_actions::OpenTarget::External { url, host } => {
            Ok(wire::WorkEvidenceTarget::ExternalLink { url, host })
        }
        work_actions::OpenTarget::Session { session_id } => {
            Ok(wire::WorkEvidenceTarget::Session { session_id })
        }
    }
}

/// Work's stored settings, with whether they were ever written.
pub fn read_work_settings(core: &Arc<BridgeCore>) -> Result<wire::WorkSettingsSnapshot, BridgeError> {
    work::read_settings(&core.db.lock().unwrap())
}

/// Persist Work's settings. Validation lives in Rust — `work::validate_settings`
/// plus the model-catalog check below, which needs the adapter registry that the
/// store-only module deliberately cannot reach.
pub fn write_work_settings(
    core: &Arc<BridgeCore>,
    params: &wire::WriteSettingsParams,
) -> Result<wire::WorkSettingsSnapshot, BridgeError> {
    if let Some(briefing) = params.settings.briefing.as_ref() {
        let descriptors = core.adapter_registry.descriptors();
        if let Some(descriptor) = descriptors
            .iter()
            .find(|descriptor| descriptor.id == briefing.harness.as_str())
        {
            // An empty catalog is a runtime-discovered one; only a non-empty
            // catalog can refuse a model by name.
            if !descriptor.models.is_empty()
                && !descriptor.models.iter().any(|model| model.id == briefing.model)
            {
                return Err(BridgeError::Invalid(format!(
                    "{} is not a model {} offers",
                    briefing.model, descriptor.label
                )));
            }
        }
    }
    work::write_settings(&core.db.lock().unwrap(), &params.settings)
}

/// Every registered harness as the briefing Settings surface needs it: certified
/// or refused with the gate's reason, plus the cheapest capable default model.
pub fn work_briefing_options(core: &Arc<BridgeCore>) -> wire::WorkBriefingOptions {
    let harnesses = core
        .adapter_registry
        .descriptors()
        .into_iter()
        .map(|descriptor| {
            let certification = crate::briefing_policy::certify_briefing(
                &descriptor.id,
                descriptor.version.as_deref(),
            );
            let supported = certification.is_ok();
            // The cheapest capable model is the Fast-tier default. Chosen here,
            // at the settings layer, because `resolve_briefing` deliberately
            // refuses to invent a model at run time.
            let default_model = supported
                .then(|| {
                    core.adapter_registry
                        .resolve_model(&descriptor.id, CapabilityTier::Fast, None)
                        .ok()
                        .map(|resolution| resolution.actual_model)
                })
                .flatten();
            // The connectors this harness itself holds, so Settings offers
            // narrowing to what actually exists. Only a certified harness gets
            // the (subprocess-backed) discovery: an uncertified one runs
            // nothing, so there is nothing to narrow.
            let connectors = (supported && descriptor.id == "claude")
                .then(|| {
                    let configuration = crate::marketplace::claude_sdk_configuration();
                    configuration
                        .mcp_servers
                        .keys()
                        .map(|instance| wire::WorkBriefingConnector {
                            id: instance.clone(),
                            family: crate::work_briefing_live::family_for_server(instance)
                                .map(|family| family.as_str().to_owned())
                                .unwrap_or_else(|| "unknown".to_owned()),
                            connected: configuration
                                .connector_health
                                .get(instance)
                                .copied()
                                .flatten(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            wire::WorkBriefingHarness {
                supported,
                reason: certification.err().map(|unsupported| unsupported.reason()),
                default_model,
                models: descriptor
                    .models
                    .iter()
                    .map(|model| wire::WorkBriefingModel {
                        id: model.id.clone(),
                        label: model.label.clone(),
                        tier: model.tier.as_str().to_owned(),
                        default_for_briefing: model.tier == CapabilityTier::Fast
                            && model.default_for_tier,
                    })
                    .collect(),
                connectors,
                id: descriptor.id,
                label: descriptor.label,
                available: descriptor.available,
            }
        })
        .collect();
    wire::WorkBriefingOptions { harnesses }
}

/// Trigger a briefing run. Every trigger — this one, app focus, and the cadence
/// thread — funnels through `work_briefing_trigger::claim`, so racing calls
/// start at most one run and the others observe it.
///
/// Returns immediately: a claimed run executes on its own thread and lands on
/// the run row, never in this response. The board's `suggestions.state` is how
/// a client follows it.
pub fn run_work_briefing(
    core: &Arc<BridgeCore>,
    params: &wire::RunBriefingParams,
) -> Result<wire::WorkBriefReceipt, BridgeError> {
    let registry = core.adapter_registry.clone();
    let versions = move |harness: &str| {
        registry
            .descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == harness)
            .and_then(|descriptor| descriptor.version)
    };
    let outcome = {
        let db = core.db.lock().unwrap();
        crate::work_briefing_trigger::claim(&db, params.trigger, &versions, chrono::Utc::now())?
    };
    Ok(match outcome {
        crate::work_briefing_trigger::ClaimOutcome::Claimed(run) => {
            let run_id = run.run_id.clone();
            let worker_core = core.clone();
            thread::spawn(move || crate::work_briefing_live::execute(&worker_core, run));
            wire::WorkBriefReceipt {
                outcome: wire::WorkBriefReceiptOutcome::Started,
                run_id: Some(run_id),
                code: None,
                detail: None,
            }
        }
        crate::work_briefing_trigger::ClaimOutcome::Observed { run_id } => wire::WorkBriefReceipt {
            outcome: wire::WorkBriefReceiptOutcome::Observed,
            run_id: Some(run_id),
            code: None,
            detail: None,
        },
        crate::work_briefing_trigger::ClaimOutcome::Refused { code, detail } => {
            wire::WorkBriefReceipt {
                outcome: wire::WorkBriefReceiptOutcome::Refused,
                run_id: None,
                code: Some(code),
                detail: Some(detail),
            }
        }
    })
}

/// Ask the active briefing run to stop. The run loop notices the flag, stops
/// the provider, records terminal status and measured usage, and leaves the
/// last good board intact.
pub fn cancel_work_briefing(core: &Arc<BridgeCore>) -> Result<wire::WorkBriefReceipt, BridgeError> {
    let cancelled = crate::work_briefing_trigger::request_cancel(&core.db.lock().unwrap())?;
    Ok(match cancelled {
        Some(run_id) => wire::WorkBriefReceipt {
            outcome: wire::WorkBriefReceiptOutcome::Observed,
            run_id: Some(run_id),
            code: None,
            detail: Some("cancellation requested; the run settles on its own thread".into()),
        },
        None => wire::WorkBriefReceipt {
            outcome: wire::WorkBriefReceiptOutcome::Refused,
            run_id: None,
            code: Some("not_running".into()),
            detail: Some("no briefing run is active".into()),
        },
    })
}

// --- base-branch divergence ----------------------------------------------------

/// Cache a divergence reading the user just paid for.
///
/// The Work board reads observations, never git, so a measurement taken on a
/// user's behalf should update the board instead of being thrown away. Failure
/// to cache is not failure to answer: the caller still gets its reading.
fn cache_base_divergence(
    core: &Arc<BridgeCore>,
    session_id: &str,
    divergence: &git::BaseBranchDivergence,
) {
    let db = core.db.lock().unwrap();
    let workspace_id: Option<String> = db
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(workspace_id) = workspace_id {
        let _ = work_observation::record_base_divergence(
            &db,
            &workspace_id,
            Ok(divergence),
            chrono::Utc::now(),
        );
    }
}

/// How far this session's workspace has drifted from the branch it builds on.
/// Read-only; `fetch` controls whether the network is consulted.
pub fn workspace_base_divergence(
    core: &Arc<BridgeCore>,
    session_id: &str,
    fetch: bool,
) -> Result<git::BaseBranchDivergence, BridgeError> {
    // Resolve under the lock, run Git outside it: a fetch can be slow.
    let path = store::repository_path_for_session(&core.db.lock().unwrap(), session_id)?
        .ok_or_else(|| BridgeError::Invalid("this session has no connected repository".into()))?;
    let divergence = git::base_branch_divergence(&path, fetch);
    cache_base_divergence(core, session_id, &divergence);
    Ok(divergence)
}

/// The "refresh" choice offered by a stale-base warning: fast-forward the
/// workspace onto its base ref. Refuses on a dirty tree, an active session, or
/// any history that is not a pure fast-forward.
pub fn refresh_workspace_base(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<git::BaseBranchDivergence, BridgeError> {
    let (path, active) = {
        let db = core.db.lock().unwrap();
        let path = store::repository_path_for_session(&db, session_id)?.ok_or_else(|| {
            BridgeError::Invalid("this session has no connected repository".into())
        })?;
        let active: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sessions WHERE (id=?1 OR parent_session_id=?1) AND status IN ('starting','working','resuming','checkpointing'))",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap_or(true);
        (path, active)
    };
    let divergence = git::fast_forward_to_base(&path, active)?;
    cache_base_divergence(core, session_id, &divergence);
    {
        let db = core.db.lock().unwrap();
        let _ = store::event(
            &db,
            "workspace",
            "workspace.base_refreshed",
            session_id,
            &serde_json::to_string(&divergence).unwrap_or_default(),
        );
        // Durable, so the resolution is still visible after a reload instead of
        // leaving only the warning that prompted it.
        let resolved = agent::NormalizedEvent {
            kind: "workspace.stale_base".into(),
            item_id: Some(format!(
                "stale-base-{}@{}",
                divergence.base_ref.as_deref().unwrap_or("unknown"),
                divergence.base_commit.as_deref().unwrap_or("unknown")
            )),
            role: Some("system".into()),
            status: Some("resolved".into()),
            title: Some(format!(
                "Workspace refreshed onto {}",
                divergence.base_ref.as_deref().unwrap_or("its base branch")
            )),
            text: Some(divergence.summary()),
            data: serde_json::json!({
                "staleBase": true,
                "phase": "refreshed",
                "divergence": divergence,
                "choices": [],
            }),
        };
        if let Ok(stored) = store::session_event(
            &db,
            session_id,
            &resolved,
            &serde_json::json!({"workspace": true}),
        ) {
            drop(db);
            core.events.publish(CoreEvent::Agent(stored));
        }
    }
    core.events.publish(CoreEvent::StateChanged);
    Ok(divergence)
}

// --- worker worktree adoption --------------------------------------------------

/// Children of this session whose changes exist only in their own worktree.
pub fn pending_worker_adoptions(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<Vec<worker_adoption::WorkerRepositoryBinding>, BridgeError> {
    worker_adoption::pending_for_parent(&core.db.lock().unwrap(), session_id)
}

/// Merge a worker's isolated worktree into the task checkout. Integration
/// refuses to run against a dirty or active task worktree, so a rejected call
/// leaves the work pending rather than losing it.
pub fn adopt_worker_worktree(
    core: &Arc<BridgeCore>,
    session_id: &str,
) -> Result<worker_adoption::WorkerRepositoryBinding, BridgeError> {
    // Validate under the lock, merge with the lock released, settle under it
    // again. `git merge` plus `git worktree remove` can take seconds, and holding
    // the shared connection across them would stall every other session.
    let plan = {
        let db = core.db.lock().unwrap();
        worker_adoption::plan_adoption(&db, session_id)?
    };
    let outcome = match worker_adoption::integrate(&plan) {
        Ok(outcome) => outcome,
        Err(error) => {
            // The claim must be handed back, or the decision is stuck forever.
            let db = core.db.lock().unwrap();
            let _ = worker_adoption::release_claim(&db, session_id, &error.to_string());
            drop(db);
            core.events.publish(CoreEvent::StateChanged);
            return Err(error);
        }
    };
    let binding = {
        let db = core.db.lock().unwrap();
        let binding = worker_adoption::settle_plan(
            &db,
            &plan,
            worker_adoption::STATE_ADOPTED,
            &outcome.detail,
        )?;
        // A merged tree that is not the verified tree must not inherit its proof.
        if !outcome.verified_tree_preserved {
            worker_adoption::supersede_stale_proof(&db, &plan, &outcome)?;
        }
        completion::reconcile_parent_readiness(&db, &binding.parent_session_id)?;
        binding
    };
    core.events.publish(CoreEvent::StateChanged);
    Ok(binding)
}

/// Throw a worker's isolated output away on purpose and release its worktree.
pub fn discard_worker_worktree(
    core: &Arc<BridgeCore>,
    session_id: &str,
    reason: &str,
) -> Result<worker_adoption::WorkerRepositoryBinding, BridgeError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(BridgeError::Invalid(
            "discarding a worker worktree requires a reason".into(),
        ));
    }
    let plan = {
        let db = core.db.lock().unwrap();
        worker_adoption::plan_discard(&db, session_id)?
    };
    let binding = {
        let db = core.db.lock().unwrap();
        let binding =
            worker_adoption::settle_plan(&db, &plan, worker_adoption::STATE_DISCARDED, reason)?;
        completion::reconcile_parent_readiness(&db, &binding.parent_session_id)?;
        binding
    };
    core.events.publish(CoreEvent::StateChanged);
    Ok(binding)
}

// --- routing -------------------------------------------------------------------

pub fn get_router_preferences(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    learning_router::load_preferences(&core.db.lock().unwrap(), workspace_id)
}

pub fn update_router_preferences(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    preferences: &learning_router::RouterPreferences,
) -> Result<learning_router::RouterPreferences, BridgeError> {
    let db = core.db.lock().unwrap();
    learning_router::save_preferences(&db, workspace_id, preferences)?;
    learning_router::load_preferences(&db, workspace_id)
}

pub fn rollback_routing_policy(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
    target_version: i64,
    explanation: &str,
) -> Result<learning_job::LearningState, BridgeError> {
    learning_job::rollback_policy(&core.db.lock().unwrap(), workspace_id, target_version, explanation)?;
    let result = learning_job::learning_state(&core.db.lock().unwrap(), workspace_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&result).unwrap_or_default(),
    ));
    Ok(result)
}

// --- model profiles --------------------------------------------------------------

pub fn get_model_setup(
    core: &Arc<BridgeCore>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::setup_state(&core.db.lock().unwrap())
}

pub fn recommended_model_profiles(
    core: &Arc<BridgeCore>,
) -> Result<Vec<model_profiles::ModelProfileDraft>, BridgeError> {
    model_profiles::recommended_profiles(&core.adapter_registry.descriptors())
}

pub fn save_model_profiles(
    core: &Arc<BridgeCore>,
    profiles: &[model_profiles::ModelProfileDraft],
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::save_profiles(
        &core.db.lock().unwrap(),
        &core.adapter_registry.descriptors(),
        profiles,
    )
}

pub fn reset_model_profiles(
    core: &Arc<BridgeCore>,
) -> Result<model_profiles::ModelSetupState, BridgeError> {
    model_profiles::reset_profiles(
        &core.db.lock().unwrap(),
        &core.adapter_registry.descriptors(),
    )
}

// --- configuration ----------------------------------------------------------------

fn opencode_directory(directory: Option<String>) -> Result<String, BridgeError> {
    let path = directory
        .map(|value| PathBuf::from(value.trim()))
        .filter(|path| !path.as_os_str().is_empty())
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)?;
    if !path.is_dir() {
        return Err(BridgeError::Invalid(format!(
            "OpenCode directory does not exist: {}",
            path.display()
        )));
    }
    Ok(path.to_string_lossy().into_owned())
}

pub fn get_config_state(core: &Arc<BridgeCore>) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::state(&core.db.lock().unwrap())
}

pub fn save_harness_config(
    core: &Arc<BridgeCore>,
    config: agent_config::HarnessConfig,
) -> Result<agent_config::ConfigState, BridgeError> {
    let opencode_settings = (config.id == "opencode")
        .then(|| agent_config::opencode_settings(Some(&config)))
        .transpose()?;
    let next = agent_config::save_harness(&core.db.lock().unwrap(), config)?;
    if let Some(settings) = opencode_settings {
        let directory = opencode_directory(None)?;
        let _ = core.adapter_registry.refresh_opencode(settings, &directory);
    }
    Ok(next)
}

pub fn reset_harness_config(
    core: &Arc<BridgeCore>,
    id: &str,
) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::reset_harness(&core.db.lock().unwrap(), id)?;
    if id == "opencode" {
        let directory = opencode_directory(None)?;
        let _ = core
            .adapter_registry
            .refresh_opencode(opencode_adapter::OpenCodeSettings::default(), &directory);
    }
    Ok(next)
}

pub fn refresh_opencode_catalog(
    core: &Arc<BridgeCore>,
    directory: Option<String>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    let settings = core.adapter_registry.opencode_settings()?;
    core.adapter_registry.refresh_opencode(settings, &directory)
}

pub fn set_opencode_provider_api_key(
    core: &Arc<BridgeCore>,
    provider_id: &str,
    api_key: &str,
    directory: Option<String>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    core.adapter_registry
        .set_opencode_provider_api_key(&directory, provider_id, api_key)
}

pub fn remove_opencode_provider_auth(
    core: &Arc<BridgeCore>,
    provider_id: &str,
    directory: Option<String>,
) -> Result<opencode_adapter::OpenCodeCatalog, BridgeError> {
    let directory = opencode_directory(directory)?;
    core.adapter_registry
        .remove_opencode_provider_auth(&directory, provider_id)
}

pub fn save_agent_config(
    core: &Arc<BridgeCore>,
    agent: agent_config::AgentDefinition,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::save_agent(&core.db.lock().unwrap(), agent)
}

pub fn delete_agent_config(
    core: &Arc<BridgeCore>,
    id: &str,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::delete_agent(&core.db.lock().unwrap(), id)
}

pub fn set_default_agent(
    core: &Arc<BridgeCore>,
    id: &str,
) -> Result<agent_config::ConfigState, BridgeError> {
    agent_config::set_default(&core.db.lock().unwrap(), id)
}

pub fn reset_all_config(core: &Arc<BridgeCore>) -> Result<agent_config::ConfigState, BridgeError> {
    let next = agent_config::reset_all(&core.db.lock().unwrap())?;
    let directory = opencode_directory(None)?;
    let _ = core
        .adapter_registry
        .refresh_opencode(opencode_adapter::OpenCodeSettings::default(), &directory);
    Ok(next)
}

// --- adaptive learning --------------------------------------------------------------

pub fn get_learning_state(
    core: &Arc<BridgeCore>,
    workspace_id: &str,
) -> Result<learning_job::LearningState, BridgeError> {
    learning_job::learning_state(&core.db.lock().unwrap(), workspace_id)
}

pub fn run_learning(
    core: &Arc<BridgeCore>,
    trigger_kind: learning_job::LearningTriggerKind,
    workspace_id: &str,
) -> Result<learning_job::LearningRun, BridgeError> {
    if matches!(
        trigger_kind,
        learning_job::LearningTriggerKind::Codex
            | learning_job::LearningTriggerKind::Claude
            | learning_job::LearningTriggerKind::OpenCode
    ) {
        return Err(BridgeError::Invalid(
            "external learning triggers must use a registered narrow command".into(),
        ));
    }
    // Learning runs open their own connection: the run must never hold the
    // global SQLite lock across model evaluation.
    let run = learning_job::run_local_database(&core.database_path, trigger_kind, workspace_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&run).unwrap_or_default(),
    ));
    Ok(run)
}

pub fn cancel_learning_run(
    core: &Arc<BridgeCore>,
    run_id: &str,
) -> Result<learning_job::LearningRun, BridgeError> {
    let run = learning_job::cancel_run(&core.db.lock().unwrap(), run_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&run).unwrap_or_default(),
    ));
    Ok(run)
}

pub fn update_learning_schedule(
    core: &Arc<BridgeCore>,
    schedule: &learning_job::LearningSchedule,
) -> Result<learning_job::LearningSchedule, BridgeError> {
    learning_job::update_schedule(&core.db.lock().unwrap(), schedule)
}

pub fn register_learning_trigger(
    core: &Arc<BridgeCore>,
    kind: learning_job::LearningTriggerKind,
    registration_id: &str,
    credential_ref: Option<&str>,
    expires_at: Option<&str>,
) -> Result<(), BridgeError> {
    learning_job::register_trigger_with_expiry(
        &core.db.lock().unwrap(),
        kind,
        registration_id,
        credential_ref,
        expires_at,
    )
}

pub fn get_learning_trigger_instructions(
    kind: learning_job::LearningTriggerKind,
    database_path: &str,
    registration_id: &str,
) -> Result<String, BridgeError> {
    learning_job::trigger_instructions(kind, database_path, registration_id)
}

pub fn enable_learning_trigger(
    core: &Arc<BridgeCore>,
    kind: learning_job::LearningTriggerKind,
    registration_id: &str,
) -> Result<(), BridgeError> {
    learning_job::enable_trigger(&core.db.lock().unwrap(), kind, registration_id)
}

pub fn approve_learning_run(
    core: &Arc<BridgeCore>,
    run_id: &str,
) -> Result<learning_job::LearningRun, BridgeError> {
    let run = learning_job::approve_run(&core.db.lock().unwrap(), run_id)?;
    core.events.publish(CoreEvent::LearningJobChanged(
        serde_json::to_value(&run).unwrap_or_default(),
    ));
    Ok(run)
}

// --- browser bridge --------------------------------------------------------------

pub fn browser_bridge_state(
    core: &Arc<BridgeCore>,
) -> Result<browser_bridge::BrowserBridgeSnapshot, BridgeError> {
    Ok(core.browser_bridge.snapshot())
}

fn find_browser_host(directory: &Path) -> Option<PathBuf> {
    let direct = directory.join("bridge-browser-host");
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("bridge-browser-host-"))
        })
}

pub fn install_browser_native_host(core: &Arc<BridgeCore>) -> Result<String, BridgeError> {
    let executable = std::env::var_os("BRIDGE_BROWSER_HOST")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|path| path.parent().and_then(find_browser_host))
        })
        .ok_or_else(|| BridgeError::Invalid("Could not locate bridge-browser-host".into()))?;
    if !executable.exists() {
        return Err(BridgeError::Invalid(format!(
            "Native host executable is missing at {}. Build the bridge-browser-host binary first.",
            executable.display()
        )));
    }
    core.browser_bridge
        .install_native_host(&executable)
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn browser_action(
    core: &Arc<BridgeCore>,
    request: browser_bridge::BrowserActionRequest,
) -> Result<String, BridgeError> {
    core.browser_bridge.issue(request)
}

pub fn set_browser_permission(core: &Arc<BridgeCore>, permission: &str) -> Result<(), BridgeError> {
    core.browser_bridge.set_permission(permission)
}

pub fn resolve_browser_approval(
    core: &Arc<BridgeCore>,
    approval_id: &str,
    allow: bool,
) -> Result<(), BridgeError> {
    core.browser_bridge.resolve_approval(approval_id, allow)
}

pub fn takeover_browser(core: &Arc<BridgeCore>) -> Result<(), BridgeError> {
    core.browser_bridge.takeover()
}

pub fn detach_browser(core: &Arc<BridgeCore>) -> Result<String, BridgeError> {
    core.browser_bridge.detach()
}

pub fn route_browser(
    request: browser_bridge::BrowserRouteRequest,
) -> browser_bridge::BrowserRouteDecision {
    browser_bridge::route_browser(request)
}

pub fn browser_skills() -> Vec<browser_bridge::BrowserSkill> {
    browser_bridge::bundled_skills()
}

pub fn configure_remote_browser(
    core: &Arc<BridgeCore>,
    config: Option<browser_bridge::RemoteBrowserConfig>,
) -> Result<(), BridgeError> {
    core.browser_bridge.configure_remote(config)
}

pub fn start_remote_browser(
    core: &Arc<BridgeCore>,
    initial_url: &str,
) -> Result<Value, BridgeError> {
    core.browser_bridge.start_remote_session(initial_url)
}

// --- marketplace -----------------------------------------------------------------

pub fn marketplace_catalog() -> marketplace::MarketplaceCatalog {
    marketplace::catalog()
}

pub fn marketplace_app_auth_states(
) -> Result<Vec<marketplace::MarketplaceAppAuthState>, BridgeError> {
    marketplace::app_auth_states()
}

pub fn marketplace_action(
    provider: marketplace::MarketplaceProvider,
    plugin_id: &str,
    marketplace_name: Option<&str>,
    action: marketplace::MarketplaceAction,
) -> Result<marketplace::MarketplaceActionResult, BridgeError> {
    marketplace::execute_action(provider, plugin_id, marketplace_name, action)
}

// --- skills ------------------------------------------------------------------------

/// The invoking user's home directory, where each harness keeps its skill root.
pub fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn skill_catalog(
    core: &Arc<BridgeCore>,
) -> Result<skill_marketplace::SkillCatalog, BridgeError> {
    skill_marketplace::catalog(&user_home(), &core.skill_store)
}

pub fn skill_suggestions(
    core: &Arc<BridgeCore>,
    query: &str,
    provider: skill_marketplace::SkillProvider,
) -> Result<Vec<skill_marketplace::CapabilitySuggestion>, BridgeError> {
    skill_marketplace::suggestions(query, provider, &user_home(), &core.skill_store)
}

pub fn preview_skill_change(
    core: &Arc<BridgeCore>,
    skill_id: &str,
    action: skill_marketplace::SkillAction,
    targets: &[skill_marketplace::SkillProvider],
) -> Result<skill_marketplace::SkillPreview, BridgeError> {
    skill_marketplace::preview(
        skill_id,
        action,
        targets,
        &user_home(),
        &core.skill_store,
        core.skill_consents.as_ref(),
    )
}

pub fn execute_skill_change(
    core: &Arc<BridgeCore>,
    confirmation_id: &str,
) -> Result<Vec<skill_marketplace::SkillActionResult>, BridgeError> {
    let results = skill_marketplace::execute(
        confirmation_id,
        &user_home(),
        &core.skill_store,
        core.skill_consents.as_ref(),
    )?;
    core.events.publish(CoreEvent::StateChanged);
    Ok(results)
}

#[cfg(test)]
mod tests {
    #[test]
    fn learning_runs_never_hold_the_global_sqlite_lock() {
        // run_learning opens its own connection via run_local_database; a
        // locked-connection call would serialize the whole app behind model
        // evaluation.
        let source = include_str!("api.rs");
        assert!(
            source.contains(
                "learning_job::run_local_database(&core.database_path, trigger_kind, workspace_id)",
            )
        );
        let locked_learning_call = [
            "learning_job::run_learning(",
            "&core.db.lock().unwrap()",
            ", trigger_kind)",
        ]
        .concat();
        assert!(!source.contains(&locked_learning_call));
    }

    #[test]
    fn memory_saves_and_forgets_publish_the_scope_hint() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let mut events = core.events.subscribe();
        let record = super::save_memory_record(&core, "Prefers tabs over spaces", None, None)
            .expect("an explicit save is accepted");
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { ref scope_key } if scope_key == "account:local"
        ));
        super::delete_memory_record(&core, &record.id).expect("forget tombstones");
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::MemoryChanged { ref scope_key } if scope_key == "account:local"
        ));
        // A refused save changes nothing, so it owes no hint.
        assert!(super::save_memory_record(&core, "   ", None, None).is_err());
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn memory_capabilities_answer_without_a_harness_comparison() {
        let scratch = tempfile::tempdir().unwrap();
        let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
        let capabilities = super::get_memory_capabilities(&core).unwrap();
        assert!(capabilities.ledger.exists);
        assert_eq!(capabilities.ledger.scope_key, "account:local");
        assert_eq!(capabilities.ledger.kinds.len(), 4);
        // The test core registers no adapters, so no provider contributes.
        assert!(capabilities.provider_native.is_empty());
    }

    #[test]
    fn slash_pin_arms_publish_the_same_hint_as_the_api() {
        // live_turn has no test scaffold; its slash arms mirror the api seam,
        // so the wiring claim is checked the way this module already checks
        // cross-module wiring: against the source.
        let source = include_str!("live_turn.rs");
        assert!(
            source.matches("core.events.publish(CoreEvent::MemoryChanged").count() >= 2,
            "both /pin and /unpin publish the memory-changed hint"
        );
    }
}

// ---- agents: the managed runtime lifecycle -------------------------------
//
// The api seam both hosts call, so the shell and the daemon share one path into
// the domain rather than each wiring its own.

pub fn list_managed_agents(
) -> Result<bridge_protocol::messages::ManagedAgentList, crate::managed_agents::ManagedAgentError> {
    crate::managed_agents::list_managed_agents()
}

pub fn inspect_managed_agent(
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentInspection,
    crate::managed_agents::ManagedAgentError,
> {
    crate::managed_agents::inspect_managed_agent(agent_id)
}

pub fn install_managed_agent(
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentOperationResult,
    crate::managed_agents::ManagedAgentError,
> {
    crate::managed_agents::install_managed_agent(agent_id)
}

pub fn repair_managed_agent(
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentOperationResult,
    crate::managed_agents::ManagedAgentError,
> {
    crate::managed_agents::repair_managed_agent(agent_id)
}

/// Takes the core because removal must first prove nothing is running against
/// the payload, which is a question only the session store can answer.
pub fn uninstall_managed_agent(
    core: &Arc<BridgeCore>,
    agent_id: &str,
) -> Result<
    bridge_protocol::messages::ManagedAgentOperationResult,
    crate::managed_agents::ManagedAgentError,
> {
    let db = core.db.lock().unwrap();
    crate::managed_agents::uninstall_managed_agent(&db, agent_id)
}

// ---- agents: the session's backend binding -------------------------------

/// Authorize continuing one session under a different backend.
///
/// A session records which implementation served it, and a resume through a
/// different one is refused — the same history answered by a different agent
/// runtime is a decision only a user can make. This is how that decision is
/// given: for one session, one exact transition, spent when it is used.
///
/// Deliberately not an RPC method yet. Its wire and desktop surface belong with
/// #166's control plane, where a second backend candidate first becomes
/// reachable; exposing a control now would put a button in front of a resolver
/// that has exactly one candidate per agent.
pub fn authorize_backend_change(
    core: &Arc<BridgeCore>,
    session_id: &str,
    to_backend: &str,
) -> Result<(), BridgeError> {
    let to_backend = bridge_protocol::messages::BackendId::parse(to_backend)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let db = core.db.lock().unwrap();
    let from = match crate::backend_binding::read_binding(&db, session_id)? {
        crate::backend_binding::StoredBinding::Bound(binding) => binding,
        // Nothing to change from: an unbound session binds on its next launch,
        // and an unreadable one must not be rebound by a caller who cannot have
        // been shown what it is changing away from.
        other => {
            return Err(BridgeError::Invalid(format!(
                "{session_id} has no backend to change from ({other:?})"
            )))
        }
    };
    let candidate = crate::backend_binding::BackendResolver::candidate(
        &core.backend_resolver,
        &from.agent,
        &to_backend,
    )
    .ok_or_else(|| {
        BridgeError::Invalid(format!("{} has no backend named {to_backend}", from.agent))
    })?;
    let to = crate::backend_binding::BackendBinding {
        agent: from.agent.clone(),
        backend: candidate.backend.clone(),
        version: None,
        installation: None,
    };
    crate::backend_binding::authorize_backend_change(&db, session_id, &from, &to)
}
