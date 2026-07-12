mod adapters;
mod agent;
mod binary;
mod claude_adapter;
mod codex_adapter;
mod delegation;
mod git;
mod model;
mod orchestrator;
mod session_forest;
mod store;

use chrono::Utc;
use model::*;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
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
    /// `{session}::{item_id}` of assistant messages already turned into spawns,
    /// so re-observing the same message never double-spawns workers.
    spawned: HashSet<String>,
    /// Parent session id → count of child workers that have not yet reported a
    /// first result. A parent forwards its own result only when this is zero.
    outstanding: HashMap<String, i64>,
    /// Child sessions that have reported to their parent at least once.
    reported: HashSet<String>,
    /// Tracks the single same-session repair allowed for malformed worker output.
    result_repairs: delegation::ResultRepairTracker,
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

#[tauri::command]
fn create_workspace(
    project_id: String,
    title: String,
    _harness: Harness,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    if title.trim().is_empty() {
        return Err(BridgeError::Invalid("Workspace name is required".into()));
    }
    let db = state.db.lock().unwrap();
    let (project_name, repo): (String, String) = db.query_row(
        "SELECT name,path FROM projects WHERE id=?1",
        params![project_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let used: Vec<String> = {
        let mut stmt = db.prepare("SELECT city FROM workspaces")?;
        let values = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        values
    };
    let city = git::CITIES
        .iter()
        .find(|c| !used.iter().any(|u| u == **c))
        .unwrap_or(&"Atlas")
        .to_string();
    let id = Uuid::new_v4().to_string();
    let slug = git::slug(&title);
    let branch = format!(
        "bridge/{}-{}",
        if slug.is_empty() { "task" } else { &slug },
        city.to_lowercase()
    );
    let path = git::workspace_path(&state.worktrees, &project_name, &city);
    git::create_worktree(Path::new(&repo), &path, &branch)?;
    db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,'idle',?7)",params![id,project_id,city,title,branch,path.to_string_lossy(),Utc::now().to_rfc3339()])?;
    let sid = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model) VALUES(?1,?2,?3,?4,'idle','estimated',?5)",
        params![
            sid,
            id,
            orchestrator::HARNESS,
            orchestrator::SESSION_LABEL,
            orchestrator::MODEL
        ],
    )?;
    store::event(
        &db,
        "supervisor",
        "workspace.created",
        &id,
        &format!("Created {city} on {branch}"),
    )?;
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
    // Starter path: never let the UI pick harness/model. Always open Bridge's
    // Codex orchestrator on GPT Luna. Worker routing comes later.
    let adapter_id = orchestrator::HARNESS;
    let session_label = orchestrator::SESSION_LABEL;
    let chosen_model = Some(orchestrator::MODEL.to_owned());
    let db = state.db.lock().unwrap();
    let path: String = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get(0),
    )?;
    let existing: Option<String> = db.query_row(
        "SELECT id FROM sessions WHERE workspace_id=?1 AND harness=?2 AND status IN ('idle','stopped','failed','ready','working','waiting') ORDER BY rowid DESC LIMIT 1",
        params![workspace_id, adapter_id],
        |r| r.get(0),
    ).ok();
    let session_id = existing
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    drop(db);
    if state.adapters.lock().unwrap().contains_key(&session_id) {
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
            return store::state(&state.db.lock().unwrap());
        }
        if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
            runtime.stop();
        }
    }

    // The orchestrator is depth 0. It gets the routing briefing plus the shared
    // delegation protocol so it can spawn workers itself.
    let orchestrator_instructions =
        format!("{}\n\n{}", orchestrator::briefing(), delegation::protocol(0));
    let started = state.adapter_registry.start(
        adapter_id,
        &path,
        chosen_model.as_deref(),
        None,
        Some(orchestrator_instructions.as_str()),
    )?;
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let reader = started.reader;
    let db = state.db.lock().unwrap();
    if existing.is_some() {
        db.execute(
            "UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported',model=?4,label=?5,depth=0,parent_session_id=NULL WHERE id=?1",
            params![
                session_id,
                Utc::now().to_rfc3339(),
                thread_id,
                chosen_model,
                session_label
            ],
        )?;
    } else {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,provider_session_id,model,depth) VALUES(?1,?2,?3,?4,'working',?5,'reported',?6,?7,0)",
            params![
                session_id,
                workspace_id,
                adapter_id,
                session_label,
                Utc::now().to_rfc3339(),
                thread_id,
                chosen_model
            ],
        )?;
    }
    db.execute(
        "UPDATE workspaces SET status='working' WHERE id=?1",
        params![workspace_id],
    )?;
    store::event(
        &db,
        "adapter",
        "session.started",
        &session_id,
        &format!("Started {session_label} on {}", chosen_model.as_deref().unwrap_or("default")),
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
                "source": "hardcoded-benchmarks",
                "benchmarks": ["swe-bench-pro", "routing-heuristics"],
                "defaultModel": orchestrator::MODEL
            }),
        };
        let _ = store::agent_event(
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

    spawn_reader_thread(app.clone(), session_id.clone(), current_turn, reader);
    let _ = app.emit("state-changed", ());
    store::state(&state.db.lock().unwrap())
}

