//! Orca's data, read-only: each profile's repositories and worktrees, the
//! chats every profile shares, the native session behind each chat, Orca's
//! own copy of each chat, and which chats a multi-agent run tied together.
//!
//! Three databases, none with a migration table: their schema version is
//! `PRAGMA user_version`. Profile state keeps one JSON document per key, and
//! only the documents named in [`PROFILE_DOCUMENTS`] are ever selected; the
//! rest (settings, SSH targets, window state) hold keys and tokens.

use crate::import_sqlite::{int, missing_reason, text, user_version, Allowed, Gate, Reads, Versions};
use crate::import_worktrees::Scripts;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The chats, at the data folder's root.
pub const CHAT_DB: &str = "agent-session-domain.db";
/// Where chats were before [`CHAT_DB`]. Orca converts it on its next start.
pub const OLD_CHAT_DB: &str = "agent-session-journal.db";
pub const ORCHESTRATION_DB: &str = "orchestration.db";
pub const PROFILE_INDEX: &str = "orca-profile-index.json";
/// `profiles/<id>/profile-state.db`.
pub const PROFILE_DB: &str = "profile-state.db";

/// `user_version`s Bridge reads. The newest checked are Orca 1.4.271's
/// (`stablyai/orca@1f4e0b30`). A newer database is read while it has all of
/// [`CHAT_NEWEST_READS`] or [`PROFILE_NEWEST_READS`].
pub const CHAT_VERSIONS: Versions = Versions { oldest: 1, newest_checked: 2 };
pub const PROFILE_VERSIONS: Versions = Versions { oldest: 1, newest_checked: 3 };

/// Every column Bridge reads from each database at its newest checked
/// version. A newer database without one of them changed what Bridge reads,
/// and is refused.
pub const CHAT_NEWEST_READS: Reads = &[
    ("agent_session_records", &["session_id", "record_json"]),
    ("agent_session_store_meta", &["key", "value"]),
    ("journal_chat_state", &["session_id", "epoch"]),
    ("journal_items", &["item_id", "payload_json", "session_id", "epoch", "hidden", "sequence", "sequence_index", "ordinal"]),
];
pub const PROFILE_NEWEST_READS: Reads = &[
    ("profile_state_documents", &["domain", "payload", "revision"]),
    ("profile_state_document_parts_meta", &["domain", "legacy_revision"]),
    ("profile_state_document_parts", &["part_key", "payload", "domain", "ordinal"]),
];
/// The `schemaVersion` document of profile state.
pub const STATE_SCHEMA: i64 = 1;

/// The only tables Bridge reads from each database.
pub const CHAT_TABLES: &[&str] = &["agent_session_records", "agent_session_store_meta", "journal_chat_state", "journal_items"];
pub const PROFILE_TABLES: &[&str] = &["profile_state_documents", "profile_state_document_parts", "profile_state_document_parts_meta"];
pub const ORCHESTRATION_TABLES: &[&str] = &["runs", "dispatch_contexts", "worker_terminal_resources"];

/// The only profile documents Bridge reads.
pub const PROFILE_DOCUMENTS: &[&str] = &["schemaVersion", "repos", "folderWorkspaces", "worktreeMeta"];

/// Files and folders under Orca's data folder (or a profile's) that hold
/// keys, tokens, sign-ins or browser data. Never opened.
pub const DENIED_FILES: &[&str] = &[
    "orca-data.json",
    "orca-runtime.json",
    "orca-e2ee-keypair.json",
    "orca-devices.json",
    "orca-secret-protection.json",
    "connector-host-key.json",
    "agent-session-authority.key",
    "agent-hooks",
    "claude-profiles",
    "claude-runtime-auth",
    "codex-accounts",
    "codex-runtime-home",
    "antigravity-accounts",
    "managed-data-accounts",
    "plugins-data",
    "Partitions",
    "google-setup-keys",
    "bot-tool-sign-in",
    "account-session.json.enc",
    "browser-passwords",
];

const CHATS: Allowed = Allowed { tool: "Orca", tables: CHAT_TABLES };
const PROFILE: Allowed = Allowed { tool: "Orca", tables: PROFILE_TABLES };
const RUNS: Allowed = Allowed { tool: "Orca", tables: ORCHESTRATION_TABLES };

/// The agent session a chat ran as: `claude`, `codex` or `opencode`, and
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

