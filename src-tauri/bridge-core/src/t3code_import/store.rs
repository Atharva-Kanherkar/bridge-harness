//! T3 Code's database, read-only: projects, threads, the native session
//! behind each thread, and its own copy of the messages.
//!
//! Two eras share one file format. Migrations 1 to 54 keep threads in
//! `projection_threads`; from 55 on they live in
//! `orchestration_v2_projection_threads`, and the v1 tables stay behind,
//! frozen, in the same file (T3 Code makes `statev2.sqlite` as a copy of
//! `state.sqlite`). A thread upgraded from v1 keeps its id, so its native
//! session is still found in `provider_session_runtime`.

use crate::import_sqlite::{int, text, Allowed, Row};
use crate::import_worktrees::Scripts;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

/// The oldest and newest migration ids Bridge reads. 5 creates the thread
/// tables; 102 is T3 Code's head at `pingdotgg/t3code@ccc1feb15`.
pub const OLDEST_MIGRATION: i64 = 5;
pub const NEWEST_MIGRATION: i64 = 102;
/// The first migration of the v2 era.
pub const FIRST_V2_MIGRATION: i64 = 55;

/// The only tables Bridge reads from T3 Code's database.
pub const ALLOWED_TABLES: &[&str] = &[
    "effect_sql_migrations",
    "projection_projects",
    "projection_threads",
    "projection_thread_messages",
    "provider_session_runtime",
    "orchestration_v2_projection_threads",
    "orchestration_v2_projection_provider_threads",
    "orchestration_v2_projection_messages",
    "orchestration_v2_legacy_imports",
];

/// Tables that hold credentials, webhooks, links to other machines or raw
/// event logs. Never read.
pub const DENIED_TABLES: &[&str] = &[
    "auth_pairing_links",
    "auth_sessions",
    "auth_connect_devices",
    "auth_connect_denied_requests",
    "scheduled_tasks",
    "scheduled_task_webhook_deliveries",
    "scheduled_task_webhook_pending",
    "scheduled_task_webhook_relay_deliveries",
    "scheduled_task_webhook_replay_keys",
    "peer_environment_links",
    "issue_tracker_bindings",
    "mcp_app_model_context",
    "orchestration_events",
    "orchestration_v2_events",
    "orchestration_command_receipts",
    "orchestration_v2_command_receipts",
    "orchestration_v2_effect_outbox",
    "checkpoint_diff_blobs",
];

const ALLOWED: Allowed = Allowed { tool: "T3 Code", tables: ALLOWED_TABLES };

/// Which tables hold the threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Era {
    V1,
    V2,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub title: Option<String>,
    pub root: PathBuf,
    pub deleted: bool,
    pub archived: bool,
    pub scripts: Option<Scripts>,
}

/// The agent session a thread ran as: `claude`, `codex` or `opencode`, and
/// that agent's own id for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NativeSession {
    pub harness: &'static str,
    pub id: String,
}

impl NativeSession {
    /// Bridge's key for it, as the importers record it (`claude:<id>`).
    pub fn key(&self) -> String {
        format!("{}:{}", self.harness, self.id)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Thread {
    pub id: String,
    pub project_id: String,
    pub title: Option<String>,
    pub branch: Option<String>,
    pub worktree: Option<PathBuf>,
    pub archived: bool,
    pub deleted: bool,
    pub pinned: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub model: Option<String>,
    /// The thread a subagent's thread belongs to.
    pub parent: Option<String>,
    /// T3 Code's driver id (`claudeAgent`, `codex`, `cursor`, ...).
    pub driver: Option<String>,
    pub native: Option<NativeSession>,
    pub pull_requests: usize,
    /// Upgraded from v1 and not yet copied over: its messages are still in
    /// `projection_thread_messages`.
    pub legacy_messages: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub version: i64,
    pub era: Era,
    pub projects: BTreeMap<String, Project>,
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

/// One message of T3 Code's own copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub text: String,
    pub at: Option<String>,
}

/// The harness Bridge knows for a T3 Code driver id. Others (`cursor`,
/// `grok`, ...) have no transcript Bridge reads.
pub fn harness_of(driver: &str) -> Option<&'static str> {
    match driver.trim() {
        "claudeAgent" | "claude" | "claudeCode" => Some("claude"),
        "codex" => Some("codex"),
        "opencode" => Some("opencode"),
        _ => None,
    }
}

fn json(row: &Row, column: &str) -> Value {
    text(row, column).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(Value::Null)
}

fn json_text(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned)
}

