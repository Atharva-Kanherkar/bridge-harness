mod adapters;
mod agent;
mod binary;
mod claude_adapter;
mod compaction_controller;
mod context;
mod codex_adapter;
mod delegation;
mod git;
mod handoff;
mod model;
mod orchestrator;
mod policy;
mod policy_coordinator;
mod restoration;
mod session_forest;
mod session_supervisor;
mod slash;
mod store;
mod worker_guard;
mod worker_lifecycle;
mod worker_pool;
mod worktree_coordinator;

use chrono::Utc;
use model::*;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("{0}")]
    Invalid(String),
    #[error("Git: {0}")]
    Git(String),
    #[error("Database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Adapter: {0}")]
    Adapter(String),
    #[error("PTY: {0}")]
    Pty(String),
}
impl Serialize for BridgeError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct RuntimeSession {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
}
struct AppState {
    db: Mutex<Connection>,
    runtimes: Mutex<HashMap<String, RuntimeSession>>,
    adapters: Mutex<HashMap<String, Box<dyn adapters::AdapterRuntime>>>,
    adapter_registry: adapters::AdapterRegistry,
    delegations: Mutex<DelegationState>,
    worktrees: PathBuf,
    database_path: PathBuf,
}

/// Bookkeeping for the multi-agent delegation tree.
#[derive(Default)]
struct DelegationState {
    /// Tracks the single same-session repair allowed for malformed worker output.
    result_repairs: delegation::ResultRepairTracker,
    /// Last observed provider turn per session, retained until the next turn
    /// so late usage events keep the originating user-request budget key.
    last_turn_by_session: HashMap<String, String>,
    /// Read-only worker session → tracked Git state captured before process start.
    read_only_baselines: HashMap<String, worker_guard::ReadOnlyBaseline>,
}

#[derive(Serialize)]
struct Health {
    ok: bool,
    version: &'static str,
    harnesses: HashMap<&'static str, bool>,
    database: String,
    adapters: Vec<AdapterDescriptor>,
}

#[tauri::command]
fn health(state: State<AppState>) -> Health {
    Health {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
        harnesses: HashMap::from([
            ("claude", binary::resolve("claude").is_some()),
            ("codex", binary::resolve("codex").is_some()),
            ("shell", true),
        ]),
        database: state.database_path.to_string_lossy().into(),
        adapters: state.adapter_registry.descriptors(),
    }
}
#[tauri::command]
fn get_state(state: State<AppState>) -> Result<BridgeState, BridgeError> {
    store::state(&state.db.lock().unwrap())
}

fn session_forest_snapshot(
    db: &Connection,
    session_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;
    let config = policy::PolicyConfig::default();
    Ok(SessionForestSnapshot {
        session_id: session_id.to_owned(),
        entries: store::session_entries(db, session_id)?,
        head: store::session_head(db, session_id)?,
        leaves: session_forest::SessionForest::new(db)
            .branch_leaves(session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?,
        worker_leases: store::worker_leases(db, &workspace_id)?,
        worker_runtimes: store::worker_runtimes(db, &workspace_id)?,
        worker_queue: store::worker_queue_requests(db, &workspace_id)?,
        usage: store::usage_ledger(db, &workspace_id, None)?,
        reasons: store::workspace_reason_events(db, &workspace_id)?,
        policy_limits: PolicyLimits {
            max_workers_per_turn: config.max_workers_per_turn as i64,
            max_strong_workers_per_turn: config.max_strong_workers_per_turn as i64,
            max_capability_units_per_turn: config.max_capability_units_per_turn,
        },
    })
}

#[tauri::command]
fn get_session_forest(
    session_id: String,
    state: State<AppState>,
) -> Result<SessionForestSnapshot, BridgeError> {
    session_forest_snapshot(&state.db.lock().unwrap(), &session_id)
}

#[tauri::command]
fn activate_session_entry(
    session_id: String,
    entry_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<SessionForestSnapshot, BridgeError> {
    let db = state.db.lock().unwrap();
    let snapshot = activate_session_entry_records(&db, &session_id, &entry_id)?;
    let _ = app.emit("state-changed", ());
    Ok(snapshot)
}

fn activate_session_entry_records(
    db: &Connection,
    session_id: &str,
    entry_id: &str,
) -> Result<SessionForestSnapshot, BridgeError> {
    session_forest::SessionForest::new(db)
        .move_head(session_id, Some(entry_id))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "session-forest",
        "session.head_moved",
        session_id,
        &format!("Conversation head moved to {entry_id}; files were not changed"),
    )?;
    session_forest_snapshot(db, session_id)
}

#[tauri::command]
fn add_project(path: String, state: State<AppState>) -> Result<BridgeState, BridgeError> {
    let clean = git::validate_repo(Path::new(&path))?;
    let name = Path::new(&clean)
        .file_name()
        .and_then(|x| x.to_str())
        .unwrap_or("Repository")
        .to_string();
    let id = Uuid::new_v4().to_string();
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
        params![id, name, clean, Utc::now().to_rfc3339()],
    )?;
    store::event(
        &db,
        "project",
        "project.added",
        &id,
        &format!("Added {name}"),
    )?;
    store::state(&db)
}

/// Scratch working directory for a chat that has no connected folder/repo.
fn chat_scratch_dir(state: &AppState, session_id: &str) -> PathBuf {
    state
        .database_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chats")
        .join(session_id)
}

fn chat_label(title: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("New chat")
        .to_string()
}

/// Create a repo-less workspace. A folder/git repo can be connected later.
#[tauri::command]
fn create_workspace(title: String, state: State<AppState>) -> Result<BridgeState, BridgeError> {
    let name = title.trim();
    if name.is_empty() {
        return Err(BridgeError::Invalid("Workspace name is required".into()));
    }
    let id = Uuid::new_v4().to_string();
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT INTO workspaces(id,title,status,created_at) VALUES(?1,?2,'idle',?3)",
        params![id, name, Utc::now().to_rfc3339()],
    )?;
    store::event(&db, "supervisor", "workspace.created", &id, &format!("Created workspace {name}"))?;
    store::state(&db)
}

/// Create a standalone direct chat (no workspace). Runs in a private scratch dir.
#[tauri::command]
fn create_chat(
    harness: Harness,
    model: Option<String>,
    title: Option<String>,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let adapter_id = store::harness_name(&harness);
    let id = Uuid::new_v4().to_string();
    let cwd = chat_scratch_dir(state.inner(), &id);
    let label = chat_label(title.as_deref());
    let db = state.db.lock().unwrap();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth) VALUES(?1,NULL,?2,?3,'idle','estimated',?4,'direct',?5,?6,0)",
        params![id, adapter_id, label, model, title, cwd.to_string_lossy()],
    )?;
    store::event(&db, "chat", "chat.created", &id, &format!("Created chat {label}"))?;
    store::state(&db)
}

/// Create an orchestrator session inside a workspace (the classic Bridge agent
/// that plans and delegates to workers). Multiple are allowed per workspace.
#[tauri::command]
fn create_workspace_session(
    workspace_id: String,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let id = Uuid::new_v4().to_string();
    let db = state.db.lock().unwrap();
    let ws_path: Option<String> = db
        .query_row("SELECT path FROM workspaces WHERE id=?1", params![workspace_id], |r| {
            r.get::<_, Option<String>>(0)
        })
        .ok()
        .flatten();
    let cwd = ws_path.unwrap_or_else(|| chat_scratch_dir(state.inner(), &id).to_string_lossy().to_string());
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,requested_tier,kind,cwd,depth) VALUES(?1,?2,?3,?4,'idle','estimated',?5,'orchestrator',?6,0)",
        params![id, workspace_id, orchestrator::HARNESS, orchestrator::SESSION_LABEL, orchestrator::TIER.as_str(), cwd],
    )?;
    store::event(&db, "supervisor", "session.created", &id, "New agent session")?;
    store::state(&db)
}

/// Change a direct chat's harness/model. Stops any running adapter so the next
/// message starts a fresh provider session with the new model.
#[tauri::command]
fn update_chat_model(
    session_id: String,
    harness: Harness,
    model: Option<String>,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let adapter_id = store::harness_name(&harness);
    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
        runtime.stop(adapters::ShutdownReason::Replaced);
    }
    let db = state.db.lock().unwrap();
    db.execute(
        "UPDATE sessions SET harness=?2,model=?3,provider_session_id=NULL,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1 AND kind='direct'",
        params![session_id, adapter_id, model],
    )?;
    store::state(&db)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SlashCommandResolve {
    name: String,
    harness: String,
    kind: String,
    /// When true, the frontend should switch the direct chat to `harness` before sending.
    switch_harness: bool,
}

/// Enumerate slash commands + skills from every signed-in provider, so the UI
/// can offer a labeled `/` menu.
#[tauri::command]
fn list_slash_commands(state: State<AppState>) -> Result<Vec<slash::SlashCommand>, BridgeError> {
    let available: std::collections::HashSet<String> = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect();
    Ok(slash::list_commands(&available))
}

/// Resolve a composer `/command` against the catalog so the UI can auto-switch
/// harness before sending.
#[tauri::command]
fn resolve_slash_command(
    text: String,
    session_id: String,
    state: State<AppState>,
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
    let available: std::collections::HashSet<String> = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect();
    let (kind, session_harness): (String, String) = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT kind, harness FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?
    };
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

/// Attach a folder (optionally a git repo) to a workspace as its working directory.
#[tauri::command]
fn connect_workspace_folder(
    workspace_id: String,
    path: String,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let folder = Path::new(&path);
    if !folder.is_dir() {
        return Err(BridgeError::Invalid("That folder no longer exists".into()));
    }
    let db = state.db.lock().unwrap();
    let (resolved_path, project_id, branch) = match git::validate_repo(folder) {
        Ok(root) => {
            let name = Path::new(&root)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Repository")
                .to_string();
            db.execute(
                "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
                params![Uuid::new_v4().to_string(), name, root, Utc::now().to_rfc3339()],
            )?;
            let project_id: Option<String> = db
                .query_row("SELECT id FROM projects WHERE path=?1", params![root], |r| r.get(0))
                .ok();
            (root.clone(), project_id, git::current_branch(folder))
        }
        Err(_) => (folder.to_string_lossy().to_string(), None, None),
    };
    db.execute(
        "UPDATE workspaces SET path=?2,project_id=?3,branch=?4 WHERE id=?1",
        params![workspace_id, resolved_path, project_id, branch],
    )?;
    store::event(&db, "supervisor", "workspace.connected", &workspace_id, &format!("Connected {resolved_path}"))?;
    store::state(&db)
}

#[tauri::command]
fn start_session(
    workspace_id: String,
    _harness: Option<Harness>,
    _model: Option<String>,
    app: AppHandle,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    // The user chooses neither harness nor model. Bridge starts its fast-tier
    // orchestrator and resolves the provider model through adapter inventory.
    let adapter_id = orchestrator::HARNESS;
    let session_label = orchestrator::SESSION_LABEL;
    let resolution = state
        .adapter_registry
        .resolve_model(adapter_id, orchestrator::TIER, None)?;
    let chosen_model = Some(resolution.actual_model);
    let db = state.db.lock().unwrap();
    let path: Option<String> = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get::<_, Option<String>>(0),
    )?;
    let existing: Option<(String, Option<String>)> = db.query_row(
        "SELECT id,provider_session_id FROM sessions WHERE workspace_id=?1 AND harness=?2 AND status IN ('idle','stopped','failed','ready','working','waiting') ORDER BY rowid DESC LIMIT 1",
        params![workspace_id, adapter_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    ).ok();
    let session_id = existing
        .as_ref()
        .map(|(id, _)| id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let stored_provider_id = existing
        .as_ref()
        .and_then(|(_, provider_id)| provider_id.clone());
    let checkpoint_context = if existing.is_some() {
        restoration::checkpoint_context(&db, &session_id)?
    } else {
        None
    };
    drop(db);
    let path = path.filter(|value| !value.is_empty()).unwrap_or_else(|| chat_scratch_dir(state.inner(), &session_id).to_string_lossy().to_string());
    std::fs::create_dir_all(&path)?;
    let process_is_hot = state.adapters.lock().unwrap().contains_key(&session_id);
    if process_is_hot {
        let current_model: Option<String> = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT model FROM sessions WHERE id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if current_model.as_deref() == chosen_model.as_deref() {
            let db = state.db.lock().unwrap();
            restoration::set_head_state(
                &db,
                &session_id,
                RestorationMode::Hot,
                if stored_provider_id.is_some() {
                    ResumeEligibility::Native
                } else {
                    ResumeEligibility::CheckpointRestored
                },
                stored_provider_id.as_deref(),
            )?;
            return store::state(&db);
        }
        let db = state.db.lock().unwrap();
        record_shutdown_reason(&db, &session_id, adapters::ShutdownReason::Replaced)?;
        drop(db);
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop(adapters::ShutdownReason::Replaced);
        }
    }

    // The orchestrator is depth 0. It gets the routing briefing plus the shared
    // delegation protocol so it can spawn workers itself.
    let orchestrator_instructions = format!(
        "{}\n\n{}",
        orchestrator::briefing(),
        delegation::protocol(0)
    );
    let plan = restoration::select_plan(
        false,
        stored_provider_id.as_deref(),
        state.adapter_registry.supports_native_resume(adapter_id),
        checkpoint_context.is_some(),
    );
    let start_fresh = |instructions: &str| {
        state.adapter_registry.start(
            adapter_id,
            adapters::StartRequest {
                cwd: &path,
                model: chosen_model.as_deref(),
                effort: None,
                instructions: Some(instructions),
                write_mode: None,
            },
        )
    };
    let checkpoint_instructions = checkpoint_context
        .as_ref()
        .map(|context| format!("{orchestrator_instructions}\n\n{context}"));
    let (started, restoration_mode, resume_eligibility) = match plan {
        restoration::RestorationPlan::Native => {
            let provider_id = stored_provider_id
                .as_deref()
                .expect("native plan has provider id");
            match state.adapter_registry.resume(
                adapter_id,
                adapters::ResumeRequest {
                    provider_session_id: provider_id,
                    cwd: &path,
                    model: chosen_model.as_deref(),
                    effort: None,
                    instructions: Some(orchestrator_instructions.as_str()),
                    write_mode: None,
                },
            ) {
                Ok(started) => (started, RestorationMode::Native, ResumeEligibility::Native),
                Err(error) => {
                    let db = state.db.lock().unwrap();
                    restoration::record_resume_failed(&db, &session_id, &error.to_string())?;
                    drop(db);
                    match restoration::fallback_after_failure(
                        restoration::RestorationPlan::Native,
                        checkpoint_instructions.is_some(),
                    ) {
                        Some(restoration::RestorationPlan::CheckpointRestored) => {
                            match start_fresh(
                                checkpoint_instructions
                                    .as_deref()
                                    .expect("checkpoint fallback has stored context"),
                            ) {
                                Ok(started) => (
                                    started,
                                    RestorationMode::CheckpointRestored,
                                    ResumeEligibility::CheckpointRestored,
                                ),
                                Err(error) => {
                                    let db = state.db.lock().unwrap();
                                    restoration::record_checkpoint_restore_failed(
                                        &db,
                                        &session_id,
                                        &error.to_string(),
                                    )?;
                                    drop(db);
                                    (
                                        start_fresh(&orchestrator_instructions)?,
                                        RestorationMode::Fresh,
                                        ResumeEligibility::Fresh,
                                    )
                                }
                            }
                        }
                        Some(restoration::RestorationPlan::Fresh) => (
                            start_fresh(&orchestrator_instructions)?,
                            RestorationMode::Fresh,
                            ResumeEligibility::Fresh,
                        ),
                        _ => unreachable!("native failure has a deterministic fallback"),
                    }
                }
            }
        }
        restoration::RestorationPlan::CheckpointRestored => {
            match start_fresh(
                checkpoint_instructions
                    .as_deref()
                    .expect("checkpoint plan has stored context"),
            ) {
                Ok(started) => (
                    started,
                    RestorationMode::CheckpointRestored,
                    ResumeEligibility::CheckpointRestored,
                ),
                Err(error) => {
                    debug_assert_eq!(
                        restoration::fallback_after_failure(
                            restoration::RestorationPlan::CheckpointRestored,
                            true,
                        ),
                        Some(restoration::RestorationPlan::Fresh)
                    );
                    let db = state.db.lock().unwrap();
                    restoration::record_checkpoint_restore_failed(
                        &db,
                        &session_id,
                        &error.to_string(),
                    )?;
                    drop(db);
                    (
                        start_fresh(&orchestrator_instructions)?,
                        RestorationMode::Fresh,
                        ResumeEligibility::Fresh,
                    )
                }
            }
        }
        restoration::RestorationPlan::Fresh => (
            start_fresh(&orchestrator_instructions)?,
            RestorationMode::Fresh,
            ResumeEligibility::Fresh,
        ),
        restoration::RestorationPlan::Hot => unreachable!("hot sessions returned above"),
    };
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let reader = started.reader;
    let started_at = Utc::now().to_rfc3339();
    let db = state.db.lock().unwrap();
    if existing.is_some() {
        db.execute(
            "UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported',model=?4,requested_tier=?5,label=?6,depth=0,parent_session_id=NULL,trace_id=COALESCE(trace_id,lower(hex(randomblob(16)))) WHERE id=?1",
            params![
                session_id,
                started_at,
                thread_id,
                chosen_model,
                orchestrator::TIER.as_str(),
                session_label
            ],
        )?;
    } else {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,provider_session_id,model,requested_tier,depth,trace_id) VALUES(?1,?2,?3,?4,'working',?5,'reported',?6,?7,?8,0,?9)",
            params![
                session_id,
                workspace_id,
                adapter_id,
                session_label,
                started_at,
                thread_id,
                chosen_model,
                orchestrator::TIER.as_str(),
                Uuid::new_v4().simple().to_string()
            ],
        )?;
    }
    restoration::set_head_state(
        &db,
        &session_id,
        restoration_mode,
        resume_eligibility,
        Some(&thread_id),
    )?;
    db.execute(
        "UPDATE workspaces SET status='working' WHERE id=?1",
        params![workspace_id],
    )?;
    store::event(
        &db,
        "adapter",
        "session.started",
        &session_id,
        &format!(
            "Started {session_label} on {} with {} restoration",
            chosen_model.as_deref().unwrap_or("default"),
            restoration_mode.as_str()
        ),
    )?;
    if adapter_id == orchestrator::HARNESS {
        let context = agent::NormalizedEvent {
            kind: "session.context".into(),
            item_id: Some("orchestrator-briefing".into()),
            role: Some("system".into()),
            status: Some("ready".into()),
            title: Some("Orchestrator routing policy".into()),
            text: Some(orchestrator::briefing()),
            data: serde_json::json!({
                "source": "capability-policy",
                "requestedTier": orchestrator::TIER,
                "runtimeModel": chosen_model
            }),
        };
        let _ = store::session_event(
            &db,
            &session_id,
            &context,
            &serde_json::json!({"adapter": adapter_id, "hidden": true}),
        );
    }
    for message in &started.startup_messages {
        persist_agent_value(
            &db,
            &state.adapter_registry,
            adapter_id,
            &session_id,
            message,
        )?;
    }
    drop(db);
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), started.runtime);

    spawn_reader_thread(
        app.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    let _ = app.emit("state-changed", ());
    store::state(&state.db.lock().unwrap())
}