/// The harness Bridge runs for an Orca agent id. Others (`pi`, `gemini`,
/// `cursor`, ...) are agents Bridge does not run.
pub fn harness_of(agent: &str) -> Option<&'static str> {
    match agent.trim() {
        "claude" => Some("claude"),
        "codex" => Some("codex"),
        "opencode" => Some("opencode"),
        _ => None,
    }
}

/// An id that can stand in a path or a native key: no separators, no
/// whitespace, not absurdly long.
pub fn plain_id(id: &str) -> Option<String> {
    let id = id.trim();
    (!id.is_empty() && id.len() <= 200 && !id.contains(['/', '\\', ':', '#']) && !id.chars().any(char::is_whitespace) && id != "." && id != "..")
        .then(|| id.to_owned())
}

/// Where a chat or worktree id points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// `<repo id>::<path>`: a worktree, or the repository's main checkout.
    Worktree { repo: String, path: PathBuf },
    /// `folder:<id>`: a folder workspace.
    Folder(String),
    /// Anything else (`global-floating-terminal`, garbage).
    None,
}

/// Read a worktree id: `<repo id>::<absolute path>`, maybe followed by
/// `::workspace:<id>`. The repository id never holds `::`; the path may.
pub fn place_of(id: &str) -> Place {
    if let Some(folder) = id.strip_prefix("folder:") {
        return plain_id(folder).map_or(Place::None, Place::Folder);
    }
    let Some((repo, rest)) = id.split_once("::") else { return Place::None };
    let path = match rest.rfind("::workspace:") {
        Some(at) if plain_id(&rest[at + "::workspace:".len()..]).is_some() => &rest[..at],
        _ => rest,
    };
    let path = PathBuf::from(path);
    match plain_id(repo) {
        Some(repo) if path.is_absolute() && !path.as_os_str().is_empty() => Place::Worktree { repo, path },
        _ => Place::None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub id: String,
    pub path: PathBuf,
    pub name: Option<String>,
    /// On another machine over SSH: its paths are not this Mac's.
    pub remote: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Orca's id for it, `<repo id>::<path>`.
    pub id: String,
    pub repo: String,
    pub path: PathBuf,
    pub name: Option<String>,
    pub archived: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub repos: Vec<Repo>,
    pub worktrees: Vec<Worktree>,
    /// Folder workspaces: id to folder.
    pub folders: BTreeMap<String, PathBuf>,
}

/// Where Orca launched an agent's account: `CLAUDE_CONFIG_DIR`, `CODEX_HOME`
/// or `HOME`, and the folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountHome {
    pub variable: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    /// Orca's session id.
    pub id: String,
    /// Orca's agent id (`claude`, `codex`, `opencode`, `pi`, ...).
    pub agent: String,
    pub place: Place,
    pub launch_directory: Option<PathBuf>,
    /// On this Mac, not over SSH or in WSL.
    pub local: bool,
    pub title: Option<String>,
    /// Renamed by the person in Orca.
    pub renamed: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub model: Option<String>,
    pub native: Option<NativeSession>,
    pub account_home: Option<AccountHome>,
}

/// A worker a multi-agent run dispatched.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Worker {
    /// It ran as an Orca chat.
    Chat(String),
    /// It ran in a terminal; Orca captured its session.
    Session(NativeSession),
    /// It ran in a terminal and Orca captured no session (OpenCode).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: String,
    pub objective: Option<String>,
    /// The chat that led it; none when a terminal did.
    pub coordinator: Option<String>,
    pub workers: Vec<Worker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    pub chat_version: i64,
    pub chats: Vec<Chat>,
    pub profiles: Vec<Profile>,
    /// `None` when there is no orchestration database Bridge can read.
    pub runs: Option<Vec<Run>>,
    /// What was not read, and why: one line each.
    pub notes: Vec<String>,
}

fn fail(what: &str) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |error| format!("Bridge could not read Orca's {what}: {error}")
}

fn json_text(value: &Value, pointer: &str) -> Option<String> {
    value.pointer(pointer).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned)
}

/// Orca's epoch milliseconds as RFC 3339.
pub fn time_of(ms: Option<i64>) -> Option<String> {
    ms.filter(|ms| *ms > 0).and_then(DateTime::<Utc>::from_timestamp_millis).map(|at| at.to_rfc3339())
}

/// The native session in the last link of a record's handle chain, from
/// its fields: Claude Code's `sessionId`, Codex's `threadId`, an ACP or
/// RPC agent's `nativeId`.
pub fn native_of(provider: Option<&str>, agent: Option<&str>, session: Option<&str>, thread: Option<&str>, native: Option<&str>) -> Option<NativeSession> {
    let (harness, id) = match provider {
        Some("claude") => ("claude", session?),
        Some("codex") => ("codex", thread?),
        Some(_) => return None,
        None => (harness_of(agent?)?, native?),
    };
    Some(NativeSession { harness, id: plain_id(id)? })
}