/// An id that can stand in a path or a native key: no separators, no
/// whitespace, not absurdly long.
fn plain_id(id: &str) -> Option<String> {
    let id = id.trim();
    (!id.is_empty() && id.len() <= 200 && !id.contains(['/', '\\', ':', '#']) && !id.chars().any(char::is_whitespace) && id != "." && id != "..")
        .then(|| id.to_owned())
}

/// The native session in a v1 resume cursor. Claude keeps its session id in
/// `resume` (older builds: `sessionId`) and T3 Code's own thread id in
/// `threadId`; every other driver keeps the native id in `threadId`.
pub fn native_from_cursor(provider: &str, cursor: &Value) -> Option<NativeSession> {
    let harness = harness_of(provider)?;
    let id = if harness == "claude" {
        json_text(cursor, "/resume").or_else(|| json_text(cursor, "/sessionId"))
    } else {
        json_text(cursor, "/threadId")
    }?;
    Some(NativeSession { harness, id: plain_id(&id)? })
}

/// The native session in a v2 provider thread's payload.
pub fn native_from_ref(driver: Option<&str>, payload: &Value) -> Option<NativeSession> {
    let driver = json_text(payload, "/nativeThreadRef/driver").or_else(|| driver.map(str::to_owned))?;
    let harness = harness_of(&driver)?;
    let id = json_text(payload, "/nativeThreadRef/nativeId")?;
    Some(NativeSession { harness, id: plain_id(&id)? })
}

/// A project's scripts as Bridge keeps them: the ones T3 Code runs when it
/// makes a worktree are the setup, the first other one is the run script.
pub fn scripts_of(raw: &Value) -> Option<Scripts> {
    let list = raw.as_array()?;
    let command = |script: &Value| json_text(script, "/command");
    let setup: Vec<String> = list
        .iter()
        .filter(|script| script.get("runOnWorktreeCreate").and_then(Value::as_bool) == Some(true))
        .filter_map(command)
        .collect();
    let run = list
        .iter()
        .filter(|script| script.get("runOnWorktreeCreate").and_then(Value::as_bool) != Some(true))
        .find_map(command);
    let scripts = Scripts { setup: (!setup.is_empty()).then(|| setup.join("\n")), run, run_mode: None, archive: None };
    (!scripts.is_empty()).then_some(scripts)
}

fn fail(error: rusqlite::Error) -> String {
    format!("Bridge could not read T3 Code's database: {error}")
}

/// Read T3 Code's database, or say why not. Fails closed: a version outside
/// [`OLDEST_MIGRATION`]..=[`NEWEST_MIGRATION`], a missing column Bridge
/// needs, or any read error returns the reason instead of a partial store.
pub fn read(conn: &Connection) -> Result<Store, String> {
    let version = ALLOWED
        .select(conn, "effect_sql_migrations", &["migration_id"])
        .map_err(fail)?
        .iter()
        .filter_map(|row| int(row, "migration_id"))
        .max()
        .ok_or_else(|| "T3 Code's database has no version Bridge can read".to_owned())?;
    if version > NEWEST_MIGRATION {
        return Err(format!("T3 Code's database is newer than Bridge knows (version {version}); update Bridge and try again"));
    }
    if version < OLDEST_MIGRATION {
        return Err(format!("T3 Code's database is older than Bridge knows (version {version})"));
    }
    let has_all = |table: &str, needed: &[&str]| -> Result<bool, String> {
        let present = ALLOWED.columns(conn, table).map_err(fail)?;
        Ok(needed.iter().all(|column| present.contains(*column)))
    };
    let era = if version >= FIRST_V2_MIGRATION { Era::V2 } else { Era::V1 };
    let threads_ok = match era {
        Era::V1 => has_all("projection_threads", &["thread_id", "project_id", "title", "deleted_at"])?,
        Era::V2 => has_all("orchestration_v2_projection_threads", &["thread_id", "project_id", "title", "deleted_at", "payload_json"])?,
    };
    if !threads_ok || !has_all("projection_projects", &["project_id", "workspace_root"])? {
        return Err(format!("T3 Code's database is missing what Bridge reads (version {version})"));
    }
    let projects = read_projects(conn)?;
    let cursors = read_cursors(conn)?;
    let threads = match era {
        Era::V1 => read_v1_threads(conn, &cursors)?,
        Era::V2 => read_v2_threads(conn, &cursors)?,
    };
    Ok(Store { version, era, projects, threads })
}