/// Start (or hot-return) a session by id. A `direct` chat runs the stored
/// harness/model with no briefing; an `orchestrator` session runs codex with the
/// routing briefing + delegation protocol (workers enabled via the reader gate).
#[tauri::command]
fn start_chat(
    session_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let (harness, kind, model, cwd_col, workspace_id, provider_id, effort): (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT harness,kind,model,cwd,workspace_id,provider_session_id,effort FROM sessions WHERE id=?1",
            params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )?
    };
    if state.adapters.lock().unwrap().contains_key(&session_id) {
        return store::state(&state.db.lock().unwrap());
    }
    let is_orchestrator = kind == "orchestrator";
    let cwd = match cwd_col.filter(|value| !value.is_empty()) {
        Some(value) => value,
        None => {
            let workspace_path = workspace_id.as_ref().and_then(|workspace| {
                state
                    .db
                    .lock()
                    .unwrap()
                    .query_row(
                        "SELECT path FROM workspaces WHERE id=?1",
                        params![workspace],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten()
            });
            workspace_path
                .unwrap_or_else(|| chat_scratch_dir(state.inner(), &session_id).to_string_lossy().to_string())
        }
    };
    std::fs::create_dir_all(&cwd)?;
    let adapter_id: &str = if is_orchestrator { orchestrator::HARNESS } else { harness.as_str() };
    let tier = if is_orchestrator { orchestrator::TIER } else { CapabilityTier::Fast };
    let chosen_model = if is_orchestrator {
        state.adapter_registry.resolve_model(adapter_id, tier, None).ok().map(|resolution| resolution.actual_model)
    } else {
        match model {
            Some(value) if !value.is_empty() => Some(value),
            _ => state.adapter_registry.resolve_model(adapter_id, tier, None).ok().map(|resolution| resolution.actual_model),
        }
    };
    let orchestrator_instructions = if is_orchestrator {
        Some(format!("{}\n\n{}", orchestrator::briefing(), delegation::protocol(0)))
    } else {
        None
    };
    let instructions_ref = orchestrator_instructions.as_deref();
    let effort_ref = effort.as_deref().filter(|value| !value.is_empty());
    let resumable = provider_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .filter(|_| state.adapter_registry.supports_native_resume(adapter_id));
    let (started, mode, eligibility) = match resumable {
        Some(provider) => match state.adapter_registry.resume(
            adapter_id,
            adapters::ResumeRequest {
                provider_session_id: provider,
                cwd: &cwd,
                model: chosen_model.as_deref(),
                effort: effort_ref,
                instructions: instructions_ref,
                write_mode: None,
            },
        ) {
            Ok(started) => (started, RestorationMode::Native, ResumeEligibility::Native),
            Err(_) => (
                state.adapter_registry.start(
                    adapter_id,
                    adapters::StartRequest {
                        cwd: &cwd,
                        model: chosen_model.as_deref(),
                        effort: effort_ref,
                        instructions: instructions_ref,
                        write_mode: None,
                    },
                )?,
                RestorationMode::Fresh,
                ResumeEligibility::Fresh,
            ),
        },
        None => (
            state.adapter_registry.start(
                adapter_id,
                adapters::StartRequest {
                    cwd: &cwd,
                    model: chosen_model.as_deref(),
                    effort: effort_ref,
                    instructions: instructions_ref,
                    write_mode: None,
                },
            )?,
            RestorationMode::Fresh,
            ResumeEligibility::Fresh,
        ),
    };
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let reader = started.reader;
    let started_at = Utc::now().to_rfc3339();
    {
        let db = state.db.lock().unwrap();
        if is_orchestrator {
            db.execute(
                "UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported',model=?4,cwd=?5,harness=?6,requested_tier=?7,label=?8,depth=0 WHERE id=?1",
                params![session_id, started_at, thread_id, chosen_model, cwd, adapter_id, tier.as_str(), orchestrator::SESSION_LABEL],
            )?;
        } else {
            db.execute(
                "UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported',model=?4,cwd=?5 WHERE id=?1",
                params![session_id, started_at, thread_id, chosen_model, cwd],
            )?;
        }
        restoration::set_head_state(&db, &session_id, mode, eligibility, Some(&thread_id))?;
        if let Some(workspace) = &workspace_id {
            let _ = db.execute(
                "UPDATE workspaces SET status='working' WHERE id=?1",
                params![workspace],
            );
        }
        store::event(
            &db,
            "adapter",
            "session.started",
            &session_id,
            &format!("Started {} on {}", if is_orchestrator { "orchestrator" } else { "chat" }, chosen_model.as_deref().unwrap_or("default")),
        )?;
        if is_orchestrator {
            let context = agent::NormalizedEvent {
                kind: "session.context".into(),
                item_id: Some("orchestrator-briefing".into()),
                role: Some("system".into()),
                status: Some("ready".into()),
                title: Some("Orchestrator routing policy".into()),
                text: Some(orchestrator::briefing()),
                data: serde_json::json!({"source": "capability-policy", "requestedTier": tier, "runtimeModel": chosen_model}),
            };
            let _ = store::session_event(&db, &session_id, &context, &serde_json::json!({"adapter": adapter_id, "hidden": true}));
        }
        for message in &started.startup_messages {
            persist_agent_value(&db, &state.adapter_registry, adapter_id, &session_id, message)?;
        }
    }
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), started.runtime);
    spawn_reader_thread(app.clone(), session_id.clone(), started_at, current_turn, reader);
    let _ = app.emit("state-changed", ());
    store::state(&state.db.lock().unwrap())
}

/// Drive one structured session's stdout: normalize every frame, then on exit
/// mark the session stopped and unblock any parent that was waiting on it.
fn spawn_reader_thread(
    app: AppHandle,
    session_id: String,
    launch_started_at: String,
    current_turn: Arc<Mutex<Option<String>>>,
    mut reader: Box<dyn BufRead + Send>,
) {
    thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                        handle_agent_value(&app, &session_id, &current_turn, &value);
                    }
                }
            }
        }
        let state = app.state::<AppState>();
        let is_current_launch = state
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT started_at=?2 FROM sessions WHERE id=?1",
                params![session_id, launch_started_at],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false);
        if !is_current_launch {
            return;
        }
        state.adapters.lock().unwrap().remove(&session_id);
        notify_parent_on_worker_exit(&app, &session_id);
        let db = state.db.lock().unwrap();
        let is_worker = store::worker_runtime(&db, &session_id)
            .ok()
            .flatten()
            .is_some();
        let workspace: Option<String> = db
            .query_row(
                "SELECT workspace_id FROM sessions WHERE id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok();
        if !is_worker {
            let _ = db.execute("UPDATE sessions SET status='stopped',ended_at=?2,active_turn_id=NULL WHERE id=?1 AND status IN ('working','waiting')", params![session_id,Utc::now().to_rfc3339()]);
        }
        if let Some(workspace) = workspace {
            let _=db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'stopped' END WHERE id=?1",params![workspace]);
        }
        drop(db);
        let _ = app.emit("state-changed", ());
    });
}

fn persist_agent_value(
    db: &Connection,
    registry: &adapters::AdapterRegistry,
    adapter_id: &str,
    session_id: &str,
    value: &serde_json::Value,
) -> Result<Vec<AgentEvent>, BridgeError> {
    let normalized = registry.normalize(adapter_id, value);
    normalized
        .iter()
        .map(|event| {
            store::session_event(
                db,
                session_id,
                event,
                &serde_json::json!({"adapter":adapter_id,"method":value.get("method")}),
            )
        })
        .collect()
}