/// Drive one structured session's stdout: normalize every frame, then on exit
/// mark the session stopped and unblock any parent that was waiting on it.
fn spawn_reader_thread(
    app: AppHandle,
    session_id: String,
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
        state.adapters.lock().unwrap().remove(&session_id);
        notify_parent_on_worker_exit(&app, &session_id);
        let db = state.db.lock().unwrap();
        let workspace: Option<String> = db
            .query_row(
                "SELECT workspace_id FROM sessions WHERE id=?1",
                params![session_id],
                |r| r.get(0),
            )
            .ok();
        let _ = db.execute("UPDATE sessions SET status='stopped',ended_at=?2,active_turn_id=NULL WHERE id=?1 AND status IN ('working','waiting')", params![session_id,Utc::now().to_rfc3339()]);
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
            store::agent_event(
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
    let state = app.state::<AppState>();
    let mut pending_directives: Vec<delegation::DelegationRequest> = Vec::new();
    let mut turn_completed = false;

    {
        let db = state.db.lock().unwrap();
        let session_context: Option<(String, String, i64)> = db
            .query_row(
                "SELECT workspace_id,harness,COALESCE(depth,0) FROM sessions WHERE id=?1",
                params![session_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok();
        let Some((workspace_id, adapter_id, own_depth)) = session_context else {
            return;
        };
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
                    let _ = db.execute(
                        "UPDATE sessions SET status='working',active_turn_id=?2 WHERE id=?1",
                        params![session_id, turn_id],
                    );
                }
                "turn.completed" => {
                    turn_completed = true;
                    *current_turn.lock().unwrap() = None;
                    let _ = db.execute(
                        "UPDATE sessions SET status='ready',active_turn_id=NULL WHERE id=?1",
                        params![session_id],
                    );
                    let _ = db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'ready' END WHERE id=?1",params![workspace_id]);
                }
                "approval.requested" => {
                    let _ = db.execute(
                        "UPDATE sessions SET status='waiting' WHERE id=?1",
                        params![session_id],
                    );
                    let _ = db.execute(
                        "UPDATE workspaces SET status='waiting' WHERE id=?1",
                        params![workspace_id],
                    );
                }
                "error" if event.status.as_deref() == Some("failed") => {
                    let _ = db.execute(
                        "UPDATE sessions SET status='failed' WHERE id=?1",
                        params![session_id],
                    );
                    let _ = db.execute(
                        "UPDATE workspaces SET status='failed' WHERE id=?1",
                        params![workspace_id],
                    );
                }
                _ => {}
            }
        }
        for mut normalized_event in normalized {
            // A completed assistant message may carry delegation directives.
            // Spawn the workers (after the lock is released) and strip the raw
            // directive block so the conversation shows prose, not machine JSON.
            if normalized_event.kind == "message.completed"
                && normalized_event.role.as_deref() == Some("assistant")
            {
                if let Some(text) = normalized_event.text.clone() {
                    match delegation::parse_delegation_requests(&text) {
                        delegation::ParseOutcome::Parsed(requests) => {
                            let key = format!(
                                "{session_id}::{}",
                                normalized_event.item_id.clone().unwrap_or_default()
                            );
                            let is_new = state.delegations.lock().unwrap().spawned.insert(key);
                            let mut accepted_count = 0;
                            let mut rejection_message = None;
                            if is_new {
                                let selection = delegation::select_transport_requests(
                                    requests,
                                    own_depth,
                                    delegation::MAX_FANOUT,
                                );
                                accepted_count = selection.accepted.len();
                                pending_directives.extend(selection.accepted);
                                for rejection in selection.rejections {
                                    rejection_message = Some(match rejection.reason {
                                        delegation::DelegationRejectionReason::DepthLimit => {
                                            "Delegation rejected: workers cannot directly spawn workers."
                                        }
                                        delegation::DelegationRejectionReason::FanoutLimit => {
                                            "Delegation partially rejected: fanout limit reached."
                                        }
                                    });
                                    if let Ok(event) = record_delegation_rejection(
                                        &db,
                                        session_id,
                                        &rejection,
                                    ) {
                                        let _ = app.emit("agent-event", event);
                                    }
                                }
                            }
                            let stripped = delegation::strip_directives(&text);
                            normalized_event.text = Some(if stripped.is_empty() {
                                if accepted_count > 0 {
                                    "_Delegating to a worker…_".to_owned()
                                } else if let Some(message) = rejection_message {
                                    format!("_{message}_")
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
            if let Ok(event) = store::agent_event(
                &db,
                session_id,
                &normalized_event,
                &serde_json::json!({"adapter":adapter_id,"method":value.get("method")}),
            ) {
                let _ = app.emit("agent-event", event);
            }
        }
    }

    for directive in &pending_directives {
        launch_worker(app, session_id, directive);
    }
    // When this session's own turn ends and it is not waiting on any child
    // worker, hand its result up to its parent (no-op if it has no parent).
    if turn_completed {
        let idle = state
            .delegations
            .lock()
            .unwrap()
            .outstanding
            .get(session_id)
            .copied()
            .unwrap_or(0)
            == 0;
        if idle {
            forward_turn_result(app, session_id);
        }
    }
    let _ = app.emit("state-changed", ());
}

fn record_delegation_rejection(
    db: &Connection,
    session_id: &str,
    rejection: &delegation::DelegationRejection,
) -> Result<AgentEvent, BridgeError> {
    let reason = serde_json::to_string(rejection)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "delegation",
        "delegation.request.rejected",
        session_id,
        &reason,
    )?;
    let event = agent::NormalizedEvent {
        kind: "delegation.rejected".into(),
        item_id: Some(format!("rejection-{}", Uuid::new_v4())),
        role: Some("system".into()),
        status: Some("rejected".into()),
        title: Some("Delegation rejected".into()),
        text: Some(reason),
        data: serde_json::to_value(rejection)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?,
    };
    store::agent_event(db, session_id, &event, &serde_json::json!({"delegation":true}))
}

/// Spawn a child worker session in the parent's workspace and hand it its task.
fn launch_worker(
    app: &AppHandle,
    parent_session_id: &str,
    directive: &delegation::DelegationRequest,
) {
    let state = app.state::<AppState>();
    let info: Option<(String, i64, String, String)> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT s.workspace_id,COALESCE(s.depth,0),w.path,w.branch FROM sessions s JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
            params![parent_session_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .ok()
    };
    let Some((workspace_id, parent_depth, path, branch)) = info else {
        return;
    };
    let depth = parent_depth + 1;
    if depth > delegation::DEFAULT_MAX_DEPTH {
        return;
    }
    let harness = directive.runtime_harness();
    let model = directive.runtime_model();
    let effort = directive.effort.as_str().to_owned();
    let label = directive.label();
    let instructions = delegation::worker_briefing(directive, depth, &branch);

    let started = match state.adapter_registry.start(
        &harness,
        &path,
        Some(model.as_str()),
        Some(&effort),
        Some(instructions.as_str()),
    ) {
        Ok(started) => started,
        Err(error) => {
            let db = state.db.lock().unwrap();
            let _ = store::event(
                &db,
                "delegation",
                "worker.failed",
                parent_session_id,
                &format!("Could not start {label}: {error}"),
            );
            drop(db);
            let _ = app.emit("state-changed", ());
            return;
        }
    };
    let session_id = Uuid::new_v4().to_string();
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let reader = started.reader;

    {
        let db = state.db.lock().unwrap();
        let _ = db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,provider_session_id,model,effort,parent_session_id,depth) VALUES(?1,?2,?3,?4,'working',?5,'reported',?6,?7,?8,?9,?10)",
            params![
                session_id,
                workspace_id,
                harness,
                label,
                Utc::now().to_rfc3339(),
                thread_id,
                model,
                effort,
                parent_session_id,
                depth
            ],
        );
        let _ = db.execute(
            "UPDATE workspaces SET status='working' WHERE id=?1",
            params![workspace_id],
        );
        for message in &started.startup_messages {
            let _ = persist_agent_value(&db, &state.adapter_registry, &harness, &session_id, message);
        }
        let spawn_event = agent::NormalizedEvent {
            kind: "delegation.spawned".into(),
            item_id: Some(format!("spawn-{session_id}")),
            role: Some("system".into()),
            status: Some("working".into()),
            title: Some(format!("Delegated to {label}")),
            text: Some(directive.objective.clone()),
            data: serde_json::json!({
                "childSessionId": session_id,
                "request": directive,
                "harness": harness,
                "model": model,
                "modelLabel": delegation::model_display(&model),
                "effort": effort,
                "depth": depth,
            }),
        };
        if let Ok(stored) =
            store::agent_event(&db, parent_session_id, &spawn_event, &serde_json::json!({"delegation": true}))
        {
            let _ = app.emit("agent-event", stored);
        }
        let _ = store::event(
            &db,
            "delegation",
            "worker.spawned",
            parent_session_id,
            &format!(
                "Spawned {label} (effort {})",
                effort
            ),
        );
    }

    state
        .adapters
        .lock()
        .unwrap()
        .insert(session_id.clone(), started.runtime);
    spawn_reader_thread(app.clone(), session_id.clone(), current_turn, reader);
    *state
        .delegations
        .lock()
        .unwrap()
        .outstanding
        .entry(parent_session_id.to_owned())
        .or_insert(0) += 1;
    if let Some(runtime) = state.adapters.lock().unwrap().get(&session_id) {
        let _ = runtime.send_turn(&directive.objective);
    }
    let _ = app.emit("state-changed", ());
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
            "SELECT text FROM agent_events WHERE session_id=?1 AND kind='message.completed' AND role='assistant' AND text IS NOT NULL AND text<>'' ORDER BY sequence DESC LIMIT 1",
            params![child_session_id],
            |r| r.get(0),
        )
        .ok()
    };
    let raw_output =
        raw_output.unwrap_or_else(|| "(worker finished without a text summary)".to_owned());
    let result_text = match {
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
    let Some(result_text) = result_text else {
        let _ = app.emit("state-changed", ());
        return;
    };
    let model_label = delegation::model_display(model.as_deref().unwrap_or("unknown"));
    let framed = format!(
        "[worker result] {label} ({harness}/{model_label}, effort {}) finished:\n\n{result_text}",
        effort.as_deref().unwrap_or("medium")
    );
    report_to_parent(app, child_session_id, &framed);
}

fn process_worker_result_output(
    db: &Connection,
    tracker: &mut delegation::ResultRepairTracker,
    child_session_id: &str,
    raw_output: &str,
    send_same_session_repair: impl FnOnce(&str) -> bool,
) -> Result<Option<String>, BridgeError> {
    match tracker.process(
        child_session_id,
        raw_output,
        send_same_session_repair,
    ) {
        delegation::WorkerOutputAction::Structured(result) => Ok(Some(
            serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.summary.clone()),
        )),
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
        delegation::WorkerOutputAction::Unstructured { raw, reason } => {
            store::event(
                db,
                "delegation",
                "worker.result.unstructured",
                child_session_id,
                &reason,
            )?;
            Ok(Some(format!("[unstructured — {reason}]\n{raw}")))
        }
    }
}

/// If a worker process exits before ever reporting, tell its parent so the
/// parent is not left waiting on a child that will never answer.
fn notify_parent_on_worker_exit(app: &AppHandle, child_session_id: &str) {
    let state = app.state::<AppState>();
    let already = state
        .delegations
        .lock()
        .unwrap()
        .reported
        .contains(child_session_id);
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
    let framed = format!(
        "[worker stopped] {label} ended without reporting a result. You may retry, delegate differently, or proceed without it."
    );
    report_to_parent(app, child_session_id, &framed);
}

/// Deliver a framed message from a child to its parent session: send it into the
/// parent's live turn stream and drop a marker card into the parent's transcript.
fn report_to_parent(app: &AppHandle, child_session_id: &str, framed_text: &str) {
    let state = app.state::<AppState>();
    let parent_id: Option<String> = {
        let db = state.db.lock().unwrap();
        db.query_row(
            "SELECT parent_session_id FROM sessions WHERE id=?1",
            params![child_session_id],
            |r| r.get(0),
        )
        .ok()
        .flatten()
    };
    let Some(parent_id) = parent_id else {
        return;
    };
    let delivered = match state.adapters.lock().unwrap().get(&parent_id) {
        Some(runtime) => runtime.send_turn(framed_text).is_ok(),
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
            text: Some(framed_text.to_owned()),
            data: serde_json::json!({"childSessionId": child_session_id, "delivered": delivered}),
        };
        if let Ok(stored) =
            store::agent_event(&db, &parent_id, &result_event, &serde_json::json!({"delegation": true}))
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
    {
        let mut delegations = state.delegations.lock().unwrap();
        if delegations.reported.insert(child_session_id.to_owned()) {
            if let Some(count) = delegations.outstanding.get_mut(&parent_id) {
                if *count > 0 {
                    *count -= 1;
                }
            }
        }
    }
    let _ = app.emit("state-changed", ());
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
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    runtime.send_turn(&text)?;
    drop(adapters);
    let db = state.db.lock().unwrap();
    let adapter_id: String = db.query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    // Claude stream-json does not reliably echo the submitted user turn; persist it locally.
    if adapter_id == "claude" {
        let user_event = agent::NormalizedEvent {
            kind: "message.completed".into(),
            item_id: Some(format!("user-{}", Uuid::new_v4())),
            role: Some("user".into()),
            status: Some("completed".into()),
            title: None,
            text: Some(text),
            data: serde_json::json!({}),
        };
        let event = store::agent_event(
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

#[tauri::command]
fn interrupt_turn(session_id: String, state: State<AppState>) -> Result<(), BridgeError> {
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    runtime.interrupt()
}

#[tauri::command]
fn resolve_approval(
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
    let (session_id, data, adapter_id): (String, String, String) = db.query_row(
        "SELECT e.session_id,e.data,s.harness FROM agent_events e JOIN sessions s ON s.id=e.session_id WHERE e.id=?1 AND e.kind='approval.requested'",
        params![event_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let data: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| BridgeError::Invalid(format!("Approval metadata is invalid: {e}")))?;
    let request_id = data
        .get("requestId")
        .cloned()
        .ok_or_else(|| BridgeError::Invalid("Approval has no adapter request id".into()))?;
    drop(db);
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    runtime.respond(request_id, &decision)?;
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
    let event = store::agent_event(
        &db,
        &session_id,
        &normalized,
        &serde_json::json!({"adapter":adapter_id}),
    )?;
    db.execute(
        "UPDATE sessions SET status='working' WHERE id=?1",
        params![session_id],
    )?;
    let _ = app.emit("agent-event", event);
    let _ = app.emit("state-changed", ());
    Ok(())
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
    if let Some(mut runtime) = state.adapters.lock().unwrap().remove(&session_id) {
        runtime.stop();
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
        "DELETE FROM agent_events WHERE session_id IN (SELECT id FROM sessions WHERE workspace_id=?1)",
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
            // Sessions cannot outlive the app process; clear stale live statuses on boot.
            let _ = connection.execute(
                "UPDATE sessions SET status='stopped', ended_at=COALESCE(ended_at, ?1), active_turn_id=NULL WHERE status IN ('working','waiting','ready')",
                params![Utc::now().to_rfc3339()],
            );
            let _ = connection.execute(
                "UPDATE workspaces SET status='stopped' WHERE status IN ('working','waiting','ready')",
                [],
            );
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            health,
            get_state,
            add_project,
            create_workspace,
            start_session,
            open_terminal,
            write_terminal,
            resize_terminal,
            send_turn,
            interrupt_turn,
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
        db.execute("INSERT INTO agent_events(session_id,sequence,kind,data,provider_meta,created_at) VALUES('s',1,'message.completed','{}','{}','now')", []).unwrap();
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
        assert_eq!(store::status("waiting"), SessionStatus::Waiting);
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
            "agent_events",
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
            "agent_events",
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
        assert!(fallback.contains("[unstructured"));
        assert!(fallback.contains("Initial invalid output:\ninvalid first output"));
        assert!(fallback.contains("Invalid repair output:\ninvalid repair output"));
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
    fn delegation_rejection_is_stored_in_audit_and_normalized_history() {
        let db = archive_fixture();
        let rejection = delegation::DelegationRejection {
            reason: delegation::DelegationRejectionReason::DepthLimit,
            rejected_count: 2,
        };
        let event = record_delegation_rejection(&db, "s", &rejection).unwrap();
        assert_eq!(event.kind, "delegation.rejected");
        assert_eq!(event.data["reason"], "depth_limit");
        assert_eq!(event.data["rejectedCount"], 2);
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "delegation.request.rejected"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM agent_events WHERE session_id='s' AND kind='delegation.rejected'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
            1
        );
    }
}
