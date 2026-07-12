mod adapters;
mod agent;
mod codex_adapter;
mod git;
mod model;
mod store;

use chrono::Utc;
use model::*;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
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
    worktrees: PathBuf,
    database_path: PathBuf,
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
            ("claude", which::which("claude").is_ok()),
            ("codex", which::which("codex").is_ok()),
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
    harness: Harness,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    if title.trim().is_empty() {
        return Err(BridgeError::Invalid("Task title is required".into()));
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
    db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES(?1,?2,?3,?4,'idle','estimated')",params![sid,id,store::harness_name(&harness),harness.label()])?;
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
    harness: Harness,
    app: AppHandle,
    state: State<AppState>,
) -> Result<BridgeState, BridgeError> {
    let adapter_id = store::harness_name(&harness);
    let db = state.db.lock().unwrap();
    let path: String = db.query_row(
        "SELECT path FROM workspaces WHERE id=?1",
        params![workspace_id],
        |r| r.get(0),
    )?;
    let existing: Option<String> = db.query_row("SELECT id FROM sessions WHERE workspace_id=?1 AND harness=?2 AND status IN ('idle','stopped','failed','ready') ORDER BY rowid DESC LIMIT 1", params![workspace_id,adapter_id], |r| r.get(0)).ok();
    let session_id = existing
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    drop(db);
    if state.adapters.lock().unwrap().contains_key(&session_id) {
        return store::state(&state.db.lock().unwrap());
    }

    let started = state.adapter_registry.start(adapter_id, &path)?;
    let thread_id = started.runtime.provider_session_id().to_owned();
    let current_turn = started.runtime.current_turn();
    let mut reader = started.reader;
    let db = state.db.lock().unwrap();
    if existing.is_some() {
        db.execute("UPDATE sessions SET status='working',started_at=?2,ended_at=NULL,provider_session_id=?3,active_turn_id=NULL,metric_source='reported' WHERE id=?1", params![session_id,Utc::now().to_rfc3339(),thread_id])?;
    } else {
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,started_at,metric_source,provider_session_id) VALUES(?1,?2,?3,?4,'working',?5,'reported',?6)", params![session_id,workspace_id,adapter_id,harness.label(),Utc::now().to_rfc3339(),thread_id])?;
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
        &format!("Started {} structured adapter session", harness.label()),
    )?;
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

    let app_reader = app.clone();
    let sid_reader = session_id.clone();
    thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                        handle_agent_value(&app_reader, &sid_reader, &current_turn, &value);
                    }
                }
            }
        }
        let state = app_reader.state::<AppState>();
        state.adapters.lock().unwrap().remove(&sid_reader);
        let db = state.db.lock().unwrap();
        let workspace: Option<String> = db
            .query_row(
                "SELECT workspace_id FROM sessions WHERE id=?1",
                params![sid_reader],
                |r| r.get(0),
            )
            .ok();
        let _ = db.execute("UPDATE sessions SET status='stopped',ended_at=?2,active_turn_id=NULL WHERE id=?1 AND status IN ('working','waiting')", params![sid_reader,Utc::now().to_rfc3339()]);
        if let Some(workspace) = workspace {
            let _=db.execute("UPDATE workspaces SET status=CASE WHEN EXISTS(SELECT 1 FROM sessions WHERE workspace_id=?1 AND status IN ('working','waiting')) THEN 'working' ELSE 'stopped' END WHERE id=?1",params![workspace]);
        }
        let _ = app_reader.emit("state-changed", ());
    });
    let _ = app.emit("state-changed", ());
    store::state(&state.db.lock().unwrap())
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
    current_turn: &std::sync::Arc<Mutex<Option<String>>>,
    value: &serde_json::Value,
) {
    let state = app.state::<AppState>();
    let db = state.db.lock().unwrap();
    let session_context: Option<(String, String)> = db
        .query_row(
            "SELECT workspace_id,harness FROM sessions WHERE id=?1",
            params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let Some((workspace_id, adapter_id)) = session_context else {
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
    for normalized_event in normalized {
        if let Ok(event) = store::agent_event(
            &db,
            session_id,
            &normalized_event,
            &serde_json::json!({"adapter":adapter_id,"method":value.get("method")}),
        ) {
            let _ = app.emit("agent-event", event);
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
fn send_turn(session_id: String, text: String, state: State<AppState>) -> Result<(), BridgeError> {
    if text.trim().is_empty() {
        return Err(BridgeError::Invalid("Message cannot be empty".into()));
    }
    let adapters = state.adapters.lock().unwrap();
    let runtime = adapters
        .get(&session_id)
        .ok_or_else(|| BridgeError::Invalid("Structured adapter session is not running".into()))?;
    runtime.send_turn(&text)
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
    git::remove_worktree(Path::new(&repo), Path::new(&path))?;
    db.execute(
        "DELETE FROM sessions WHERE workspace_id=?1",
        params![workspace_id],
    )?;
    db.execute("DELETE FROM workspaces WHERE id=?1", params![workspace_id])?;
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

fn start_health_server(database: PathBuf) {
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
                        "harnesses": {
                            "claude": which::which("claude").is_ok(),
                            "codex": which::which("codex").is_ok(),
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
            start_health_server(db_path.clone());
            app.manage(AppState {
                db: Mutex::new(connection),
                runtimes: Mutex::new(HashMap::new()),
                adapters: Mutex::new(HashMap::new()),
                adapter_registry: adapters::AdapterRegistry::built_in()
                    .map_err(|error| Box::<dyn std::error::Error>::from(error))?,
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
    #[test]
    fn session_status_round_trip() {
        assert_eq!(store::status("waiting"), SessionStatus::Waiting);
    }
}