fn handle_agent_value(
    app: &AppHandle,
    session_id: &str,
    current_turn: &Arc<Mutex<Option<String>>>,
    value: &serde_json::Value,
) {
    // Codex account rate-limit frames (the reply to `account/rateLimits/read`
    // and its rolling push) are subscription telemetry, not conversation. Route
    // them straight to the ambient usage channel without persisting.
    if let Some(rate_limits) = codex_rate_limits_from_frame(value) {
        emit_account_usage(app, "codex", rate_limits);
        return;
    }
    let state = app.state::<AppState>();
    let mut pending_directives: Vec<(delegation::DelegationRequest, String)> = Vec::new();
    let mut pending_ui_events: Vec<AgentEvent> = Vec::new();
    let mut turn_completed = false;
    let mut checkpoint_prompt_after_turn: Option<String> = None;
    let mut checkpoint_response_seen = false;
    let mut checkpoint_turn_handled = false;
    let mut finish_checkpointing = false;
    let mut finish_requested_shutdown = false;
    let mut recover_compaction = false;

    {
        let db = state.db.lock().unwrap();
        let session_context: Option<(Option<String>, String, i64, Option<String>, String)> = db
            .query_row(
                "SELECT workspace_id,harness,COALESCE(depth,0),active_turn_id,kind FROM sessions WHERE id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .ok();
        let Some((workspace_id, adapter_id, own_depth, stored_turn_id, session_kind)) =
            session_context
        else {
            return;
        };
        // Direct chats are single-agent: no worker delegation and no auto-compaction.
        let is_direct = session_kind == "direct";
        let observed_turn_id = current_turn
            .lock()
            .unwrap()
            .clone()
            .or(stored_turn_id)
            .or_else(|| {
                state
                    .delegations
                    .lock()
                    .unwrap()
                    .last_turn_by_session
                    .get(session_id)
                    .cloned()
            });
        let normalized = state.adapter_registry.normalize(&adapter_id, value);
        for event in &normalized {
            match event.kind.as_str() {
                "turn.started" => {
                    let turn_id = event
                        .data
                        .pointer("/turn/id")
                        .or_else(|| event.data.get("turnId"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                    *current_turn.lock().unwrap() = turn_id.clone();
                    if let Some(turn_id) = &turn_id {
                        state
                            .delegations
                            .lock()
                            .unwrap()
                            .last_turn_by_session
                            .insert(session_id.into(), turn_id.clone());
                    }
                    let checkpointing = db
                        .query_row(
                            "SELECT status='checkpointing' FROM sessions WHERE id=?1",
                            params![session_id],
                            |row| row.get::<_, bool>(0),
                        )
                        .unwrap_or(false);
                    let _ = if checkpointing {
                        db.execute(
                            "UPDATE sessions SET active_turn_id=?2 WHERE id=?1",
                            params![session_id, turn_id],
                        )
                    } else {
                        db.execute(
                            "UPDATE sessions SET status='working',active_turn_id=?2 WHERE id=?1",
                            params![session_id, turn_id],
                        )
                    };
                }
                "turn.completed" => {
                    turn_completed = true;
                    *current_turn.lock().unwrap() = None;
                    let checkpointing_worker = own_depth > 0
                        && store::worker_runtime(&db, session_id)
                            .ok()
                            .flatten()
                            .is_some_and(|runtime| runtime.lifecycle_state == "checkpointing");
                    if !checkpointing_worker {
                        let _ = db.execute(
                            "UPDATE sessions SET status='ready',active_turn_id=NULL WHERE id=?1",
                            params![session_id],
                        );
                    }
                    if let Some(workspace_id) = &workspace_id {
                        let _ = db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'ready' END WHERE id=?1",params![workspace_id]);
                    }
                }
                "approval.requested" => {
                    if own_depth > 0 {
                        let _ = session_supervisor::SessionSupervisor::transition(
                            &db,
                            session_id,
                            worker_lifecycle::WorkerLifecycleState::Waiting,
                            Some("approval_requested"),
                        );
                    } else {
                        let _ = db.execute(
                            "UPDATE sessions SET status='waiting' WHERE id=?1",
                            params![session_id],
                        );
                    }
                    if let Some(workspace_id) = &workspace_id {
                        let _ = db.execute(
                            "UPDATE workspaces SET status='waiting' WHERE id=?1",
                            params![workspace_id],
                        );
                    }
                }
                "usage.updated" => {
                    let scope = workspace_id.as_deref().unwrap_or(session_id);
                    let _ = policy::record_provider_usage(
                        &db,
                        scope,
                        session_id,
                        observed_turn_id.as_deref(),
                        &format!("provider.{adapter_id}"),
                        &event.data,
                    );
                }
                "error" if event.status.as_deref() == Some("failed") => {
                    if own_depth > 0 {
                        let lifecycle = store::worker_runtime(&db, session_id)
                            .ok()
                            .flatten()
                            .map(|runtime| runtime.lifecycle_state);
                        if lifecycle.as_deref() == Some("waiting") {
                            let _ = session_supervisor::SessionSupervisor::transition(
                                &db,
                                session_id,
                                worker_lifecycle::WorkerLifecycleState::Working,
                                Some("approval_aborted_by_error"),
                            );
                        }
                        let _ = session_supervisor::SessionSupervisor::transition(
                            &db,
                            session_id,
                            worker_lifecycle::WorkerLifecycleState::Failed,
                            Some("provider_error"),
                        );
                    } else {
                        let _ = db.execute(
                            "UPDATE sessions SET status='failed' WHERE id=?1",
                            params![session_id],
                        );
                    }
                    if let Some(workspace_id) = &workspace_id {
                        let _ = db.execute(
                            "UPDATE workspaces SET status='failed' WHERE id=?1",
                            params![workspace_id],
                        );
                    }
                }
                _ => {}
            }
        }
        for mut normalized_event in normalized {
            // A completed assistant message may carry delegation directives.
            // Spawn the workers (after the lock is released) and strip the raw
            // directive block so the conversation shows prose, not machine JSON.
            if !is_direct
                && normalized_event.kind == "message.completed"
                && normalized_event.role.as_deref() == Some("assistant")
            {
                if let Some(text) = normalized_event.text.clone() {
                    match delegation::parse_delegation_requests(&text) {
                        delegation::ParseOutcome::Parsed(requests) => {
                            let item_id = normalized_event.item_id.clone().unwrap_or_default();
                            let is_new = store::claim_delegation_receipt(
                                &db,
                                session_id,
                                &item_id,
                            )
                            .unwrap_or(false);
                            let mut accepted_count = 0;
                            if is_new {
                                accepted_count = requests.len();
                                let turn_id = observed_turn_id
                                    .clone()
                                    .or_else(|| normalized_event.item_id.clone())
                                    .unwrap_or_else(|| format!("turn-{}", Uuid::new_v4()));
                                pending_directives.extend(
                                    requests
                                        .into_iter()
                                        .map(|request| (request, turn_id.clone())),
                                );
                            }
                            let stripped = delegation::strip_directives(&text);
                            normalized_event.text = Some(if stripped.is_empty() {
                                if accepted_count > 0 {
                                    "_Delegating to a worker…_".to_owned()
                                } else {
                                    "_Delegation request already processed._".to_owned()
                                }
                            } else {
                                stripped
                            });
                        }
                        delegation::ParseOutcome::Invalid { reason, .. } => {
                            let _ = store::event(
                                &db,
                                "delegation",
                                "delegation.request.invalid",
                                session_id,
                                &reason,
                            );
                            let stripped = delegation::strip_directives(&text);
                            normalized_event.text = Some(if stripped.is_empty() {
                                format!("_Invalid delegation request: {reason}_")
                            } else {
                                stripped
                            });
                        }
                        delegation::ParseOutcome::Absent => {}
                    }
                }
            }
            if let Ok(event) = store::session_event(
                &db,
                session_id,
                &normalized_event,
                &serde_json::json!({"adapter":adapter_id,"method":value.get("method")}),
            ) {
                pending_ui_events.push(event);
            }
            let pending_compaction = compaction_controller::CompactionController::pending(
                &db,
                session_id,
            )
            .ok()
            .flatten();
            if normalized_event.kind == "message.completed"
                && normalized_event.role.as_deref() == Some("assistant")
                && pending_compaction.is_some()
            {
                checkpoint_response_seen = true;
                checkpoint_turn_handled = true;
                let output = normalized_event.text.as_deref().unwrap_or_default();
                match compaction_controller::CompactionController::handle_output(
                    &db,
                    session_id,
                    output,
                ) {
                    Ok(compaction_controller::CheckpointOutcome::Repair { prompt }) => {
                        checkpoint_prompt_after_turn = Some(prompt);
                    }
                    Ok(compaction_controller::CheckpointOutcome::Completed { .. }) => {
                        finish_checkpointing = own_depth > 0;
                        finish_requested_shutdown = pending_compaction.is_some_and(|pending| {
                            pending.reason
                                == compaction_controller::CompactionReason::BeforeShutdown
                        });
                    }
                    Ok(compaction_controller::CheckpointOutcome::Failed) => {
                        recover_compaction = true;
                        finish_checkpointing = own_depth > 0;
                        finish_requested_shutdown = pending_compaction.is_some_and(|pending| {
                            pending.reason
                                == compaction_controller::CompactionReason::BeforeShutdown
                        });
                    }
                    Ok(compaction_controller::CheckpointOutcome::NotPending) | Err(_) => {}
                }
            }
        }
        if turn_completed && checkpoint_prompt_after_turn.is_none() {
            if let Ok(Some(pending)) =
                compaction_controller::CompactionController::pending(&db, session_id)
            {
                if pending.attempt == 1 {
                    checkpoint_turn_handled = true;
                    checkpoint_prompt_after_turn = Some(
                        compaction_controller::CompactionController::checkpoint_prompt(
                            session_id,
                            &pending,
                            Some("repair the invalid checkpoint response"),
                        ),
                    );
                }
            }
        }
        if turn_completed
            && checkpoint_prompt_after_turn.is_none()
            && !checkpoint_response_seen
            && own_depth == 0
            && !is_direct
        {
            if let Ok(Some(prompt)) = begin_pressure_compaction(&db, session_id) {
                checkpoint_prompt_after_turn = Some(prompt);
            }
        }
    }

    if turn_completed {
        if let Some(prompt) = checkpoint_prompt_after_turn {
            if let Err(error) = send_internal_checkpoint_turn(app, session_id, &prompt) {
                let db = state.db.lock().unwrap();
                let pending = compaction_controller::CompactionController::pending(&db, session_id)
                    .ok()
                    .flatten();
                let attempt = pending.as_ref().map_or(0, |pending| pending.attempt);
                let shutdown = pending.is_some_and(|pending| {
                    pending.reason == compaction_controller::CompactionReason::BeforeShutdown
                });
                let _ = compaction_controller::CompactionController::record_failure(
                    &db,
                    session_id,
                    &format!("checkpoint turn could not start: {error}"),
                    attempt,
                );
                finish_checkpointing = true;
                finish_requested_shutdown = shutdown;
            }
        }
    }
    if finish_checkpointing {
        finish_worker_checkpoint(app, session_id, adapters::ShutdownReason::Completed);
    }
    if recover_compaction {
        let _ = run_compaction_recovery(app, session_id);
    }
    if finish_requested_shutdown {
        finish_orchestrator_shutdown(app, session_id, adapters::ShutdownReason::UserStopped);
    }

    for (directive, turn_id) in &pending_directives {
        let _ = launch_worker(app, session_id, turn_id, directive, true);
    }
    // When this session's own turn ends and it is not waiting on any child
    // worker, hand its result up to its parent (no-op if it has no parent).
    if turn_completed && !checkpoint_response_seen && !checkpoint_turn_handled {
        let idle = store::outstanding_children(&state.db.lock().unwrap(), session_id)
            .unwrap_or(0)
            == 0;
        if idle {
            forward_turn_result(app, session_id);
        }
    }
    for event in pending_ui_events {
        let _ = app.emit("agent-event", event);
    }
    let _ = app.emit("state-changed", ());
}

fn begin_pressure_compaction(
    db: &Connection,
    session_id: &str,
) -> Result<Option<String>, BridgeError> {
    let context_percent = db
        .query_row(
            "SELECT CAST(context_percent AS REAL) FROM usage_ledger WHERE session_id=?1 AND context_percent IS NOT NULL ORDER BY id DESC LIMIT 1",
            params![session_id],
            |row| row.get::<_, f64>(0),
        )
        .ok();
    let branch = session_forest::SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let last_boundary = branch
        .iter()
        .rposition(|entry| entry.kind == "compaction")
        .map_or(0, |index| index + 1);
    let meaningful = branch[last_boundary..].iter().any(|entry| {
        matches!(
            entry.kind.as_str(),
            "user.message" | "assistant.message" | "worker.result" | "tool.completed"
        )
    });
    let trigger = compaction_controller::TriggerState {
        reason: compaction_controller::CompactionReason::ContextPressure,
        context_percent,
        projected_tokens_with_reserve: None,
        context_window_tokens: None,
        has_valid_typed_result: false,
        one_shot_worker: false,
        tool_call_active: false,
        approval_active: false,
        has_meaningful_new_work: meaningful,
        wall_clock_only: false,
    };
    let Ok(reason) = compaction_controller::decide(&trigger) else {
        return Ok(None);
    };
    let tokens = compaction_controller::active_token_estimate(db, session_id)?;
    compaction_controller::CompactionController::begin(db, session_id, reason, tokens)
}

fn send_internal_checkpoint_turn(
    app: &AppHandle,
    session_id: &str,
    prompt: &str,
) -> Result<(), BridgeError> {
    let state = app.state::<AppState>();
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(session_id)
        .ok_or_else(|| BridgeError::Invalid("checkpoint agent process is not running".into()))?;
    runtime.send_turn(prompt)?;
    drop(adapters);
    store::event(
        &state.db.lock().unwrap(),
        "compaction",
        "checkpoint.turn_started",
        session_id,
        "Checkpoint-only structured turn started",
    )?;
    Ok(())
}

fn finish_worker_checkpoint(app: &AppHandle, session_id: &str, reason: adapters::ShutdownReason) {
    let state = app.state::<AppState>();
    let should_stop = {
        let db = state.db.lock().unwrap();
        let checkpointing = store::worker_runtime(&db, session_id)
            .ok()
            .flatten()
            .is_some_and(|runtime| runtime.lifecycle_state == "checkpointing");
        if checkpointing {
            let _ = session_supervisor::SessionSupervisor::transition(
                &db,
                session_id,
                worker_lifecycle::WorkerLifecycleState::Stopped,
                Some("checkpoint_turn_finished"),
            );
            let _ = db.execute(
                "UPDATE worker_leases SET lease_status='checkpointed',updated_at=?2 WHERE session_id=?1",
                params![session_id, Utc::now().to_rfc3339()],
            );
        }
        checkpointing
    };
    if should_stop {
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(session_id) {
            runtime.stop(reason);
        }
    }
}

fn finish_orchestrator_shutdown(
    app: &AppHandle,
    session_id: &str,
    reason: adapters::ShutdownReason,
) {
    let state = app.state::<AppState>();
    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(session_id) {
        runtime.stop(reason);
    }
    let db = state.db.lock().unwrap();
    let workspace_id = db
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let _ = record_shutdown_reason(&db, session_id, reason);
    let _ = db.execute(
        "UPDATE sessions SET status='stopped',ended_at=?2,active_turn_id=NULL WHERE id=?1",
        params![session_id, Utc::now().to_rfc3339()],
    );
    if let Some(workspace_id) = workspace_id {
        let _ = db.execute(
            "UPDATE workspaces SET status='stopped' WHERE id=?1",
            params![workspace_id],
        );
    }
    let _ = app.emit("state-changed", ());
}

fn run_compaction_recovery(app: &AppHandle, session_id: &str) -> Result<(), BridgeError> {
    let state = app.state::<AppState>();
    let workspace_path: String = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT w.path FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
            params![session_id],
            |row| row.get(0),
        )?
    };
    let git_status = worker_guard::tracked_status(Path::new(&workspace_path)).unwrap_or_default();
    compaction_controller::CompactionController::reconstruct_from_normalized_events_and_git(
        &state.db.lock().unwrap(),
        session_id,
        &git_status,
    )?;
    Ok(())
}

/// Spawn a child worker session in the parent's workspace and hand it its task.
struct WorkerLaunchReservation {
    session_id: String,
    workspace_id: String,
    depth: i64,
    path: String,
    branch: String,
    actual_model: String,
    outcome: policy::PolicyOutcome,
    reuse_existing: bool,
}

enum WorkerReservationOutcome {
    Reserved(WorkerLaunchReservation),
    Queued,
    Blocked,
}

enum WorkerLaunchOutcome {
    Launched(String),
    Queued,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkerActivation {
    Fresh,
    Native,
    CheckpointRestored,
}

fn record_model_resolution_warning(
    db: &Connection,
    parent_session_id: &str,
    resolution: &adapters::ModelResolution,
) -> Result<(), BridgeError> {
    if let Some(warning) = &resolution.warning {
        store::event(
            db,
            "capability",
            "capability.model_fallback",
            parent_session_id,
            warning,
        )?;
    }
    Ok(())
}

fn reserve_worker_launch_outcome(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    actual_model: &str,
    queue_on_block: bool,
) -> Result<WorkerReservationOutcome, BridgeError> {
    let route = policy_coordinator::PolicyCoordinator::decide_worker_route(
        db,
        parent_session_id,
        turn_id,
        directive,
        true,
    )?;
    let policy_coordinator::WorkerRouteContext {
        workspace_id,
        parent_depth,
        path,
        branch,
        outcome,
    } = route;
    match &outcome.decision {
        policy::RouteDecision::Queue => {
            if queue_on_block {
                worker_pool::WorkerPool::enqueue(
                    db,
                    parent_session_id,
                    &workspace_id,
                    turn_id,
                    directive,
                    actual_model,
                )?;
            }
            return Ok(if queue_on_block {
                WorkerReservationOutcome::Queued
            } else {
                WorkerReservationOutcome::Blocked
            });
        }
        policy::RouteDecision::ResumeWorker { session_id } => {
            let runtime = store::worker_runtime(db, session_id)?
                .ok_or_else(|| BridgeError::Invalid(format!("warm worker {session_id} has no runtime record")))?;
            let worker_path = runtime.worktree_path.unwrap_or_else(|| path.clone());
            let worker_branch = runtime.worktree_branch.unwrap_or_else(|| branch.clone());
            return Ok(WorkerReservationOutcome::Reserved(WorkerLaunchReservation {
                session_id: session_id.clone(),
                workspace_id,
                depth: parent_depth + 1,
                path: worker_path,
                branch: worker_branch,
                actual_model: actual_model.into(),
                outcome,
                reuse_existing: true,
            }));
        }
        policy::RouteDecision::SpawnWorker(_) => {}
        _ => return Ok(WorkerReservationOutcome::Blocked),
    }

    let session_id = Uuid::new_v4().to_string();
    let depth = parent_depth + 1;
    let harness = directive.runtime_harness();
    let effort = directive.effort.as_str();
    let now = Utc::now().to_rfc3339();
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,model,requested_tier,effort,parent_session_id,depth) VALUES(?1,?2,?3,?4,'starting',?5,'reported',?6,?7,?8,?9,?10)",
        params![
            session_id,
            workspace_id,
            harness,
            directive.label(),
            now,
            actual_model,
            directive.capability_tier.as_str(),
            effort,
            parent_session_id,
            depth,
        ],
    )?;
    let compatibility_key =
        worker_pool::WorkerCompatibilityKey::for_request(&workspace_id, directive)?.encode()?;
    store::upsert_worker_lease(
        &transaction,
        &WorkerLease {
            session_id: session_id.clone(),
            workspace_id: workspace_id.clone(),
            role: policy::role_name(directive.role).into(),
            capability_tier: policy::tier_name(directive.capability_tier).into(),
            task_family: policy::role_name(directive.role).into(),
            owned_paths: serde_json::json!(directive.owned_paths),
            write_mode: policy::write_mode_name(directive.write_mode).into(),
            lease_status: "active".into(),
            expires_at: None,
            created_at: now.clone(),
            updated_at: now,
        },
    )?;
    store::upsert_worker_runtime(
        &transaction,
        &WorkerRuntimeRecord {
            session_id: session_id.clone(),
            parent_session_id: parent_session_id.to_owned(),
            lifecycle_state: "starting".into(),
            task_family: worker_pool::task_family(directive),
            compatibility_key,
            result_status: "pending".into(),
            retry_count: 0,
            warm_until: None,
            worktree_path: None,
            worktree_branch: None,
            last_result: None,
            updated_at: Utc::now().to_rfc3339(),
        },
    )?;
    policy::record_spawn_usage(
        &transaction,
        &workspace_id,
        &session_id,
        turn_id,
        &outcome,
        directive.capability_tier,
    )?;
    let outbox_created_at = Utc::now().to_rfc3339();
    store::enqueue_outbox(
        &transaction,
        &OutboxMessage {
            id: Uuid::new_v4().to_string(),
            destination: "integration".into(),
            event_type: "worker.spawned".into(),
            payload: serde_json::json!({
                "sessionId": session_id,
                "parentSessionId": parent_session_id,
                "turnId": turn_id,
            }),
            idempotency_key: format!("worker-spawn:{parent_session_id}:{turn_id}:{session_id}"),
            status: "pending".into(),
            attempt_count: 0,
            next_attempt_at: outbox_created_at.clone(),
            last_error: None,
            created_at: outbox_created_at,
            delivered_at: None,
        },
    )?;
    transaction.commit()?;
    Ok(WorkerReservationOutcome::Reserved(WorkerLaunchReservation {
        session_id,
        workspace_id,
        depth,
        path,
        branch,
        actual_model: actual_model.into(),
        outcome,
        reuse_existing: false,
    }))
}