/// A table's columns include all of `needed`.
fn has_all(allowed: &Allowed, conn: &Connection, table: &str, needed: &[&str], what: &str) -> Result<bool, String> {
    let present = allowed.columns(conn, table).map_err(fail(what))?;
    Ok(needed.iter().all(|column| present.contains(*column)))
}

/// Refuse a database without what Bridge needs; for a newer one, without
/// any column of `newest_reads`.
fn require(allowed: &Allowed, conn: &Connection, what: &str, label: &str, (version, gate): (i64, Gate), needed: Reads, newest_reads: Reads) -> Result<(), String> {
    let missing = allowed.missing(conn, if gate == Gate::Newer { newest_reads } else { needed }).map_err(fail(what))?;
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing_reason(label, version, gate, &missing))
    }
}

/// Read the chats, or say why not. Fails closed on a version older than
/// [`CHAT_VERSIONS`], a missing column Bridge reads (for a version newer
/// than the newest checked: any of [`CHAT_NEWEST_READS`]), or any read
/// error.
pub fn read_chats(conn: &Connection) -> Result<(i64, Vec<Chat>), String> {
    let version = user_version(conn).map_err(fail("chats"))?;
    let label = "Orca's chat database";
    let gate = CHAT_VERSIONS.gate(label, version)?;
    require(&CHATS, conn, "chats", label, (version, gate), &[("agent_session_records", &["session_id", "record_json"])], CHAT_NEWEST_READS)?;
    let deleted = deleted_chats(conn)?;
    let fields = [
        ("id", "$.sessionId"),
        ("agent", "$.provider"),
        ("workspace", "$.location.workspaceId"),
        ("host", "$.location.executionHostId"),
        ("wsl", "$.location.wslDistro"),
        ("launch", "$.launchDirectory"),
        ("title", "$.conversationName"),
        ("title_source", "$.conversationNameSource"),
        ("created", "$.createdAt"),
        ("updated", "$.updatedAt"),
        ("model", "$.options.model"),
        ("home_variable", "$.accountHome.variable"),
        ("home_path", "$.accountHome.path"),
        ("handle_provider", "$.providerHandleChain[#-1].handle.provider"),
        ("handle_agent", "$.providerHandleChain[#-1].handle.agent"),
        ("handle_session", "$.providerHandleChain[#-1].handle.sessionId"),
        ("handle_thread", "$.providerHandleChain[#-1].handle.threadId"),
        ("handle_native", "$.providerHandleChain[#-1].handle.nativeId"),
    ];
    let rows = CHATS
        .select_json(conn, "agent_session_records", &["session_id"], "record_json", &fields, Some("1 ORDER BY session_id"), &[])
        .map_err(fail("chats"))?;
    let mut out = Vec::new();
    for row in rows {
        let Some(id) = text(&row, "session_id").and_then(|id| plain_id(&id)) else { continue };
        if deleted.contains(&id) || text(&row, "id").is_some_and(|inner| inner != id) {
            continue;
        }
        let agent = text(&row, "agent").unwrap_or_default();
        let local = text(&row, "host").is_none_or(|host| host == "local") && text(&row, "wsl").is_none();
        let account_home = match (text(&row, "home_variable"), text(&row, "home_path")) {
            (Some(variable), Some(path)) if Path::new(&path).is_absolute() => Some(AccountHome { variable, path: PathBuf::from(path) }),
            _ => None,
        };
        out.push(Chat {
            native: native_of(
                text(&row, "handle_provider").as_deref(),
                text(&row, "handle_agent").as_deref(),
                text(&row, "handle_session").as_deref(),
                text(&row, "handle_thread").as_deref(),
                text(&row, "handle_native").as_deref(),
            ),
            place: text(&row, "workspace").map_or(Place::None, |workspace| place_of(&workspace)),
            launch_directory: text(&row, "launch").map(PathBuf::from).filter(|path| path.is_absolute()),
            local,
            title: text(&row, "title").map(|title| title.trim().to_owned()),
            renamed: text(&row, "title_source").as_deref() == Some("user"),
            created_at: time_of(int(&row, "created")),
            updated_at: time_of(int(&row, "updated")),
            model: text(&row, "model"),
            account_home,
            agent,
            id,
        });
    }
    Ok((version, out))
}