fn read_projects(conn: &Connection) -> Result<BTreeMap<String, Project>, String> {
    let wanted = ["project_id", "title", "workspace_root", "scripts_json", "deleted_at", "archived_at"];
    let mut out = BTreeMap::new();
    for row in ALLOWED.select(conn, "projection_projects", &wanted).map_err(fail)? {
        let (Some(id), Some(root)) = (text(&row, "project_id"), text(&row, "workspace_root")) else { continue };
        out.insert(
            id.clone(),
            Project {
                id,
                title: text(&row, "title"),
                root: PathBuf::from(root),
                deleted: text(&row, "deleted_at").is_some(),
                archived: text(&row, "archived_at").is_some(),
                scripts: scripts_of(&json(&row, "scripts_json")),
            },
        );
    }
    Ok(out)
}

/// Native sessions from v1 resume cursors, by thread id.
fn read_cursors(conn: &Connection) -> Result<HashMap<String, NativeSession>, String> {
    let mut out = HashMap::new();
    for row in ALLOWED
        .select(conn, "provider_session_runtime", &["thread_id", "provider_name", "resume_cursor_json"])
        .map_err(fail)?
    {
        let (Some(thread), Some(provider)) = (text(&row, "thread_id"), text(&row, "provider_name")) else { continue };
        if let Some(native) = native_from_cursor(&provider, &json(&row, "resume_cursor_json")) {
            out.insert(thread, native);
        }
    }
    Ok(out)
}

fn read_v1_threads(conn: &Connection, cursors: &HashMap<String, NativeSession>) -> Result<Vec<Thread>, String> {
    let wanted = [
        "thread_id", "project_id", "title", "branch", "worktree_path", "archived_at", "deleted_at", "pinned_at",
        "created_at", "updated_at", "model", "model_selection_json", "linked_pull_request_json",
    ];
    let mut drivers: HashMap<String, String> = HashMap::new();
    for row in ALLOWED.select(conn, "provider_session_runtime", &["thread_id", "provider_name"]).map_err(fail)? {
        if let (Some(thread), Some(provider)) = (text(&row, "thread_id"), text(&row, "provider_name")) {
            drivers.insert(thread, provider);
        }
    }
    let mut out = Vec::new();
    for row in ALLOWED.select(conn, "projection_threads", &wanted).map_err(fail)? {
        let (Some(id), Some(project_id)) = (text(&row, "thread_id"), text(&row, "project_id")) else { continue };
        let selection = json(&row, "model_selection_json");
        out.push(Thread {
            native: cursors.get(&id).cloned(),
            driver: drivers.get(&id).cloned().or_else(|| json_text(&selection, "/provider")),
            project_id,
            title: text(&row, "title"),
            branch: text(&row, "branch"),
            worktree: text(&row, "worktree_path").map(PathBuf::from),
            archived: text(&row, "archived_at").is_some(),
            deleted: text(&row, "deleted_at").is_some(),
            pinned: text(&row, "pinned_at").is_some(),
            created_at: text(&row, "created_at"),
            updated_at: text(&row, "updated_at"),
            model: json_text(&selection, "/model").or_else(|| text(&row, "model")),
            parent: None,
            pull_requests: usize::from(json(&row, "linked_pull_request_json").is_object()),
            legacy_messages: false,
            id,
        });
    }
    Ok(out)
}