#[cfg(test)]
fn reserve_worker_launch(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    actual_model: &str,
    queue_on_block: bool,
) -> Result<Option<WorkerLaunchReservation>, BridgeError> {
    Ok(match reserve_worker_launch_outcome(
        db,
        parent_session_id,
        turn_id,
        directive,
        actual_model,
        queue_on_block,
    )? {
        WorkerReservationOutcome::Reserved(reservation) => Some(reservation),
        WorkerReservationOutcome::Queued | WorkerReservationOutcome::Blocked => None,
    })
}

fn launch_worker_outcome(
    app: &AppHandle,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    queue_on_block: bool,
) -> WorkerLaunchOutcome {
    let state = app.state::<AppState>();
    let harness = directive.runtime_harness();
    let resolution = match state.adapter_registry.resolve_model(
        &harness,
        directive.capability_tier,
        directive.model.as_deref(),
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "capability",
                "capability.resolution_failed",
                parent_session_id,
                &error.to_string(),
            );
            drop(db);
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Failed;
        }
    };
    let reservation = {
        let db = state.db.lock().unwrap();
        let _ = record_model_resolution_warning(&db, parent_session_id, &resolution);
        reserve_worker_launch_outcome(
            &db,
            parent_session_id,
            turn_id,
            directive,
            &resolution.actual_model,
            queue_on_block,
        )
    };
    let mut reservation = match reservation {
        Ok(WorkerReservationOutcome::Reserved(reservation)) => reservation,
        Ok(WorkerReservationOutcome::Queued) => {
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Queued;
        }
        Ok(WorkerReservationOutcome::Blocked) => {
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Failed;
        }
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "policy",
                "policy.decision_failed",
                parent_session_id,
                &error.to_string(),
            );
            drop(db);
            let _ = app.emit("state-changed", ());
            return WorkerLaunchOutcome::Failed;
        }
    };
    let requires_child_worktree = matches!(
        &reservation.outcome.decision,
        policy::RouteDecision::SpawnWorker(spec) if spec.requires_child_worktree
    );
    if requires_child_worktree {
        match worktree_coordinator::WorktreeCoordinator::prepare_isolated_worker(
            &state.db.lock().unwrap(),
            &state.worktrees.join("workers"),
            &reservation.workspace_id,
            Path::new(&reservation.path),
            &reservation.branch,
            &reservation.session_id,
            &directive.owned_paths,
        ) {
            Ok((path, branch)) => {
                reservation.path = path.to_string_lossy().into_owned();
                reservation.branch = branch;
            }
            Err(error) => {
                let db = state.db.lock().unwrap();
                let transaction = match db.unchecked_transaction() {
                    Ok(transaction) => transaction,
                    Err(_) => return WorkerLaunchOutcome::Failed,
                };
                let _ = transaction.execute("DELETE FROM worker_runtime WHERE session_id=?1", params![reservation.session_id]);
                let _ = transaction.execute("DELETE FROM worker_leases WHERE session_id=?1", params![reservation.session_id]);
                let _ = transaction.execute("DELETE FROM sessions WHERE id=?1", params![reservation.session_id]);
                let _ = transaction.commit();
                let queued = queue_on_block
                    && worker_pool::WorkerPool::enqueue(&db, parent_session_id, &reservation.workspace_id, turn_id, directive, &reservation.actual_model).is_ok();
                let _ = store::event(&db, "worktree", "worker.worktree_queued", parent_session_id, &error.to_string());
                let _ = app.emit("state-changed", ());
                return if queued {
                    WorkerLaunchOutcome::Queued
                } else {
                    WorkerLaunchOutcome::Failed
                };
            }
        }
    }
    let model = reservation.actual_model.clone();
    let effort = directive.effort.as_str().to_owned();
    let label = directive.label();
    let instructions =
        delegation::worker_briefing(directive, reservation.depth, &reservation.branch);

    if reservation.reuse_existing
        && state.adapters.lock().unwrap().contains_key(&reservation.session_id)
    {
        let current = store::worker_runtime(&state.db.lock().unwrap(), &reservation.session_id)
            .ok()
            .flatten()
            .map(|runtime| runtime.lifecycle_state);
        let transition_result = match current.as_deref() {
            Some("warm") => session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("compatible_hot_task"),
            ),
            Some("stopped") => session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                worker_lifecycle::WorkerLifecycleState::Resuming,
                Some("compatible_hot_task"),
            )
            .and_then(|_| session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("hot_process_reused"),
            )),
            _ => Err(BridgeError::Invalid("compatible hot worker is not reusable".into())),
        };
        if transition_result.is_ok()
            && worker_pool::WorkerPool::activate_reused_worker(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                &reservation.workspace_id,
                directive,
            )
            .is_ok()
        {
            let provider_session_id = state
                .adapters
                .lock()
                .unwrap()
                .get(&reservation.session_id)
                .map(|runtime| runtime.provider_session_id().to_owned());
            let _ = restoration::set_head_state(
                &state.db.lock().unwrap(),
                &reservation.session_id,
                RestorationMode::Hot,
                ResumeEligibility::Native,
                provider_session_id.as_deref(),
            );
            if let Some(runtime) = state.adapters.lock().unwrap().get(&reservation.session_id) {
                if runtime.send_turn(&directive.objective).is_ok() {
                    let _ = app.emit("state-changed", ());
                    return WorkerLaunchOutcome::Launched(reservation.session_id);
                }
            }
        }
        let _ = store::event(&state.db.lock().unwrap(), "worker-pool", "worker.hot_resume_failed", &reservation.session_id, "Could not reactivate compatible hot worker");
        return WorkerLaunchOutcome::Failed;
    }

    if directive.write_mode == delegation::WriteMode::ReadOnly {
        match worker_guard::ReadOnlyBaseline::capture(&reservation.path) {
            Ok(baseline) => {
                state
                    .delegations
                    .lock()
                    .unwrap()
                    .read_only_baselines
                    .insert(reservation.session_id.clone(), baseline);
            }
            Err(error) => {
                fail_reserved_worker(
                    app,
                    &reservation.session_id,
                    &label,
                    &format!("Could not capture tracked-file baseline: {error}"),
                );
                return WorkerLaunchOutcome::Failed;
            }
        }
    }

    let activation = if reservation.reuse_existing {
        if session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            &reservation.session_id,
            worker_lifecycle::WorkerLifecycleState::Resuming,
            Some("compatible_cold_task"),
        )
        .is_err()
        {
            return WorkerLaunchOutcome::Failed;
        }
        let provider_id: Option<String> = state.db.lock().unwrap().query_row(
            "SELECT provider_session_id FROM sessions WHERE id=?1",
            params![reservation.session_id],
            |row| row.get(0),
        ).ok().flatten();
        let checkpoint = restoration::checkpoint_context(
            &state.db.lock().unwrap(),
            &reservation.session_id,
        ).ok().flatten();
        let resumed = provider_id.as_deref().filter(|_| state.adapter_registry.supports_native_resume(&harness)).map(|provider_session_id| {
            state.adapter_registry.resume(&harness, adapters::ResumeRequest {
                provider_session_id,
                cwd: &reservation.path,
                model: Some(model.as_str()),
                effort: Some(&effort),
                instructions: Some(instructions.as_str()),
                write_mode: Some(directive.write_mode),
            })
        }).transpose();
        match resumed {
            Ok(Some(started)) => Ok((started, WorkerActivation::Native)),
            Err(error) => {
                let _ = restoration::record_resume_failed(&state.db.lock().unwrap(), &reservation.session_id, &error.to_string());
                let restored_instructions = format!("{instructions}\n\n{}", checkpoint.unwrap_or_else(|| "Bridge checkpoint-restoration context: prior typed worker result is stored in the session forest.".into()));
                state.adapter_registry.start(&harness, adapters::StartRequest {
                    cwd: &reservation.path,
                    model: Some(model.as_str()),
                    effort: Some(&effort),
                    instructions: Some(restored_instructions.as_str()),
                    write_mode: Some(directive.write_mode),
                }).map(|started| (started, WorkerActivation::CheckpointRestored))
            }
            Ok(None) => {
                let restored_instructions = format!("{instructions}\n\n{}", checkpoint.unwrap_or_else(|| "Bridge checkpoint-restoration context: prior typed worker result is stored in the session forest.".into()));
                state.adapter_registry.start(&harness, adapters::StartRequest {
                    cwd: &reservation.path,
                    model: Some(model.as_str()),
                    effort: Some(&effort),
                    instructions: Some(restored_instructions.as_str()),
                    write_mode: Some(directive.write_mode),
                }).map(|started| (started, WorkerActivation::CheckpointRestored))
            }
        }
    } else {
        state.adapter_registry.start(
            &harness,
            adapters::StartRequest {
                cwd: &reservation.path,
                model: Some(model.as_str()),
                effort: Some(&effort),
                instructions: Some(instructions.as_str()),
                write_mode: Some(directive.write_mode),
            },
        ).map(|started| (started, WorkerActivation::Fresh))
    };
    let (started, activation) = match activation {
        Ok(started) => started,
        Err(error) => {
            state
                .delegations
                .lock()
                .unwrap()
                .read_only_baselines
                .remove(&reservation.session_id);
            fail_reserved_worker(
                app,
                &reservation.session_id,
                &label,
                &format!("Could not start provider process: {error}"),
            );
            return WorkerLaunchOutcome::Failed;
        }
    };
    let session_id = reservation.session_id;
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let reader = started.reader;
    let startup_messages = started.startup_messages;
    let mut runtime = started.runtime;
    let started_at = Utc::now().to_rfc3339();

    let transition_result = match activation {
        WorkerActivation::Fresh | WorkerActivation::Native => {
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some(if activation == WorkerActivation::Native { "native_resumed" } else { "provider_started" }),
            ).map(|_| ())
        }
        WorkerActivation::CheckpointRestored => {
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
                worker_lifecycle::WorkerLifecycleState::Restored,
                Some("checkpoint_fallback"),
            ).and_then(|_| session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("checkpoint_restored"),
            )).map(|_| ())
        }
    };
    if let Err(error) = transition_result {
        runtime.stop(adapters::ShutdownReason::Failed);
        let _ = store::event(
            &state.db.lock().unwrap(),
            "supervisor",
            "worker.transition_failed",
            &session_id,
            &error.to_string(),
        );
        return WorkerLaunchOutcome::Failed;
    }
    let (restoration_mode, resume_eligibility) = match activation {
        WorkerActivation::Fresh => (RestorationMode::Fresh, ResumeEligibility::Fresh),
        WorkerActivation::Native => (RestorationMode::Native, ResumeEligibility::Native),
        WorkerActivation::CheckpointRestored => (
            RestorationMode::CheckpointRestored,
            ResumeEligibility::CheckpointRestored,
        ),
    };
    if restoration::set_head_state(
        &state.db.lock().unwrap(),
        &session_id,
        restoration_mode,
        resume_eligibility,
        Some(&thread_id),
    )
    .is_err()
    {
        runtime.stop(adapters::ShutdownReason::Failed);
        return WorkerLaunchOutcome::Failed;
    }

    {
        let db = state.db.lock().unwrap();
        let _ = db.execute(
            "UPDATE sessions SET status='working',started_at=?2,provider_session_id=?3,label=?4,model=?5,effort=?6 WHERE id=?1",
            params![
                session_id,
                started_at,
                thread_id,
                label,
                model,
                effort,
            ],
        );
        let _ = db.execute(
            "UPDATE workspaces SET status='working' WHERE id=?1",
            params![reservation.workspace_id],
        );
        for message in &startup_messages {
            let _ = persist_agent_value(&db, &state.adapter_registry, &harness, &session_id, message);
        }
        let spawn_event = agent::NormalizedEvent {
            kind: if reservation.reuse_existing { "delegation.resumed".into() } else { "delegation.spawned".into() },
            item_id: Some(format!("spawn-{session_id}")),
            role: Some("system".into()),
            status: Some("working".into()),
            title: Some(if reservation.reuse_existing { format!("Resumed {label}") } else { format!("Delegated to {label}") }),
            text: Some(directive.objective.clone()),
            data: serde_json::json!({
                "childSessionId": session_id,
                "request": directive,
                "harness": harness,
                "requestedTier": directive.capability_tier,
                "model": model,
                "modelLabel": delegation::model_display(&model),
                "effort": effort,
                "depth": reservation.depth,
                "turnId": turn_id,
                "policy": reservation.outcome,
                "restorationMode": restoration_mode,
            }),
        };
        if let Ok(stored) =
            store::session_event(&db, parent_session_id, &spawn_event, &serde_json::json!({"delegation": true}))
        {
            let _ = app.emit("agent-event", stored);
        }
        let _ = store::event(
            &db,
            "delegation",
            if reservation.reuse_existing { "worker.resumed" } else { "worker.spawned" },
            parent_session_id,
            &format!(
                "{} {label} (effort {})",
                if reservation.reuse_existing { "Resumed" } else { "Spawned" },
                effort
            ),
        );
    }

    if reservation.reuse_existing
        && worker_pool::WorkerPool::activate_reused_worker(
            &state.db.lock().unwrap(),
            &session_id,
            &reservation.workspace_id,
            directive,
        )
        .is_err()
    {
        return WorkerLaunchOutcome::Failed;
    }
    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), runtime);
    spawn_reader_thread(
        app.clone(),
        session_id.clone(),
        started_at,
        current_turn,
        reader,
    );
    if let Some(runtime) = state.adapters.lock().unwrap().get(&session_id) {
        let _ = runtime.send_turn(&directive.objective);
    }
    let _ = app.emit("state-changed", ());
    WorkerLaunchOutcome::Launched(session_id)
}