/// Chats Orca's tombstones say were deleted (`chat.deleted:<id>`).
fn deleted_chats(conn: &Connection) -> Result<HashSet<String>, String> {
    let rows = CHATS
        .select_where(conn, "agent_session_store_meta", &["key", "value"], Some("key LIKE 'chat.deleted:%'"), &[])
        .map_err(fail("chats"))?;
    let mut out = HashSet::new();
    for row in rows {
        if let Some(id) = text(&row, "key").and_then(|key| key.strip_prefix("chat.deleted:").map(str::to_owned)) {
            out.insert(id);
        }
        let value: Value = text(&row, "value").and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(Value::Null);
        for id in value.get("sessionIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            out.insert(id.to_owned());
        }
    }
    Ok(out)
}

/// Read one profile's state, or say why not.
pub fn read_profile(conn: &Connection, id: &str) -> Result<Profile, String> {
    let what = "profile state";
    let version = user_version(conn).map_err(fail(what))?;
    let label = format!("Orca's profile '{id}'");
    let gate = PROFILE_VERSIONS.gate(&label, version)?;
    require(&PROFILE, conn, what, &label, (version, gate), &[("profile_state_documents", &["domain", "payload", "revision"])], PROFILE_NEWEST_READS)?;
    let placeholders = PROFILE_DOCUMENTS.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let params: Vec<&dyn rusqlite::ToSql> = PROFILE_DOCUMENTS.iter().map(|domain| domain as &dyn rusqlite::ToSql).collect();
    let filter = format!("domain IN ({placeholders})");
    let mut documents: HashMap<String, (Value, Option<i64>)> = HashMap::new();
    for row in PROFILE.select_where(conn, "profile_state_documents", &["domain", "payload", "revision"], Some(&filter), &params).map_err(fail(what))? {
        let (Some(domain), Some(payload)) = (text(&row, "domain"), text(&row, "payload")) else { continue };
        let value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        documents.insert(domain, (value, int(&row, "revision")));
    }
    if let Some((schema, _)) = documents.get("schemaVersion") {
        if schema.as_i64() != Some(STATE_SCHEMA) {
            return Err(format!("Orca's profile '{id}' is in a format Bridge does not know (schema {schema})"));
        }
    }
    let field = |value: &Value, key: &str| json_text(value, &format!("/{key}"));
    let flag = |value: &Value, key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    let mut profile = Profile { id: id.to_owned(), ..Profile::default() };
    for repo in documents.get("repos").and_then(|(value, _)| value.as_array()).into_iter().flatten() {
        let (Some(id), Some(path)) = (field(repo, "id").and_then(|id| plain_id(&id)), field(repo, "path")) else { continue };
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            continue;
        }
        profile.repos.push(Repo { id, path, name: field(repo, "displayName"), remote: field(repo, "connectionId").is_some() });
    }
    for folder in documents.get("folderWorkspaces").and_then(|(value, _)| value.as_array()).into_iter().flatten() {
        if let (Some(id), Some(path)) = (field(folder, "id").and_then(|id| plain_id(&id)), field(folder, "folderPath")) {
            if Path::new(&path).is_absolute() {
                profile.folders.insert(id, PathBuf::from(path));
            }
        }
    }
    let mut meta: BTreeMap<String, Value> = documents
        .get("worktreeMeta")
        .and_then(|(value, _)| value.as_object())
        .map(|map| map.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
        .unwrap_or_default();
    if let Some(parts) = current_parts(conn, documents.get("worktreeMeta").and_then(|(_, revision)| *revision))? {
        meta = parts;
    }
    for (id, value) in meta {
        let Place::Worktree { repo, path } = place_of(&id) else { continue };
        profile.worktrees.push(Worktree {
            name: field(&value, "displayName"),
            archived: flag(&value, "isArchived") || value.get("archivedCheckout").is_some_and(Value::is_object),
            id,
            repo,
            path,
        });
    }
    Ok(profile)
}