fn read_v2_threads(conn: &Connection, cursors: &HashMap<String, NativeSession>) -> Result<Vec<Thread>, String> {
    // Each thread's provider threads, by id; the active one wins, else the
    // one used last.
    let mut by_id: HashMap<String, (Option<String>, Option<String>, Option<NativeSession>)> = HashMap::new();
    let mut latest: HashMap<String, (String, Option<NativeSession>, Option<String>)> = HashMap::new();
    for row in ALLOWED
        .select(conn, "orchestration_v2_projection_provider_threads", &["provider_thread_id", "thread_id", "driver", "updated_at", "payload_json"])
        .map_err(fail)?
    {
        let Some(id) = text(&row, "provider_thread_id") else { continue };
        let driver = text(&row, "driver");
        let native = native_from_ref(driver.as_deref(), &json(&row, "payload_json"));
        let thread = text(&row, "thread_id");
        let updated = text(&row, "updated_at").unwrap_or_default();
        if let Some(thread) = thread.clone() {
            let newer = latest.get(&thread).is_none_or(|(at, _, _)| updated > *at);
            if newer && native.is_some() {
                latest.insert(thread, (updated, native.clone(), driver.clone()));
            }
        }
        by_id.insert(id, (thread, driver, native));
    }
    let mut untranscribed: HashSet<String> = HashSet::new();
    for row in ALLOWED
        .select(conn, "orchestration_v2_legacy_imports", &["thread_id", "transcript_imported_at"])
        .map_err(fail)?
    {
        if let (Some(thread), None) = (text(&row, "thread_id"), text(&row, "transcript_imported_at")) {
            untranscribed.insert(thread);
        }
    }
    let wanted = [
        "thread_id", "project_id", "title", "default_provider", "active_provider_thread_id", "created_at", "updated_at",
        "archived_at", "deleted_at", "payload_json",
    ];
    let mut out = Vec::new();
    for row in ALLOWED.select(conn, "orchestration_v2_projection_threads", &wanted).map_err(fail)? {
        let (Some(id), Some(project_id)) = (text(&row, "thread_id"), text(&row, "project_id")) else { continue };
        let payload = json(&row, "payload_json");
        let active = text(&row, "active_provider_thread_id")
            .and_then(|active| by_id.get(&active))
            .filter(|(owner, _, native)| owner.as_deref().is_none_or(|owner| owner == id) && native.is_some());
        let (driver, native) = match active {
            Some((_, driver, native)) => (driver.clone(), native.clone()),
            None => match latest.get(&id) {
                Some((_, native, driver)) => (driver.clone(), native.clone()),
                None => (None, cursors.get(&id).cloned()),
            },
        };
        let lineage_parent = json_text(&payload, "/lineage/parentThreadId");
        let is_subagent = json_text(&payload, "/lineage/relationshipToParent").as_deref() == Some("subagent");
        let pull_requests = payload.get("pullRequests").and_then(Value::as_array).map_or(0, Vec::len)
            .max(usize::from(payload.get("linkedPullRequest").is_some_and(Value::is_object)));
        out.push(Thread {
            driver: driver
                .or_else(|| json_text(&payload, "/modelSelection/instanceId"))
                .or_else(|| text(&row, "default_provider")),
            native,
            project_id,
            title: text(&row, "title"),
            branch: json_text(&payload, "/branch"),
            worktree: json_text(&payload, "/worktreePath").map(PathBuf::from),
            archived: text(&row, "archived_at").is_some(),
            deleted: text(&row, "deleted_at").is_some() || json_text(&payload, "/deletedAt").is_some(),
            pinned: json_text(&payload, "/pinnedAt").is_some(),
            created_at: text(&row, "created_at"),
            updated_at: text(&row, "updated_at"),
            model: json_text(&payload, "/modelSelection/model"),
            parent: if is_subagent { lineage_parent } else { None },
            pull_requests,
            legacy_messages: untranscribed.contains(&id),
            id,
        });
    }
    Ok(out)
}

/// T3 Code's own copy of a thread's messages, user and assistant text in
/// order. Messages still streaming, empty ones and other roles are left out.
pub fn messages(conn: &Connection, era: Era, thread: &Thread) -> Result<Vec<Message>, String> {
    let legacy = era == Era::V1 || thread.legacy_messages;
    let rows = if legacy {
        ALLOWED.select_where(
            conn,
            "projection_thread_messages",
            &["message_id", "role", "text", "is_streaming", "created_at"],
            Some("thread_id = ?1 ORDER BY created_at, message_id"),
            &[&thread.id],
        )
    } else {
        ALLOWED.select_where(
            conn,
            "orchestration_v2_projection_messages",
            &["message_id", "role", "streaming", "created_at", "payload_json"],
            Some("thread_id = ?1 ORDER BY created_at, message_id"),
            &[&thread.id],
        )
    }
    .map_err(fail)?;
    let mut out = Vec::new();
    for row in rows {
        let streaming = int(&row, if legacy { "is_streaming" } else { "streaming" }).is_some_and(|flag| flag != 0);
        let role = match text(&row, "role").as_deref() {
            Some("user") => Role::User,
            Some("assistant") => Role::Assistant,
            _ => continue,
        };
        let body = if legacy {
            text(&row, "text")
        } else {
            json(&row, "payload_json").get("text").and_then(Value::as_str).filter(|body| !body.trim().is_empty()).map(str::to_owned)
        };
        let (Some(id), Some(body)) = (text(&row, "message_id"), body) else { continue };
        if streaming {
            continue;
        }
        out.push(Message { id, role, text: body, at: text(&row, "created_at") });
    }
    Ok(out)
}