fn launch_worker(
    app: &AppHandle,
    parent_session_id: &str,
    turn_id: &str,
    directive: &delegation::DelegationRequest,
    queue_on_block: bool,
) -> Option<String> {
    match launch_worker_outcome(
        app,
        parent_session_id,
        turn_id,
        directive,
        queue_on_block,
    ) {
        WorkerLaunchOutcome::Launched(session_id) => Some(session_id),
        WorkerLaunchOutcome::Queued | WorkerLaunchOutcome::Failed => None,
    }
}

fn fail_reserved_worker(app: &AppHandle, session_id: &str, label: &str, reason: &str) {
    let state = app.state::<AppState>();
    if session_supervisor::SessionSupervisor::transition(
        &state.db.lock().unwrap(),
        session_id,
        worker_lifecycle::WorkerLifecycleState::Working,
        Some("startup_failed_before_process"),
    )
    .is_err()
    {
        return;
    }
    let result = delegation::WorkerResult {
        schema_version: delegation::SCHEMA_VERSION,
        status: delegation::WorkerResultStatus::Failed,
        summary: format!("{label} could not start: {reason}"),
        files_changed: vec![],
        tests: vec![],
        decisions: vec![],
        risks: vec![reason.to_owned()],
        remaining_work: vec!["Retry or delegate the task differently".into()],
        suggested_next_action: delegation::SuggestedNextAction::Finish,
        suggested_role: None,
        suggested_task: None,
    };
    if settle_worker_after_result(app, session_id, &result).unwrap_or(false) {
        report_to_parent(app, session_id, &result);
    }
}

/// Frame a finished worker's final message and send it up to its parent.
fn forward_turn_result(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    let meta: Option<(Option<String>, String, String, Option<String>, Option<String>)> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT parent_session_id,label,harness,model,effort FROM sessions WHERE id=?1",
            params![child_session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .ok()
    };
    let Some((parent, label, harness, model, effort)) = meta else {
        return;
    };
    if parent.is_none() {
        return;
    }
    let raw_output: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT json_extract(payload,'$.text') FROM session_entries
             WHERE session_id=?1 AND kind='message.completed'
               AND json_extract(payload,'$.role')='assistant'
               AND COALESCE(json_extract(payload,'$.text'),'')<>''
             ORDER BY sequence DESC LIMIT 1",
            params![child_session_id],
            |r| r.get(0),
        )
        .ok()
    };
    let raw_output =
        raw_output.unwrap_or_else(|| "(worker finished without a text summary)".to_owned());
    let result = match {
        let db = state.db.lock().unwrap();
        let mut delegations = state.delegations.lock().unwrap();
        process_worker_result_output(
            &db,
            &mut delegations.result_repairs,
            child_session_id,
            &raw_output,
            |prompt| {
                state
                    .adapters
                    .lock()
                    .unwrap()
                    .get(child_session_id)
                    .is_some_and(|runtime| runtime.send_turn(prompt).is_ok())
            },
        )
    } {
        Ok(result) => result,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "delegation",
                "worker.result.processing_failed",
                child_session_id,
                &error.to_string(),
            );
            return;
        }
    };
    let Some(result) = result else {
        let _ = app.emit("state-changed", ());
        return;
    };
    verify_read_only_worker(app, child_session_id);
    let _ = (label, harness, model, effort);
    match settle_worker_after_result(app, child_session_id, &result) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            let _ = store::event(&state.db.lock().unwrap(), "supervisor", "worker.settle_failed", child_session_id, &error.to_string());
            return;
        }
    }
    report_to_parent(app, child_session_id, &result);
    let terminal = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT lifecycle_state IN ('completed','cancelled') FROM worker_runtime WHERE session_id=?1",
            params![child_session_id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    if terminal {
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(child_session_id) {
            runtime.stop(adapters::ShutdownReason::Completed);
        }
    }
}

fn process_worker_result_output(
    db: &Connection,
    tracker: &mut delegation::ResultRepairTracker,
    child_session_id: &str,
    raw_output: &str,
    send_same_session_repair: impl FnOnce(&str) -> bool,
) -> Result<Option<delegation::WorkerResult>, BridgeError> {
    match tracker.process(
        child_session_id,
        raw_output,
        send_same_session_repair,
    ) {
        delegation::WorkerOutputAction::Structured(result) => Ok(Some(result)),
        delegation::WorkerOutputAction::AwaitingRepair { reason } => {
            store::event(
                db,
                "delegation",
                "worker.result.repair_requested",
                child_session_id,
                &reason,
            )?;
            Ok(None)
        }
        delegation::WorkerOutputAction::Unstructured { raw: _, reason } => {
            store::event(
                db,
                "delegation",
                "worker.result.unstructured",
                child_session_id,
                &reason,
            )?;
            Ok(Some(delegation::WorkerResult {
                schema_version: delegation::SCHEMA_VERSION,
                status: delegation::WorkerResultStatus::Failed,
                summary: format!("Unstructured worker result after repair failure: {reason}"),
                files_changed: vec![],
                tests: vec![],
                decisions: vec![],
                risks: vec!["The raw worker response was excluded from parent context".into()],
                remaining_work: vec!["Review the worker transcript manually".into()],
                suggested_next_action: delegation::SuggestedNextAction::Finish,
                suggested_role: None,
                suggested_task: None,
            }))
        }
    }
}

fn settle_worker_after_result(
    app: &AppHandle,
    child_session_id: &str,
    result: &delegation::WorkerResult,
) -> Result<bool, BridgeError> {
    let state = app.state::<AppState>();
    let current = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT lifecycle_state FROM worker_runtime WHERE session_id=?1",
            params![child_session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    if current.as_deref() == Some("failed")
        && matches!(
            result.status,
            delegation::WorkerResultStatus::Failed | delegation::WorkerResultStatus::Blocked
        )
    {
        session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            child_session_id,
            worker_lifecycle::WorkerLifecycleState::Completed,
            Some("terminal_failure_reported"),
        )?;
        return Ok(true);
    }
    if current.as_deref() != Some("working") {
        return Ok(true);
    }
    if result.is_retryable() {
        let retry_count = store::worker_runtime(&state.db.lock().unwrap(), child_session_id)?
            .map(|runtime| runtime.retry_count)
            .unwrap_or(1);
        let can_retry_hot = worker_pool::should_retry(
            result,
            retry_count,
            state.adapters.lock().unwrap().contains_key(child_session_id),
        );
        if can_retry_hot {
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Failed,
                Some("typed_failure"),
            )?;
            state.db.lock().unwrap().execute(
                "UPDATE worker_runtime SET retry_count=1,updated_at=?2 WHERE session_id=?1",
                params![child_session_id, Utc::now().to_rfc3339()],
            )?;
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Resuming,
                Some("automatic_retry"),
            )?;
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Working,
                Some("same_process_retry"),
            )?;
            let sent = state.adapters.lock().unwrap().get(child_session_id).is_some_and(|runtime| {
                runtime.send_turn("Retry the same assigned task once. Address the failure, rerun verification, and return a typed worker result.").is_ok()
            });
            if sent {
                return Ok(false);
            }
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Failed,
                Some("retry_delivery_failed"),
            )?;
            session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                child_session_id,
                worker_lifecycle::WorkerLifecycleState::Completed,
                Some("terminal_failure_reported"),
            )?;
            return Ok(true);
        }
    }
    let (next, warm_until) = match result.status {
        delegation::WorkerResultStatus::Completed
        | delegation::WorkerResultStatus::NeedsDelegation => {
            let attributes: Option<(String, String, String)> = state
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT role,capability_tier,write_mode FROM worker_leases WHERE session_id=?1",
                    params![child_session_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok();
            match attributes.map(|(role, tier, mode)| {
                worker_pool::retention_action_for_attributes(&role, &tier, &mode, Utc::now())
            }) {
                Some(worker_pool::RetentionAction::KeepWarmUntil(until)) => {
                    (worker_lifecycle::WorkerLifecycleState::Warm, Some(until.to_rfc3339()))
                }
                _ => (worker_lifecycle::WorkerLifecycleState::Completed, None),
            }
        }
        delegation::WorkerResultStatus::Cancelled => (
            worker_lifecycle::WorkerLifecycleState::Cancelled,
            None,
        ),
        delegation::WorkerResultStatus::Failed | delegation::WorkerResultStatus::Blocked => {
            (worker_lifecycle::WorkerLifecycleState::Failed, None)
        }
    };
    session_supervisor::SessionSupervisor::transition(
        &state.db.lock().unwrap(),
        child_session_id,
        next,
        Some("typed_result"),
    )?;
    if matches!(
        result.status,
        delegation::WorkerResultStatus::Failed | delegation::WorkerResultStatus::Blocked
    ) {
        session_supervisor::SessionSupervisor::transition(
            &state.db.lock().unwrap(),
            child_session_id,
            worker_lifecycle::WorkerLifecycleState::Completed,
            Some("terminal_failure_reported"),
        )?;
    }
    if let Some(warm_until) = warm_until {
        state.db.lock().unwrap().execute(
            "UPDATE worker_runtime SET warm_until=?2 WHERE session_id=?1",
            params![child_session_id, warm_until],
        )?;
    }
    Ok(true)
}

/// If a worker process exits before ever reporting, tell its parent so the
/// parent is not left waiting on a child that will never answer.
fn notify_parent_on_worker_exit(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    verify_read_only_worker(app, child_session_id);
    let already = store::worker_runtime(&state.db.lock().unwrap(), child_session_id)
        .ok()
        .flatten()
        .is_some_and(|runtime| runtime.result_status == "reported");
    if already {
        return;
    }
    let meta: Option<(Option<String>, String)> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT parent_session_id,label FROM sessions WHERE id=?1",
            params![child_session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok()
    };
    let Some((Some(_parent), label)) = meta else {
        return;
    };
    let result = delegation::WorkerResult {
        schema_version: delegation::SCHEMA_VERSION,
        status: delegation::WorkerResultStatus::Failed,
        summary: format!("{label} ended without reporting a result"),
        files_changed: vec![],
        tests: vec![],
        decisions: vec![],
        risks: vec!["Worker process exited before a typed result was produced".into()],
        remaining_work: vec!["Retry or delegate the task differently".into()],
        suggested_next_action: delegation::SuggestedNextAction::Finish,
        suggested_role: None,
        suggested_task: None,
    };
    match settle_worker_after_result(app, child_session_id, &result) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            let _ = store::event(&state.db.lock().unwrap(), "supervisor", "worker.settle_failed", child_session_id, &error.to_string());
            return;
        }
    }
    report_to_parent(app, child_session_id, &result);
}

fn verify_read_only_worker(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    let baseline = state
        .delegations
        .lock()
        .unwrap()
        .read_only_baselines
        .remove(child_session_id);
    let Some(baseline) = baseline else {
        return;
    };
    let db = state.db.lock().unwrap();
    if let Err(error) = worker_guard::verify_and_record(&db, child_session_id, &baseline) {
        let _ = store::event(
            &db,
            "sandbox",
            "worker.read_only_verification_failed",
            child_session_id,
            &error.to_string(),
        );
    }
}

/// Deliver a framed message from a child to its parent session: send it into the
/// parent's live turn stream and drop a marker card into the parent's transcript.
fn report_to_parent(
    app: &AppHandle,
    child_session_id: &str,
    result: &delegation::WorkerResult,
) {
    let state = app.state::<AppState>();
    let parent_id = {
        let db = state.db.lock().unwrap();
        session_supervisor::SessionSupervisor::record_result(&db, child_session_id, result)
            .ok()
            .flatten()
    };
    let Some(parent_id) = parent_id else {
        return;
    };
    let typed_result = serde_json::to_string(result).unwrap_or_else(|_| result.summary.clone());
    let delivered = match state.adapters.lock().unwrap().get(&parent_id) {
        Some(runtime) => runtime.send_turn(&typed_result).is_ok(),
        None => false,
    };
    {
        let db = state.db.lock().unwrap();
        let result_event = agent::NormalizedEvent {
            kind: "delegation.result".into(),
            item_id: Some(format!("result-{}", Uuid::new_v4())),
            role: Some("system".into()),
            status: Some("completed".into()),
            title: Some("Worker result".into()),
            text: Some(result.summary.clone()),
            data: serde_json::json!({"childSessionId": child_session_id, "delivered": delivered, "result": result}),
        };
        if let Ok(stored) =
            store::session_event(&db, &parent_id, &result_event, &serde_json::json!({"delegation": true}))
        {
            let _ = app.emit("agent-event", stored);
        }
        if delivered {
            let _ = db.execute(
                "UPDATE sessions SET status='working' WHERE id=?1 AND ended_at IS NULL",
                params![parent_id],
            );
        }
    }
    let workspace_id = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![child_session_id],
            |row| row.get::<_, String>(0),
        )
        .ok();
    let _ = app.emit("state-changed", ());
    if let Some(workspace_id) = workspace_id {
        dispatch_next_queued_worker(app, &workspace_id);
    }
}