/// `worktreeMeta` from its split parts, when they are current: written at
/// the document's revision. Otherwise the document itself is.
fn current_parts(conn: &Connection, revision: Option<i64>) -> Result<Option<BTreeMap<String, Value>>, String> {
    let what = "profile state";
    let meta = PROFILE
        .select_where(conn, "profile_state_document_parts_meta", &["domain", "legacy_revision"], Some("domain = 'worktreeMeta'"), &[])
        .map_err(fail(what))?;
    let current = meta.first().and_then(|row| int(row, "legacy_revision")).is_some_and(|parts| Some(parts) == revision);
    if !current {
        return Ok(None);
    }
    let mut out = BTreeMap::new();
    for row in PROFILE
        .select_where(conn, "profile_state_document_parts", &["part_key", "payload"], Some("domain = 'worktreeMeta' ORDER BY ordinal"), &[])
        .map_err(fail(what))?
    {
        let key: Option<Vec<String>> = text(&row, "part_key").and_then(|raw| serde_json::from_str(&raw).ok());
        let (Some([id]), Some(payload)) = (key.as_deref(), text(&row, "payload")) else { continue };
        out.insert(id.clone(), serde_json::from_str(&payload).unwrap_or(Value::Null));
    }
    Ok(Some(out))
}

/// The multi-agent runs, or why they could not be read. Read at any
/// version that has the columns Bridge reads: Orca changes this database
/// often, and only the links between chats come from it.
pub fn read_runs(conn: &Connection) -> Result<Vec<Run>, String> {
    let what = "multi-agent runs";
    if !has_all(&RUNS, conn, "runs", &["id", "coordinator_orca_session_id"], what)?
        || !has_all(&RUNS, conn, "dispatch_contexts", &["id", "run_id", "assignee_orca_session_id"], what)?
    {
        return Err("Orca's multi-agent runs are in a format Bridge does not know".into());
    }
    let mut runs: BTreeMap<String, Run> = BTreeMap::new();
    let wanted = ["id", "objective", "coordinator_orca_session_id", "coordinator_orca_session_id_generation", "consumer_generation"];
    for row in RUNS.select(conn, "runs", &wanted).map_err(fail(what))? {
        let Some(id) = text(&row, "id") else { continue };
        // The chat counts only while it was bound at the run's current
        // generation; a rebind or unbind bumps the run's.
        let bound = match (int(&row, "coordinator_orca_session_id_generation"), int(&row, "consumer_generation")) {
            (Some(chat), Some(run)) => chat == run,
            _ => true,
        };
        let coordinator = text(&row, "coordinator_orca_session_id").and_then(|id| plain_id(&id)).filter(|_| bound);
        runs.insert(id.clone(), Run { id, objective: text(&row, "objective"), coordinator, workers: Vec::new() });
    }
    // A terminal worker's captured session, by the dispatch it ran for.
    let mut terminal: HashMap<String, NativeSession> = HashMap::new();
    let fields = [("agent", "$.agent"), ("session", "$.providerSession.id")];
    for row in RUNS
        .select_json(conn, "worker_terminal_resources", &["origin_dispatch_id", "owner_dispatch_id"], "resume_session", &fields, None, &[])
        .map_err(fail(what))?
    {
        let native = text(&row, "agent")
            .and_then(|agent| harness_of(&agent))
            .zip(text(&row, "session").and_then(|id| plain_id(&id)))
            .map(|(harness, id)| NativeSession { harness, id });
        let Some(native) = native else { continue };
        for dispatch in [text(&row, "origin_dispatch_id"), text(&row, "owner_dispatch_id")].into_iter().flatten() {
            terminal.entry(dispatch).or_insert_with(|| native.clone());
        }
    }
    for row in RUNS.select(conn, "dispatch_contexts", &["id", "run_id", "assignee_orca_session_id"]).map_err(fail(what))? {
        let (Some(id), Some(run)) = (text(&row, "id"), text(&row, "run_id")) else { continue };
        let Some(run) = runs.get_mut(&run) else { continue };
        let worker = match text(&row, "assignee_orca_session_id").and_then(|id| plain_id(&id)) {
            Some(chat) => Worker::Chat(chat),
            None => terminal.get(&id).cloned().map_or(Worker::Unknown, Worker::Session),
        };
        if !run.workers.contains(&worker) || worker == Worker::Unknown {
            run.workers.push(worker);
        }
    }
    Ok(runs.into_values().filter(|run| !run.workers.is_empty()).collect())
}

/// One entry of Orca's own copy of a chat.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    User(String),
    Assistant(String),
    Reasoning(String),
    Tool { call_id: String, name: String, input: Value, output: Option<String>, failed: Option<bool> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub at: Option<String>,
    pub body: Body,
}

/// The text blocks of a message, joined.
fn block_text(body: &Value) -> Option<String> {
    let text: Vec<&str> = body
        .get("blocks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .filter(|text| !text.trim().is_empty())
        .collect();
    (!text.is_empty()).then(|| text.join("\n\n"))
}

/// One journal row's payload as an entry, when it is conversation: a user,
/// assistant or reasoning message with text, or a tool call. A subagent's
/// rows (`agentId`), status, turn and Orca's own cards are not.
pub fn item_of(payload: &Value) -> Option<Item> {
    if payload.get("agentId").is_some_and(|agent| !agent.is_null()) {
        return None;
    }
    let id = json_text(payload, "/itemId")?;
    let at = time_of(payload.get("observedAt").and_then(Value::as_i64));
    let body = payload.get("body")?;
    let body = match body.get("kind").and_then(Value::as_str)? {
        "message" => {
            let text = block_text(body)?;
            match body.get("role").and_then(Value::as_str)? {
                "user" => Body::User(text),
                "assistant" => Body::Assistant(text),
                "reasoning" => Body::Reasoning(text),
                _ => return None,
            }
        }
        "tool-call" => Body::Tool {
            call_id: json_text(body, "/callId").unwrap_or_else(|| id.clone()),
            name: json_text(body, "/name")?,
            input: body.get("input").cloned().unwrap_or(Value::Null),
            output: body.pointer("/output/head").and_then(Value::as_str).map(str::to_owned),
            failed: match body.get("state").and_then(Value::as_str) {
                Some("completed") => Some(false),
                Some("failed" | "error" | "denied" | "cancelled") => Some(true),
                _ => None,
            },
        },
        _ => return None,
    };
    Some(Item { id, at, body })
}

/// Orca's own copy of a chat: its current epoch's visible rows, in order.
pub fn items(conn: &Connection, chat: &str) -> Result<Vec<Item>, String> {
    let what = "chat";
    let epoch = CHATS
        .select_where(conn, "journal_chat_state", &["session_id", "epoch"], Some("session_id = ?1"), &[&chat])
        .map_err(fail(what))?
        .first()
        .and_then(|row| text(row, "epoch"));
    let Some(epoch) = epoch else { return Ok(Vec::new()) };
    let rows = CHATS
        .select_where(
            conn,
            "journal_items",
            &["item_id", "payload_json"],
            Some("session_id = ?1 AND epoch = ?2 AND hidden = 0 ORDER BY sequence, sequence_index, ordinal"),
            &[&chat, &epoch],
        )
        .map_err(fail(what))?;
    let mut seen = BTreeSet::new();
    Ok(rows
        .iter()
        .filter_map(|row| text(row, "payload_json").and_then(|raw| serde_json::from_str::<Value>(&raw).ok()))
        .filter_map(|payload| item_of(&payload))
        .filter(|item| seen.insert(item.id.clone()))
        .collect())
}

/// `scripts.setup` and `scripts.archive` from a repository's `orca.yaml`.
/// Only that block is read, as plain, quoted, literal (`|`) or folded (`>`)
/// values; anything else reads as none. Orca has no run script.
pub fn yaml_scripts(raw: &str) -> Option<Scripts> {
    let lines: Vec<&str> = raw.lines().collect();
    let start = lines.iter().position(|line| line.trim_end() == "scripts:")?;
    let mut scripts = Scripts::default();
    let mut index = start + 1;
    let indent = |line: &str| line.len() - line.trim_start().len();
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            index += 1;
            continue;
        }
        let key_indent = indent(line);
        if key_indent == 0 {
            break;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            index += 1;
            continue;
        };
        let value = value.trim();
        index += 1;
        let text = if let Some(style) = value.chars().next().filter(|first| *first == '|' || *first == '>') {
            let mut block = Vec::new();
            let mut block_indent = None;
            while index < lines.len() {
                let next = lines[index];
                if next.trim().is_empty() {
                    block.push(String::new());
                    index += 1;
                    continue;
                }
                let next_indent = indent(next);
                if next_indent <= key_indent {
                    break;
                }
                let at = *block_indent.get_or_insert(next_indent);
                block.push(next.get(at.min(next.len())..).unwrap_or_default().to_owned());
                index += 1;
            }
            while block.last().is_some_and(String::is_empty) {
                block.pop();
            }
            if style == '>' { block.join(" ") } else { block.join("\n") }
        } else {
            let unquoted = value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .or_else(|| value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')))
                .unwrap_or(value);
            unquoted.to_owned()
        };
        let text = Some(text.trim().to_owned()).filter(|text| !text.is_empty());
        match key.trim() {
            "setup" => scripts.setup = text,
            "archive" => scripts.archive = text,
            _ => {}
        }
    }
    (!scripts.is_empty()).then_some(scripts)
}