fn dispatch_next_queued_worker(app: &AppHandle, workspace_id: &str) {
    let state = app.state::<AppState>();
    let queued = worker_pool::WorkerPool::claim_next_queued(
        &state.db.lock().unwrap(),
        workspace_id,
    )
    .ok()
    .flatten();
    let Some(queued) = queued else {
        return;
    };
    let directive: delegation::DelegationRequest = match serde_json::from_value(queued.request) {
        Ok(directive) => directive,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::update_worker_queue(&db, &queued.id, "rejected", None);
            let _ = store::event(&db, "worker-pool", "worker.queue.invalid", &queued.id, &error.to_string());
            return;
        }
    };
    let launched = launch_worker(
        app,
        &queued.parent_session_id,
        &queued.turn_id,
        &directive,
        false,
    );
    let db = state.db.lock().unwrap();
    let _ = if let Some(session_id) = launched.as_deref() {
        store::update_worker_queue(&db, &queued.id, "dispatched", Some(session_id))
    } else {
        store::update_worker_queue(&db, &queued.id, "rejected", None)
    };
}

fn maintain_worker_pool(app: &AppHandle) {
    let state = app.state::<AppState>();
    let expired = worker_pool::WorkerPool::warm_workers_due(
        &state.db.lock().unwrap(),
        Utc::now(),
    )
    .unwrap_or_default();
    for session_id in expired {
        let prompt = {
            let db = state.db.lock().unwrap();
            let tokens = compaction_controller::active_token_estimate(&db, &session_id)
                .unwrap_or_default();
            let prompt = compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeSuspend,
                tokens,
            )
            .ok()
            .flatten();
            if prompt.is_some() {
                let _ = session_supervisor::SessionSupervisor::transition(
                    &db,
                    &session_id,
                    worker_lifecycle::WorkerLifecycleState::Checkpointing,
                    Some("warm_idle_timeout"),
                );
                let _ = db.execute(
                    "UPDATE worker_runtime SET warm_until=NULL WHERE session_id=?1",
                    params![session_id],
                );
            }
            prompt
        };
        if let Some(prompt) = prompt {
            if let Err(error) = send_internal_checkpoint_turn(app, &session_id, &prompt) {
                let _ = compaction_controller::CompactionController::record_failure(
                    &state.db.lock().unwrap(),
                    &session_id,
                    &format!("checkpoint turn could not start: {error}"),
                    0,
                );
                finish_worker_checkpoint(app, &session_id, adapters::ShutdownReason::Failed);
            }
        }
    }
    let timed_out = {
        let db = state.db.lock().unwrap();
        let mut statement = match db.prepare(
            "SELECT session_id FROM worker_runtime WHERE lifecycle_state='checkpointing' ORDER BY session_id",
        ) {
            Ok(statement) => statement,
            Err(_) => return,
        };
        let result = match statement.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows
                .filter_map(Result::ok)
                .filter(|session_id| {
                    compaction_controller::CompactionController::pending(&db, session_id)
                        .ok()
                        .flatten()
                        .and_then(|pending| chrono::DateTime::parse_from_rfc3339(&pending.requested_at).ok())
                        .is_some_and(|requested| {
                            Utc::now().signed_duration_since(requested.with_timezone(&Utc)).num_seconds()
                                >= compaction_controller::CHECKPOINT_TIMEOUT_SECONDS
                        })
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        result
    };
    for session_id in timed_out {
        let _ = compaction_controller::CompactionController::record_failure(
            &state.db.lock().unwrap(),
            &session_id,
            "checkpoint turn timed out; suspension continued",
            1,
        );
        finish_worker_checkpoint(app, &session_id, adapters::ShutdownReason::Failed);
    }
    let shutdown_timeouts = {
        let db = state.db.lock().unwrap();
        let mut statement = match db.prepare(
            "SELECT id FROM sessions WHERE status='checkpointing' AND parent_session_id IS NULL ORDER BY id",
        ) {
            Ok(statement) => statement,
            Err(_) => return,
        };
        let result = match statement.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows
                .filter_map(Result::ok)
                .filter(|session_id| {
                    compaction_controller::CompactionController::pending(&db, session_id)
                        .ok()
                        .flatten()
                        .filter(|pending| {
                            pending.reason
                                == compaction_controller::CompactionReason::BeforeShutdown
                        })
                        .and_then(|pending| chrono::DateTime::parse_from_rfc3339(&pending.requested_at).ok())
                        .is_some_and(|requested| {
                            Utc::now().signed_duration_since(requested.with_timezone(&Utc)).num_seconds()
                                >= compaction_controller::CHECKPOINT_TIMEOUT_SECONDS
                        })
                })
                .collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        result
    };
    for session_id in shutdown_timeouts {
        let _ = compaction_controller::CompactionController::record_failure(
            &state.db.lock().unwrap(),
            &session_id,
            "shutdown checkpoint timed out; termination continued",
            1,
        );
        finish_orchestrator_shutdown(app, &session_id, adapters::ShutdownReason::UserStopped);
    }
    let workspaces = {
        let db = state.db.lock().unwrap();
        let mut statement = match db.prepare(
            "SELECT workspace_id FROM worker_queue WHERE queue_status='queued' GROUP BY workspace_id ORDER BY MIN(sequence),workspace_id",
        ) {
            Ok(statement) => statement,
            Err(_) => return,
        };
        let workspaces = match statement.query_map([], |row| row.get::<_, String>(0)) {
            Ok(rows) => rows.filter_map(Result::ok).collect::<Vec<_>>(),
            Err(_) => return,
        };
        workspaces
    };
    for workspace_id in workspaces {
        dispatch_next_queued_worker(app, &workspace_id);
    }
}

fn start_worker_maintenance(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        maintain_worker_pool(&app);
    });
}

#[tauri::command]
fn open_terminal(
    workspace_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), BridgeError> {
    let runtime_id = format!("terminal:{workspace_id}");
    if state.runtimes.lock().unwrap().contains_key(&runtime_id) {
        return Ok(());
    }
    let db = state.db.lock().unwrap();
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
    command.env("BRIDGE_WORKSPACE_ID", &workspace_id);
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
    state.runtimes.lock().unwrap().insert(
        runtime_id.clone(),
        RuntimeSession {
            writer,
            master: pair.master,
            child,
        },
    );
    let app_reader = app.clone();
    let workspace_reader = workspace_id.clone();
    let runtime_reader = runtime_id.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = app_reader.emit(
                        "session-output",
                        TerminalChunk {
                            session_id: workspace_reader.clone(),
                            data,
                        },
                    );
                }
            }
        }
        let state = app_reader.state::<AppState>();
        state.runtimes.lock().unwrap().remove(&runtime_reader);
    });
    Ok(())
}

#[tauri::command]
fn write_terminal(
    workspace_id: String,
    data: String,
    state: State<AppState>,
) -> Result<(), BridgeError> {
    let mut sessions = state.runtimes.lock().unwrap();
    let runtime = sessions
        .get_mut(&format!("terminal:{workspace_id}"))
        .ok_or_else(|| BridgeError::Invalid("Workspace terminal is not open".into()))?;
    runtime.writer.write_all(data.as_bytes())?;
    runtime.writer.flush()?;
    Ok(())
}

#[tauri::command]
fn send_turn(session_id: String, text: String, app: AppHandle, state: State<AppState>) -> Result<(), BridgeError> {
    if text.trim().is_empty() {
        return Err(BridgeError::Invalid("Message cannot be empty".into()));
    }
    if store::worker_runtime(&state.db.lock().unwrap(), &session_id)?.is_some() {
        return Err(BridgeError::Invalid(
            "Worker turns are scheduled through the policy-controlled worker pool".into(),
        ));
    }

    let available: std::collections::HashSet<String> = state
        .adapter_registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| descriptor.available)
        .map(|descriptor| descriptor.id)
        .collect();
    let session_harness: String = state.db.lock().unwrap().query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![session_id],
        |row| row.get(0),
    )?;

    let outbound = match slash::dispatch(&text, &session_harness, &available) {
        slash::SlashDispatch::Usage => {
            refresh_account_usage(app.clone(), state.clone())?;
            emit_local_assistant(
                &app,
                &state,
                &session_id,
                &session_harness,
                "Refreshed account usage. Check the meter in the title bar.",
            )?;
            return Ok(());
        }
        slash::SlashDispatch::Compact { .. } => {
            compact_session(session_id.clone(), app.clone(), state.clone())?;
            return Ok(());
        }
        slash::SlashDispatch::Clear => {
            if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
                runtime.stop(adapters::ShutdownReason::UserStopped);
            }
            let db = state.db.lock().unwrap();
            db.execute(
                "UPDATE sessions SET provider_session_id=NULL,status='idle',active_turn_id=NULL,ended_at=NULL WHERE id=?1",
                params![session_id],
            )?;
            emit_local_assistant(
                &app,
                &state,
                &session_id,
                &session_harness,
                "Cleared this chat’s provider session. Send a message to start fresh.",
            )?;
            let _ = app.emit("state-changed", ());
            return Ok(());
        }
        slash::SlashDispatch::Unsupported { name, harness } => {
            emit_local_assistant(
                &app,
                &state,
                &session_id,
                &session_harness,
                &format!("`/{name}` is a {harness} terminal UI command and isn’t available inside Bridge yet."),
            )?;
            return Ok(());
        }
        slash::SlashDispatch::Expand { text } => text,
        slash::SlashDispatch::Forward { text } => text,
    };

    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    if let Err(error) = runtime.send_turn(&outbound) {
        drop(adapters);
        record_recoverable_adapter_failure(&state, &session_id, &error)?;
        return Err(error);
    }
    drop(adapters);
    let db = state.db.lock().unwrap();
    let adapter_id: String = db.query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    // Claude stream-json does not reliably echo the submitted user turn; persist it locally.
    // Prefer the original slash text for the transcript when we expanded a skill/prompt.
    let display_text = if outbound != text { text.clone() } else { outbound.clone() };
    if adapter_id == "claude" {
        let user_event = agent::NormalizedEvent {
            kind: "message.completed".into(),
            item_id: Some(format!("user-{}", Uuid::new_v4())),
            role: Some("user".into()),
            status: Some("completed".into()),
            title: None,
            text: Some(display_text),
            data: serde_json::json!({}),
        };
        let event = store::session_event(
            &db,
            &session_id,
            &user_event,
            &serde_json::json!({"adapter": adapter_id}),
        )?;
        let _ = app.emit("agent-event", event);
    }
    let _ = db.execute(
        "UPDATE sessions SET status='working' WHERE id=?1",
        params![session_id],
    );
    let _ = app.emit("state-changed", ());
    Ok(())
}

fn record_recoverable_adapter_failure(state: &State<AppState>, session_id: &str, error: &BridgeError) -> Result<(), BridgeError> {
    let db = state.db.lock().unwrap();
    db.execute("UPDATE sessions SET status='failed',active_turn_id=NULL,ended_at=?2 WHERE id=?1", params![session_id, Utc::now().to_rfc3339()])?;
    store::event(&db, "adapter", "adapter.request_failed", session_id, &error.to_string())?;
    Ok(())
}

fn emit_local_assistant(
    app: &AppHandle,
    state: &State<AppState>,
    session_id: &str,
    adapter_id: &str,
    text: &str,
) -> Result<(), BridgeError> {
    let db = state.db.lock().unwrap();
    let event = store::session_event(
        &db,
        session_id,
        &agent::NormalizedEvent {
            kind: "message.completed".into(),
            item_id: Some(format!("bridge-{}", Uuid::new_v4())),
            role: Some("assistant".into()),
            status: Some("completed".into()),
            title: None,
            text: Some(text.into()),
            data: serde_json::json!({ "bridgeLocal": true }),
        },
        &serde_json::json!({ "adapter": adapter_id }),
    )?;
    let _ = app.emit("agent-event", event);
    Ok(())
}

#[tauri::command]
fn compact_session(
    session_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), BridgeError> {
    let prompt = {
        let db = state.db.lock().unwrap();
        let status: String = db.query_row(
            "SELECT status FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if matches!(status.as_str(), "working" | "waiting" | "checkpointing") {
            return Err(BridgeError::Invalid(
                "Compaction waits until the active tool, approval, or turn finishes".into(),
            ));
        }
        let branch = session_forest::SessionForest::new(&db)
            .active_branch(&session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let meaningful = branch.iter().any(|entry| {
            matches!(
                entry.kind.as_str(),
                "user.message" | "assistant.message" | "worker.result" | "tool.completed"
            )
        });
        compaction_controller::decide(&compaction_controller::TriggerState {
            reason: compaction_controller::CompactionReason::Manual,
            context_percent: None,
            projected_tokens_with_reserve: None,
            context_window_tokens: None,
            has_valid_typed_result: false,
            one_shot_worker: false,
            tool_call_active: false,
            approval_active: false,
            has_meaningful_new_work: meaningful,
            wall_clock_only: false,
        })
        .map_err(|reason| BridgeError::Invalid(format!("Compaction suppressed: {reason:?}")))?;
        let tokens = compaction_controller::active_token_estimate(&db, &session_id)?;
        compaction_controller::CompactionController::begin(
            &db,
            &session_id,
            compaction_controller::CompactionReason::Manual,
            tokens,
        )?
        .ok_or_else(|| BridgeError::Invalid("Compaction is already pending".into()))?
    };
    send_internal_checkpoint_turn(&app, &session_id, &prompt)
}

#[tauri::command]
fn interrupt_turn(session_id: String, state: State<AppState>) -> Result<(), BridgeError> {
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    runtime.interrupt()
}

/// Refresh subscription usage for every provider, independent of which session
/// is on screen. Claude is queried out-of-band via its headless `/usage`
/// command; Codex is asked on a live session and answers on its event stream.
/// Both results are broadcast on the `account-usage` channel.
#[tauri::command]
fn refresh_account_usage(app: AppHandle, state: State<AppState>) -> Result<(), BridgeError> {
    // Claude: a global, read-only account query — no running session required.
    if binary::resolve("claude").is_some() {
        let app = app.clone();
        thread::spawn(move || {
            let cwd = std::env::temp_dir();
            let cwd = cwd.to_string_lossy();
            if let Some(data) = claude_adapter::read_usage_snapshot(cwd.as_ref()) {
                if let Some(rate_limits) = data.get("rateLimits") {
                    emit_account_usage(&app, "claude", rate_limits.clone());
                }
            }
        });
    }
    // Codex: rate limits are account-wide, so a single running session answers
    // for the whole account. Its reply routes back through handle_agent_value.
    let codex_sessions: Vec<String> = {
        let db = state.db.lock().unwrap();
        let mut statement =
            db.prepare("SELECT id FROM sessions WHERE harness='codex' AND ended_at IS NULL")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        ids
    };
    let adapters = state.adapters.lock().unwrap();
    for session_id in codex_sessions {
        if let Some(runtime) = adapters.get(&session_id) {
            let _ = runtime.read_usage();
            break;
        }
    }
    Ok(())
}

/// Rate-limit snapshot carried by a Codex account frame, if this is one.
fn codex_rate_limits_from_frame(value: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(rate_limits) = value.pointer("/result/rateLimits") {
        return Some(rate_limits.clone());
    }
    if value.get("method").and_then(|m| m.as_str()) == Some("account/rateLimits/updated") {
        if let Some(rate_limits) = value.pointer("/params/rateLimits") {
            return Some(rate_limits.clone());
        }
    }
    None
}

/// Broadcast a provider's subscription usage to the UI's ambient meter.
fn emit_account_usage(app: &AppHandle, provider: &str, rate_limits: serde_json::Value) {
    let _ = app.emit(
        "account-usage",
        serde_json::json!({ "provider": provider, "rateLimits": rate_limits }),
    );
}

#[tauri::command]
fn resolve_approval(
    session_id: String,
    event_id: i64,
    decision: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), BridgeError> {
    if !matches!(
        decision.as_str(),
        "accept" | "acceptForSession" | "decline" | "cancel"
    ) {
        return Err(BridgeError::Invalid("Unsupported approval decision".into()));
    }
    let db = state.db.lock().unwrap();
    let (data, adapter_id): (String, String) = db.query_row(
        "SELECT e.payload,s.harness FROM session_entries e
         JOIN sessions s ON s.id=e.session_id
         WHERE e.session_id=?1 AND e.sequence=?2 AND e.kind='approval.requested'",
        params![session_id, event_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let data: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| BridgeError::Invalid(format!("Approval metadata is invalid: {e}")))?;
    if data.get("approvalType").and_then(serde_json::Value::as_str) == Some("delegation_path_scope")
    {
        let launch =
            resolve_policy_delegation_approval(&db, &session_id, event_id, &decision, &data)?;
        drop(db);
        if let Some((turn_id, request)) = launch {
            match launch_worker_outcome(&app, &session_id, &turn_id, &request, true) {
                WorkerLaunchOutcome::Launched(_) | WorkerLaunchOutcome::Queued => {}
                WorkerLaunchOutcome::Failed => {
                    let db = state.db.lock().unwrap();
                    record_approved_launch_failure(&db, &session_id, &turn_id, &request)?;
                    let _ = app.emit("state-changed", ());
                    return Err(BridgeError::Invalid(
                        "Write scope was approved, but the worker could not launch; the delegation may be retried for this turn".into(),
                    ));
                }
            }
        }
        let _ = app.emit("state-changed", ());
        return Ok(());
    }
    let request_id = data
        .get("requestId")
        .cloned()
        .ok_or_else(|| BridgeError::Invalid("Approval has no adapter request id".into()))?;
    let is_worker = store::worker_runtime(&db, &session_id)?.is_some();
    if is_worker {
        session_supervisor::SessionSupervisor::transition(
            &db,
            &session_id,
            worker_lifecycle::WorkerLifecycleState::Working,
            Some("approval_resolved"),
        )?;
    }
    drop(db);
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    if let Err(error) = runtime.respond(request_id, &decision) {
        drop(adapters);
        if is_worker {
            let _ = session_supervisor::SessionSupervisor::transition(
                &state.db.lock().unwrap(),
                &session_id,
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
        status: Some(decision.clone()),
        title: Some("Approval resolved".into()),
        text: None,
        data: serde_json::json!({"requestEventId":event_id,"decision":decision}),
    };
    normalized.item_id = data
        .get("itemId")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let db = state.db.lock().unwrap();
    let event = store::session_event(
        &db,
        &session_id,
        &normalized,
        &serde_json::json!({"adapter":adapter_id}),
    )?;
    if !is_worker {
        db.execute(
            "UPDATE sessions SET status='working' WHERE id=?1",
            params![session_id],
        )?;
    }
    let _ = app.emit("agent-event", event);
    let _ = app.emit("state-changed", ());
    Ok(())
}

fn record_approved_launch_failure(
    db: &Connection,
    session_id: &str,
    turn_id: &str,
    request: &delegation::DelegationRequest,
) -> Result<(), BridgeError> {
    session_forest::SessionForest::new(db)
        .append(
            session_id,
            session_forest::EntryKind::DelegationRejected,
            serde_json::json!({
                "requestId": turn_id,
                "turnId": turn_id,
                "status": "failed",
                "reason": "approved_launch_failed",
                "title": "Approved delegation could not launch",
                "text": "The approved same-turn scope remains available if the delegation is retried.",
                "request": request,
            }),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "policy",
        "policy.approved_launch_failed",
        session_id,
        turn_id,
    )?;
    Ok(())
}

fn resolve_policy_delegation_approval(
    db: &Connection,
    session_id: &str,
    event_id: i64,
    decision: &str,
    payload: &serde_json::Value,
) -> Result<Option<(String, delegation::DelegationRequest)>, BridgeError> {
    if decision == "acceptForSession" {
        return Err(BridgeError::Invalid(
            "Delegation path scope can only be approved for this turn".into(),
        ));
    }
    let branch = session_forest::SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let approval_id = payload
        .get("approvalId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BridgeError::Invalid("Policy approval has no approval id".into()))?;
    let request_entry = branch
        .iter()
        .find(|entry| {
            entry.sequence == event_id
                && entry.kind == "approval.requested"
                && entry.payload["approvalId"] == approval_id
        })
        .ok_or_else(|| {
            BridgeError::Invalid("Approval is no longer on the active conversation branch".into())
        })?;
    if branch.iter().any(|entry| {
        entry.kind == "approval.resolved" && entry.payload["approvalId"] == approval_id
    }) {
        return Err(BridgeError::Invalid("Approval was already resolved".into()));
    }
    let request: delegation::DelegationRequest =
        serde_json::from_value(payload.get("request").cloned().ok_or_else(|| {
            BridgeError::Invalid("Policy approval has no delegation request".into())
        })?)
        .map_err(|error| {
            BridgeError::Invalid(format!("Policy approval request is invalid: {error}"))
        })?;
    request.validate().map_err(BridgeError::Invalid)?;
    let turn_id = payload
        .get("turnId")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| BridgeError::Invalid("Policy approval has no parent turn".into()))?
        .to_owned();
    session_forest::SessionForest::new(db)
        .append(
            session_id,
            session_forest::EntryKind::ApprovalResolved,
            serde_json::json!({
                "approvalId": approval_id,
                "approvalType": "delegation_path_scope",
                "requestEventId": event_id,
                "requestEntryId": request_entry.id,
                "turnId": turn_id,
                "decision": decision,
                "approvedOwnedPaths": request.owned_paths,
            }),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    Ok(matches!(decision, "accept" | "acceptForSession").then_some((turn_id, request)))
}

#[tauri::command]
fn resize_terminal(
    workspace_id: String,
    rows: u16,
    cols: u16,
    state: State<AppState>,
) -> Result<(), BridgeError> {
    if let Some(runtime) = state
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
#[tauri::command]
fn stop_session(
    session_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let is_worker = state
        .db
        .lock()
        .unwrap()
        .query_row(
            "SELECT parent_session_id IS NOT NULL FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get::<_, bool>(0),
        )?;
    if is_worker {
        if let Some(runtime) = state.adapters.lock().unwrap().get(&session_id) {
            let _ = runtime.interrupt();
        }
        let result = delegation::WorkerResult {
            schema_version: delegation::SCHEMA_VERSION,
            status: delegation::WorkerResultStatus::Cancelled,
            summary: "Worker cancelled by user".into(),
            files_changed: vec![],
            tests: vec![],
            decisions: vec![],
            risks: vec![],
            remaining_work: vec!["Cancelled work was not completed".into()],
            suggested_next_action: delegation::SuggestedNextAction::Finish,
            suggested_role: None,
            suggested_task: None,
        };
        if !settle_worker_after_result(&app, &session_id, &result)? {
            return Err(BridgeError::Invalid("cancelled worker cannot be retried".into()));
        }
        report_to_parent(&app, &session_id, &result);
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop(adapters::ShutdownReason::UserCancelled);
        }
        let db = state.db.lock().unwrap();
        let workspace_id: String = db.query_row(
            "SELECT workspace_id FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('starting','working','waiting','warm','checkpointing','resuming','restored')) THEN 'working' ELSE 'ready' END WHERE id=?1",params![workspace_id])?;
        let _ = app.emit("state-changed", ());
        return store::state(&db);
    }
    let has_process = state.adapters.lock().unwrap().contains_key(&session_id);
    let shutdown_prompt = {
        let db = state.db.lock().unwrap();
        let status: String = db.query_row(
            "SELECT status FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let branch = session_forest::SessionForest::new(&db)
            .active_branch(&session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let meaningful = branch.iter().any(|entry| {
            matches!(
                entry.kind.as_str(),
                "user.message" | "assistant.message" | "worker.result" | "tool.completed"
            )
        });
        if has_process
            && meaningful
            && !matches!(status.as_str(), "working" | "waiting" | "checkpointing")
        {
            let tokens = compaction_controller::active_token_estimate(&db, &session_id)?;
            compaction_controller::CompactionController::begin(
                &db,
                &session_id,
                compaction_controller::CompactionReason::BeforeShutdown,
                tokens,
            )?
        } else {
            None
        }
    };
    if let Some(prompt) = shutdown_prompt {
        match send_internal_checkpoint_turn(&app, &session_id, &prompt) {
            Ok(()) => {
                let db = state.db.lock().unwrap();
                db.execute(
                    "UPDATE sessions SET status='checkpointing' WHERE id=?1",
                    params![session_id],
                )?;
                let _ = app.emit("state-changed", ());
                return store::state(&db);
            }
            Err(error) => {
                let _ = compaction_controller::CompactionController::record_failure(
                    &state.db.lock().unwrap(),
                    &session_id,
                    &format!("shutdown checkpoint could not start: {error}"),
                    0,
                );
            }
        }
    }
    {
        let db = state.db.lock().unwrap();
        record_shutdown_reason(&db, &session_id, adapters::ShutdownReason::UserStopped)?;
    }
    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
        runtime.stop(adapters::ShutdownReason::UserStopped);
    }
    if let Some(mut runtime) = state.runtimes.lock().unwrap().remove(&session_id) {
        runtime
            .child
            .kill()
            .map_err(|e| BridgeError::Pty(e.to_string()))?;
        let _ = runtime.child.wait();
    }
    let db = state.db.lock().unwrap();
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    db.execute(
        "UPDATE sessions SET status='stopped',ended_at=?2 WHERE id=?1",
        params![session_id, Utc::now().to_rfc3339()],
    )?;
    db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'stopped' END WHERE id=?1",params![workspace_id])?;
    store::event(
        &db,
        "supervisor",
        "session.stopped",
        &session_id,
        "Session stopped by user",
    )?;
    let _ = app.emit("state-changed", ());
    store::state(&db)
}

fn record_shutdown_reason(
    db: &Connection,
    session_id: &str,
    reason: adapters::ShutdownReason,
) -> Result<(), BridgeError> {
    session_forest::SessionForest::new(db)
        .append(
            session_id,
            session_forest::EntryKind::SessionStatus,
            serde_json::json!({"status":"stopped","reason":reason.as_str()}),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "adapter",
        "session.shutdown",
        session_id,
        reason.as_str(),
    )
}
#[tauri::command]
fn refresh_workspace(
    workspace_id: String,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let db = state.db.lock().unwrap();
    let path: String = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get(0),
    )?;
    let (dirty, adds, dels) = git::stats(Path::new(&path))?;
    db.execute(
        "UPDATE workspaces SET dirty_files=?2,additions=?3,deletions=?4 WHERE id=?1",
        params![workspace_id, dirty, adds, dels],
    )?;
    store::state(&db)
}

#[tauri::command]
fn archive_workspace(
    workspace_id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let db = state.db.lock().unwrap();
    let (path, repo): (String, String) = db.query_row(
        "SELECT w.path,p.path FROM workspaces w JOIN projects p ON p.id=w.project_id WHERE w.id=?1",
        params![workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let active: i64 = db.query_row(
        "SELECT COUNT(*) FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting','ready') AND ended_at IS NULL",
        params![workspace_id],
        |r| r.get(0),
    )?;
    if active > 0 {
        return Err(BridgeError::Invalid(
            "Stop every running session before archiving this workspace".into(),
        ));
    }
    let (dirty, _, _) = git::stats(Path::new(&path))?;
    if dirty > 0 {
        return Err(BridgeError::Invalid(format!(
            "Workspace has {dirty} uncommitted file(s). Commit or discard them before archiving"
        )));
    }
    archive_workspace_records(&db, &workspace_id, || {
        git::remove_worktree(Path::new(&repo), Path::new(&path))
    })?;
    store::event(
        &db,
        "supervisor",
        "workspace.archived",
        &workspace_id,
        "Archived clean workspace; branch preserved",
    )?;
    let _ = app.emit("state-changed", ());
    store::state(&db)
}

fn archive_workspace_records(
    db: &Connection,
    workspace_id: &str,
    remove_worktree: impl FnOnce() -> Result<(), BridgeError>,
) -> Result<(), BridgeError> {
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "DELETE FROM task_knowledge WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM worker_leases WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM session_heads WHERE session_id IN (SELECT id FROM sessions WHERE workspace_id=?1)",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM session_entries WHERE session_id IN (SELECT id FROM sessions WHERE workspace_id=?1)",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM usage_ledger WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM sessions WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    transaction.execute(
        "DELETE FROM workspaces WHERE id=?1",
        params![workspace_id],
    )?;
    remove_worktree()?;
    transaction.commit()?;
    Ok(())
}

fn start_health_server(database: PathBuf, adapters: Vec<AdapterDescriptor>) {
    thread::spawn(move || {
        let Ok(server) = tiny_http::Server::http("127.0.0.1:4317") else {
            return;
        };
        for request in server.incoming_requests() {
            let (status, body) = if request.url() == "/health" {
                (
                    200,
                    serde_json::json!({
                        "ok": true,
                        "version": env!("CARGO_PKG_VERSION"),
                        "database": database,
                        "adapters": adapters,
                        "harnesses": {
                            "claude": binary::resolve("claude").is_some(),
                            "codex": binary::resolve("codex").is_some(),
                            "shell": true
                        }
                    })
                    .to_string(),
                )
            } else {
                (
                    404,
                    serde_json::json!({"ok": false, "error": "not found"}).to_string(),
                )
            };
            let mut response = tiny_http::Response::from_string(body).with_status_code(status);
            if let Ok(header) = tiny_http::Header::from_bytes("Content-Type", "application/json") {
                response.add_header(header);
            }
            let _ = request.respond(response);
        }
    });
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let db_path = data.join("bridge.db");
            let connection =
                store::open(&db_path).map_err(|e| Box::<dyn std::error::Error>::from(e))?;
            session_supervisor::SessionSupervisor::recover_orphaned_workers(&connection)
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            let adapter_registry = adapters::AdapterRegistry::built_in()
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            start_health_server(db_path.clone(), adapter_registry.descriptors());
            app.manage(AppState {
                db: Mutex::new(connection),
                runtimes: Mutex::new(HashMap::new()),
                adapters: Mutex::new(HashMap::new()),
                adapter_registry,
                delegations: Mutex::new(DelegationState::default()),
                worktrees: data.join("worktrees"),
                database_path: db_path,
            });
            start_worker_maintenance(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health,
            get_state,
            get_session_forest,
            activate_session_entry,
            add_project,
            create_workspace,
            create_chat,
            create_workspace_session,
            connect_workspace_folder,
            update_chat_model,
            list_slash_commands,
            resolve_slash_command,
            start_session,
            start_chat,
            open_terminal,
            write_terminal,
            resize_terminal,
            send_turn,
            compact_session,
            interrupt_turn,
            refresh_account_usage,
            resolve_approval,
            stop_session,
            refresh_workspace,
            archive_workspace
        ])
        .run(tauri::generate_context!())
        .expect("Bridge failed to start")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_request(paths: &[&str]) -> delegation::DelegationRequest {
        delegation::DelegationRequest {
            schema_version: 1,
            role: delegation::WorkerRole::Implementation,
            objective: "Implement auth".into(),
            acceptance_criteria: vec!["Tests pass".into()],
            known_facts: Vec::new(),
            decisions: Vec::new(),
            relevant_files: Vec::new(),
            owned_paths: paths.iter().map(|path| (*path).into()).collect(),
            write_mode: delegation::WriteMode::Isolated,
            capability_tier: delegation::CapabilityTier::Standard,
            effort: delegation::Effort::Medium,
            verification: vec!["cargo test".into()],
            output_contract: delegation::OutputContract::ImplementationResult,
            harness: Some("codex".into()),
            model: None,
        }
    }

    fn policy_fixture() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        let workspace_path = Path::new(env!("CARGO_MANIFEST_DIR"));
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![workspace_path.to_string_lossy()],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task',?1,'idle','now')", params![workspace_path.to_string_lossy()]).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,depth) VALUES('parent','w','codex','Parent','working','reported',0)", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('parent','fresh','now')", []).unwrap();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Write scope: src/**"}),
            )
            .unwrap();
        db
    }

    fn archive_fixture() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/archive-demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/archive-workspace','stopped','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','Codex','stopped','reported')", []).unwrap();
        db.execute("INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,created_at) VALUES('e1','s',NULL,1,'user.message','{\"text\":\"one\"}','now'),('e2','s','e1',2,'assistant.message','{\"text\":\"two\"}','now')", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,active_entry_id,restoration_mode,latest_checkpoint_entry_id,updated_at) VALUES('s','e2','fresh','e1','now')", []).unwrap();
        db.execute("INSERT INTO task_knowledge(id,workspace_id,session_id,kind,body,source_entry_id,created_at) VALUES('k','w','s','decision','Keep history','e1','now')", []).unwrap();
        db.execute("INSERT INTO worker_leases(session_id,workspace_id,role,capability_tier,write_mode,lease_status,created_at,updated_at) VALUES('s','w','implementation','standard','shared','expired','now','now')", []).unwrap();
        db.execute("INSERT INTO usage_ledger(workspace_id,session_id,turn_id,capability_units,source,created_at) VALUES('w','s','turn',3,'test','now')", []).unwrap();
        db
    }

    fn count(db: &Connection, table: &str) -> i64 {
        db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn session_status_round_trip() {
        for (value, expected) in [
            ("starting", SessionStatus::Starting),
            ("working", SessionStatus::Working),
            ("waiting", SessionStatus::Waiting),
            ("warm", SessionStatus::Warm),
            ("checkpointing", SessionStatus::Checkpointing),
            ("stopped", SessionStatus::Stopped),
            ("resuming", SessionStatus::Resuming),
            ("restored", SessionStatus::Restored),
            ("failed", SessionStatus::Failed),
            ("completed", SessionStatus::Completed),
            ("cancelled", SessionStatus::Cancelled),
        ] {
            assert_eq!(store::status(value), expected);
        }
    }

    #[test]
    fn pressure_compaction_starts_only_at_seventy_five_percent_with_new_work() {
        let db = policy_fixture();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"meaningful work"}),
            )
            .unwrap();
        db.execute("INSERT INTO usage_ledger(workspace_id,session_id,context_percent,capability_units,source,created_at) VALUES('w','parent',74,0,'test','now')", []).unwrap();
        assert!(begin_pressure_compaction(&db, "parent").unwrap().is_none());
        db.execute("INSERT INTO usage_ledger(workspace_id,session_id,context_percent,capability_units,source,created_at) VALUES('w','parent',75,0,'test','later')", []).unwrap();
        assert!(begin_pressure_compaction(&db, "parent").unwrap().is_some());
        assert_eq!(
            compaction_controller::CompactionController::pending(&db, "parent")
                .unwrap()
                .unwrap()
                .reason,
            compaction_controller::CompactionReason::ContextPressure
        );
    }

    #[test]
    fn archive_workspace_records_cleans_every_dependent_table() {
        let db = archive_fixture();
        archive_workspace_records(&db, "w", || Ok(())).unwrap();
        for table in [
            "task_knowledge",
            "worker_leases",
            "session_heads",
            "session_entries",
            "usage_ledger",
            "sessions",
            "workspaces",
        ] {
            assert_eq!(count(&db, table), 0, "{table} retained archive rows");
        }
        assert_eq!(count(&db, "projects"), 1);
        assert_eq!(
            db.query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn sqlite_snapshot_replays_forest_and_rewind_changes_only_active_head() {
        let db = archive_fixture();
        let before_entries = store::session_entries(&db, "s").unwrap();
        let before_workspace_path: String = db
            .query_row("SELECT path FROM workspaces WHERE id='w'", [], |row| row.get(0))
            .unwrap();
        let initial = session_forest_snapshot(&db, "s").unwrap();
        assert_eq!(initial.head.unwrap().active_entry_id.as_deref(), Some("e2"));
        assert_eq!(initial.entries.len(), 2);
        assert_eq!(initial.leaves.iter().map(|entry| entry.id.as_str()).collect::<Vec<_>>(), vec!["e2"]);
        assert_eq!(initial.worker_leases.len(), 1);
        assert_eq!(initial.usage.len(), 1);

        let rewound = activate_session_entry_records(&db, "s", "e1").unwrap();
        assert_eq!(rewound.head.unwrap().active_entry_id.as_deref(), Some("e1"));
        assert_eq!(store::session_entries(&db, "s").unwrap(), before_entries);
        assert_eq!(
            db.query_row("SELECT path FROM workspaces WHERE id='w'", [], |row| row.get::<_, String>(0))
                .unwrap(),
            before_workspace_path
        );
        assert!(rewound.reasons.iter().any(|event| {
            event.kind == "session.head_moved" && event.body.contains("files were not changed")
        }));
    }

    #[test]
    fn archive_workspace_records_rolls_back_when_worktree_removal_fails() {
        let db = archive_fixture();
        let result = archive_workspace_records(&db, "w", || {
            Err(BridgeError::Git("injected removal failure".into()))
        });
        assert!(matches!(result, Err(BridgeError::Git(_))));
        for table in [
            "task_knowledge",
            "worker_leases",
            "session_heads",
            "session_entries",
            "usage_ledger",
            "sessions",
            "workspaces",
        ] {
            assert!(count(&db, table) > 0, "{table} was not rolled back");
        }
    }

    #[test]
    fn repair_and_fallback_store_audit_events() {
        let db = store::open(Path::new(":memory:")).unwrap();
        let mut tracker = delegation::ResultRepairTracker::default();
        let first = process_worker_result_output(
            &db,
            &mut tracker,
            "worker",
            "invalid first output",
            |prompt| {
                assert!(prompt.contains("one repair turn"));
                true
            },
        )
        .unwrap();
        assert_eq!(first, None);
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "worker.result.repair_requested"
        );

        let fallback = process_worker_result_output(
            &db,
            &mut tracker,
            "worker",
            "invalid repair output",
            |_| panic!("a second repair must not be sent"),
        )
        .unwrap()
        .unwrap();
        assert!(fallback.summary.contains("Unstructured worker result"));
        assert!(!fallback.summary.contains("invalid first output"));
        assert!(!fallback.summary.contains("invalid repair output"));
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "worker.result.unstructured"
        );
    }

    #[test]
    fn policy_reservation_precedes_spawn_and_queues_overlapping_writer() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        let first = reserve_worker_launch(&db, "parent", "turn-1", &request, "gpt-5.6-terra", true)
            .unwrap()
            .expect("first writer should reserve");
        assert!(matches!(
            first.outcome.decision,
            policy::RouteDecision::SpawnWorker(_)
        ));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM worker_leases WHERE workspace_id='w' AND lease_status='active'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
        assert_eq!(
            db.query_row(
                "SELECT turn_id FROM usage_ledger WHERE session_id=?1",
                params![first.session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "turn-1"
        );
        assert_eq!(
            db.query_row(
                "SELECT requested_tier || ':' || model FROM sessions WHERE id=?1",
                params![first.session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "standard:gpt-5.6-terra"
        );

        let second = reserve_worker_launch_outcome(
            &db,
            "parent",
            "turn-1",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        assert!(matches!(second, WorkerReservationOutcome::Queued));
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM sessions WHERE workspace_id='w'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            2
        );
        let entries = store::session_entries(&db, "parent").unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[2].kind, "delegation.requested");
        assert_eq!(entries[2].payload["decision"], "queue");
        assert_eq!(entries[2].payload["reason"], "writer_conflict");
    }

    #[test]
    fn policy_approval_blocks_launch_side_effects_until_accepted() {
        let db = policy_fixture();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Please implement the auth change"}),
            )
            .unwrap();
        let request = policy_request(&["src/auth/**"]);
        assert!(reserve_worker_launch(
            &db,
            "parent",
            "turn-approval",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap()
        .is_none());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM worker_leases", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM usage_ledger WHERE source LIKE 'policy.spawn.%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            0
        );
        let approval = store::session_entries(&db, "parent")
            .unwrap()
            .into_iter()
            .last()
            .unwrap();
        assert_eq!(approval.kind, "approval.requested");
        let (turn_id, approved_request) = resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "accept",
            &approval.payload,
        )
        .unwrap()
        .unwrap();
        assert_eq!(turn_id, "turn-approval");
        assert!(reserve_worker_launch(
            &db,
            "parent",
            &turn_id,
            &approved_request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap()
        .is_some());
        assert!(resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "accept",
            &approval.payload,
        )
        .is_err());
    }

    #[test]
    fn declined_policy_approval_never_launches() {
        let db = policy_fixture();
        session_forest::SessionForest::new(&db)
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Explain the auth module only"}),
            )
            .unwrap();
        let request = policy_request(&["src/auth/**"]);
        reserve_worker_launch(
            &db,
            "parent",
            "turn-decline",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        let approval = store::session_entries(&db, "parent")
            .unwrap()
            .into_iter()
            .last()
            .unwrap();
        assert!(resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "decline",
            &approval.payload,
        )
        .unwrap()
        .is_none());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM worker_leases", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
        reserve_worker_launch(
            &db,
            "parent",
            "turn-decline",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        assert_eq!(
            store::session_entries(&db, "parent")
                .unwrap()
                .iter()
                .filter(|entry| entry.kind == "approval.requested")
                .count(),
            1,
            "a declined turn/scope must not create a dead follow-up card"
        );
    }

    #[test]
    fn policy_approval_is_idempotent_and_stale_branches_cannot_resolve() {
        let db = policy_fixture();
        let forest = session_forest::SessionForest::new(&db);
        let branch_point = forest
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Explain the auth module only"}),
            )
            .unwrap();
        let request = policy_request(&["src/auth/**"]);
        reserve_worker_launch(
            &db,
            "parent",
            "turn-stale",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        reserve_worker_launch(
            &db,
            "parent",
            "turn-stale",
            &request,
            "gpt-5.6-terra",
            true,
        )
        .unwrap();
        let entries = store::session_entries(&db, "parent").unwrap();
        let approvals = entries
            .iter()
            .filter(|entry| entry.kind == "approval.requested")
            .collect::<Vec<_>>();
        assert_eq!(approvals.len(), 1);
        let approval = approvals[0];
        forest.move_head("parent", Some(&branch_point.id)).unwrap();
        forest
            .append(
                "parent",
                session_forest::EntryKind::UserMessage,
                serde_json::json!({"text":"Do not make any changes"}),
            )
            .unwrap();
        assert!(resolve_policy_delegation_approval(
            &db,
            "parent",
            approval.sequence,
            "accept",
            &approval.payload,
        )
        .is_err());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM worker_leases", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn approved_launch_failure_is_durable_and_retryable_for_the_turn() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        record_approved_launch_failure(&db, "parent", "turn-retry", &request).unwrap();
        let entries = session_forest::SessionForest::new(&db)
            .active_branch("parent")
            .unwrap();
        let failure = entries.last().unwrap();
        assert_eq!(failure.kind, "delegation.rejected");
        assert_eq!(failure.payload["reason"], "approved_launch_failed");
        assert_eq!(failure.payload["turnId"], "turn-retry");
        assert!(failure.payload["text"]
            .as_str()
            .unwrap()
            .contains("retried"));
    }

    #[test]
    fn unknown_model_hint_falls_back_and_records_warning_event() {
        let db = policy_fixture();
        let registry = adapters::AdapterRegistry::built_in().unwrap();
        let resolution = registry
            .resolve_model(
                "codex",
                CapabilityTier::Standard,
                Some("not-an-advertised-model"),
            )
            .unwrap();
        assert_eq!(resolution.actual_model, "gpt-5.6-terra");
        record_model_resolution_warning(&db, "parent", &resolution).unwrap();
        let (kind, body): (String, String) = db
            .query_row(
                "SELECT kind,body FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(kind, "capability.model_fallback");
        assert!(body.contains("not-an-advertised-model"));
        assert!(body.contains("gpt-5.6-terra"));
    }

    #[test]
    fn policy_budget_is_scoped_to_parent_turn() {
        let db = policy_fixture();
        let request = policy_request(&["src/auth/**"]);
        let outcome = policy::PolicyEngine::default().decide(&policy::PolicyInput {
            workspace_id: "w".into(),
            worktree_id: "w".into(),
            parent_session_id: "parent".into(),
            turn_id: "turn-1".into(),
            parent_depth: 0,
            request: request.clone(),
            owned_path_provenance: policy::OwnedPathProvenance {
                trusted_paths: request.owned_paths.clone(),
                source_entry_ids: vec!["test-user-entry".into()],
            },
            requested_harness: "codex".into(),
            task_family: "implementation".into(),
            active_workers: Vec::new(),
            warm_workers: Vec::new(),
            budget: policy::RequestBudget::default(),
            retry_count: 0,
            parent_can_execute: false,
            requires_user_approval: false,
            child_worktrees_available: false,
        });
        for index in 0..3 {
            policy::record_spawn_usage(
                &db,
                "w",
                "parent",
                "turn-1",
                &outcome,
                delegation::CapabilityTier::Standard,
            )
            .unwrap();
            assert!(index < 3);
        }
        assert!(reserve_worker_launch(&db, "parent", "turn-1", &request, "gpt-5.6-terra", true)
            .unwrap()
            .is_none());
        let next_turn = reserve_worker_launch(&db, "parent", "turn-2", &request, "gpt-5.6-terra", true)
            .unwrap()
            .expect("new turn should reset request counters");
        assert!(matches!(
            next_turn.outcome.decision,
            policy::RouteDecision::SpawnWorker(_)
        ));
    }
}
