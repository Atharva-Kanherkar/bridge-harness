//! Superset's databases, read-only: each organization's projects,
//! workspaces, terminal agent sessions and agent configurations
//! (`host.db`), and its chats with Superset's own copy of them (`chat.db`).
//!
//! Both are drizzle databases: the version is the newest migration's
//! journal time in `__drizzle_migrations.created_at`.

use crate::import_sqlite::{int, text, Allowed, Row};
use crate::import_worktrees::Scripts;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// The host database migrations Bridge reads: 0000 to 0038
/// (`superset-sh/superset@17bbe650`).
pub const HOST_OLDEST: i64 = 1_773_188_543_980;
pub const HOST_NEWEST: i64 = 1_792_467_855_578;
/// The chat database migrations Bridge reads: 0000 to 0005.
pub const CHAT_OLDEST: i64 = 1_785_962_456_515;
pub const CHAT_NEWEST: i64 = 1_794_194_559_956;

pub const HOST_DB: &str = "host.db";
pub const CHAT_DB: &str = "chat.db";

/// The only tables Bridge reads from each database. `host_agent_configs`
/// is read without `env_json`, whose key names alone SQLite lists.
pub const HOST_TABLES: &[&str] = &[
    "__drizzle_migrations",
    "projects",
    "workspaces",
    "terminal_agent_bindings",
    "workspace_tags",
    "workspace_pull_requests",
    "host_settings",
    "host_agent_configs",
];
pub const CHAT_TABLES: &[&str] = &["__drizzle_migrations", "chat_sessions_local", "chat_journal"];

/// Files in Superset's app home (or an organization's folder) that hold
/// sign-ins, tokens or tickets. Never opened.
pub const DENIED_FILES: &[&str] = &[
    "auth-token.enc",
    "config.json",
    "manifest.json",
    "terminal-host.token",
    "cloud-workspace-tickets.json",
    "plugin-mcp-sessions.json",
    "state",
    "agent",
    "local.db",
];

const HOST: Allowed = Allowed { tool: "Superset", tables: HOST_TABLES };
const CHATS: Allowed = Allowed { tool: "Superset", tables: CHAT_TABLES };

/// The agent session a chat or terminal ran as: `claude`, `codex` or
/// `opencode`, and that agent's own id for it.
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

/// The harness Bridge runs for a chat's `harness` (ACP or the older
/// adapters). Pi, pdev and Gemini are agents Bridge does not run.
pub fn harness_of_chat(harness: &str) -> Option<&'static str> {
    match harness.trim() {
        "claude-acp" | "claude-code" => Some("claude"),
        "codex-acp" | "codex" => Some("codex"),
        "opencode-acp" => Some("opencode"),
        _ => None,
    }
}

/// The harness Bridge runs for a terminal agent's `agent_id`.
pub fn harness_of_agent(agent: &str) -> Option<&'static str> {
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

/// Superset's epoch milliseconds as RFC 3339.
pub fn time_of(ms: Option<i64>) -> Option<String> {
    ms.filter(|ms| *ms > 0).and_then(DateTime::<Utc>::from_timestamp_millis).map(|at| at.to_rfc3339())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub root: PathBuf,
    pub name: Option<String>,
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Its own git worktree.
    Worktree,
    /// The project's own checkout.
    Local,
    /// A project-less scratch folder.
    Session,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub project: Option<String>,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub name: Option<String>,
    pub kind: Kind,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub id: String,
    /// The workspace it ran in (`scope_id`).
    pub workspace: String,
    /// Superset's harness id (`opencode-acp`, `claude-code`, ...).
    pub harness: String,
    pub native: Option<NativeSession>,
    pub epoch: Option<String>,
    pub title: Option<String>,
    pub updated_at: Option<String>,
}

/// A terminal an agent ran in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub terminal: String,
    pub workspace: String,
    pub agent: String,
    pub native: Option<NativeSession>,
    /// Claude Code's transcript, as its hook reported it.
    pub transcript: Option<PathBuf>,
}

/// An agent configuration, without its environment's values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub id: String,
    pub preset: Option<String>,
    pub label: String,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env_names: Vec<String>,
}

/// One organization's host database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Host {
    pub version: i64,
    pub projects: BTreeMap<String, Project>,
    pub workspaces: BTreeMap<String, Workspace>,
    pub bindings: Vec<Binding>,
    pub configs: Vec<AgentConfig>,
    pub tags: usize,
    pub pull_requests: usize,
    /// Superset's default Claude Code folder and Codex home, when set.
    pub default_claude: Option<PathBuf>,
    pub default_codex: Option<PathBuf>,
}

fn fail(what: &str) -> impl Fn(rusqlite::Error) -> String + '_ {
    move |error| format!("Bridge could not read Superset's {what}: {error}")
}

fn has_all(allowed: &Allowed, conn: &Connection, table: &str, needed: &[&str], what: &str) -> Result<bool, String> {
    let present = allowed.columns(conn, table).map_err(fail(what))?;
    Ok(needed.iter().all(|column| present.contains(*column)))
}

/// The newest migration a drizzle database applied, gated to `range`.
fn version(allowed: &Allowed, conn: &Connection, what: &str, oldest: i64, newest: i64) -> Result<i64, String> {
    let version = allowed
        .select(conn, "__drizzle_migrations", &["created_at"])
        .map_err(fail(what))?
        .iter()
        .filter_map(|row| int(row, "created_at"))
        .max()
        .ok_or_else(|| format!("Superset's {what} has no version Bridge can read"))?;
    if version > newest {
        return Err(format!("Superset's {what} is newer than Bridge knows (version {version}); update Bridge and try again"));
    }
    if version < oldest {
        return Err(format!("Superset's {what} is older than Bridge knows (version {version})"));
    }
    Ok(version)
}

fn json(row: &Row, column: &str) -> Value {
    text(row, column).and_then(|raw| serde_json::from_str(&raw).ok()).unwrap_or(Value::Null)
}

fn absolute(path: Option<String>) -> Option<PathBuf> {
    path.map(PathBuf::from).filter(|path| path.is_absolute())
}

/// Read one organization's host database, or say why not.
pub fn read_host(conn: &Connection) -> Result<Host, String> {
    let what = "workspaces";
    let version = version(&HOST, conn, what, HOST_OLDEST, HOST_NEWEST)?;
    if !has_all(&HOST, conn, "projects", &["id", "repo_path"], what)?
        || !has_all(&HOST, conn, "workspaces", &["id", "worktree_path"], what)?
    {
        return Err(format!("Superset's workspaces are missing what Bridge reads (version {version})"));
    }
    let mut host = Host { version, ..Host::default() };
    for row in HOST.select(conn, "projects", &["id", "repo_path", "name", "deleted_at"]).map_err(fail(what))? {
        let (Some(id), Some(root)) = (text(&row, "id").and_then(|id| plain_id(&id)), absolute(text(&row, "repo_path"))) else { continue };
        host.projects.insert(id.clone(), Project { id, root, name: text(&row, "name"), deleted: text(&row, "deleted_at").is_some() });
    }
    let wanted = ["id", "project_id", "worktree_path", "branch", "name", "type", "archived_at"];
    for row in HOST.select(conn, "workspaces", &wanted).map_err(fail(what))? {
        let (Some(id), Some(path)) = (text(&row, "id").and_then(|id| plain_id(&id)), absolute(text(&row, "worktree_path"))) else { continue };
        let kind = match text(&row, "type").as_deref() {
            None | Some("worktree") => Kind::Worktree,
            Some("local" | "main") => Kind::Local,
            Some("session") => Kind::Session,
            Some(_) => Kind::Other,
        };
        host.workspaces.insert(
            id.clone(),
            Workspace {
                project: text(&row, "project_id"),
                branch: text(&row, "branch"),
                name: text(&row, "name"),
                archived: text(&row, "archived_at").is_some(),
                id,
                path,
                kind,
            },
        );
    }
    let wanted = ["terminal_id", "workspace_id", "agent_id", "agent_session_id", "transcript_path"];
    for row in HOST.select(conn, "terminal_agent_bindings", &wanted).map_err(fail(what))? {
        let (Some(terminal), Some(workspace), Some(agent)) = (text(&row, "terminal_id"), text(&row, "workspace_id"), text(&row, "agent_id")) else { continue };
        let native = harness_of_agent(&agent)
            .zip(text(&row, "agent_session_id").and_then(|id| plain_id(&id)))
            .map(|(harness, id)| NativeSession { harness, id });
        host.bindings.push(Binding { terminal, workspace, agent, native, transcript: absolute(text(&row, "transcript_path")) });
    }
    let wanted = ["id", "preset_id", "label", "command", "args_json"];
    for row in HOST.select_json_keys(conn, "host_agent_configs", &wanted, "env_json", "env_names").map_err(fail(what))? {
        let (Some(id), Some(label)) = (text(&row, "id"), text(&row, "label")) else { continue };
        let strings = |value: Value| -> Vec<String> {
            value.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
        };
        host.configs.push(AgentConfig {
            preset: text(&row, "preset_id"),
            command: text(&row, "command"),
            args: strings(json(&row, "args_json")),
            env_names: strings(json(&row, "env_names")),
            label,
            id,
        });
    }
    host.tags = HOST.select(conn, "workspace_tags", &["workspace_id"]).map_err(fail(what))?.len();
    host.pull_requests = HOST.select(conn, "workspace_pull_requests", &["workspace_id"]).map_err(fail(what))?.len();
    if let Some(row) = HOST.select(conn, "host_settings", &["default_claude_config_dir", "default_codex_home"]).map_err(fail(what))?.first() {
        host.default_claude = absolute(text(row, "default_claude_config_dir"));
        host.default_codex = absolute(text(row, "default_codex_home"));
    }
    Ok(host)
}

/// Read one organization's chats, or say why not.
pub fn read_chats(conn: &Connection) -> Result<(i64, Vec<Chat>), String> {
    let what = "chats";
    let version = version(&CHATS, conn, what, CHAT_OLDEST, CHAT_NEWEST)?;
    if !has_all(&CHATS, conn, "chat_sessions_local", &["session_id", "scope_id", "harness"], what)? {
        return Err(format!("Superset's chats are missing what Bridge reads (version {version})"));
    }
    let wanted = ["session_id", "scope_id", "harness", "harness_session_id", "epoch", "title", "updated_at"];
    let mut out = Vec::new();
    for row in CHATS.select_where(conn, "chat_sessions_local", &wanted, Some("1 ORDER BY session_id"), &[]).map_err(fail(what))? {
        let (Some(id), Some(workspace), Some(harness)) = (text(&row, "session_id").and_then(|id| plain_id(&id)), text(&row, "scope_id"), text(&row, "harness")) else { continue };
        let native = harness_of_chat(&harness)
            .zip(text(&row, "harness_session_id").and_then(|id| plain_id(&id)))
            .map(|(harness, id)| NativeSession { harness, id });
        out.push(Chat {
            native,
            epoch: text(&row, "epoch"),
            title: text(&row, "title").map(|title| title.trim().to_owned()),
            updated_at: time_of(int(&row, "updated_at")),
            id,
            workspace,
            harness,
        });
    }
    Ok((version, out))
}

/// One entry of Superset's own copy of a chat.
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

/// The person's words in a message part: `<superset-context>` blocks are
/// what Superset added around them.
fn without_context(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("<superset-context>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</superset-context>") {
            Some(end) => rest = &rest[start + end + "</superset-context>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_owned()
}

/// One journaled item as an entry, when it is conversation of this chat's
/// own: the person's message, the agent's, reasoning, or a tool call.
/// Subagent items (`parentItemId`), plans, approvals and notices are not.
pub fn item_of(item: &Value) -> Option<Item> {
    if item.get("parentItemId").is_some_and(|parent| !parent.is_null()) {
        return None;
    }
    let id = item.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?.to_owned();
    let at = time_of(item.get("startedAtMs").and_then(Value::as_i64));
    let text_of = |key: &str| item.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_owned);
    let body = match item.get("kind").and_then(Value::as_str)? {
        "user_message" => {
            if item.get("discarded").and_then(Value::as_bool) == Some(true) {
                return None;
            }
            let parts: Vec<String> = item
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .filter(|part| part.get("hidden").and_then(Value::as_bool) != Some(true))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .map(without_context)
                .filter(|text| !text.is_empty())
                .collect();
            if parts.is_empty() {
                return None;
            }
            Body::User(parts.join("\n\n"))
        }
        "agent_message" => Body::Assistant(text_of("text")?),
        "reasoning" => Body::Reasoning(text_of("text").or_else(|| text_of("summary"))?),
        "tool_call" => {
            let output: Vec<String> = item
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                    Some("text") => part.get("text").and_then(Value::as_str).map(str::to_owned),
                    Some("terminal") => part.get("output").and_then(Value::as_str).map(str::to_owned),
                    Some("diff") => part.get("path").and_then(Value::as_str).map(|path| format!("Edited {path}")),
                    _ => None,
                })
                .filter(|text| !text.trim().is_empty())
                .collect();
            let raw_output = item.get("rawOutput").filter(|value| !value.is_null()).map(|value| match value.as_str() {
                Some(text) => text.to_owned(),
                None => value.to_string(),
            });
            Body::Tool {
                call_id: id.clone(),
                name: text_of("toolName").or_else(|| text_of("title")).or_else(|| text_of("toolKind"))?,
                input: item.get("rawInput").cloned().unwrap_or(Value::Null),
                output: if output.is_empty() { raw_output } else { Some(output.join("\n")) },
                failed: match item.get("status").and_then(Value::as_str) {
                    Some("completed") => Some(false),
                    Some("failed" | "error" | "rejected" | "cancelled") => Some(true),
                    _ => None,
                },
            }
        }
        _ => return None,
    };
    Some(Item { id, at, body })
}

/// Superset's own copy of a chat: its epoch's items, each item's latest
/// body at the position it first appeared.
pub fn items(conn: &Connection, chat: &str, epoch: &str) -> Result<Vec<Item>, String> {
    let rows = CHATS
        .select_where(conn, "chat_journal", &["seq", "event_json"], Some("session_id = ?1 AND epoch = ?2 ORDER BY seq"), &[&chat, &epoch])
        .map_err(fail("chat"))?;
    let mut order: Vec<String> = Vec::new();
    let mut latest: HashMap<String, Value> = HashMap::new();
    for row in rows {
        let event = json(&row, "event_json");
        if event.get("type").and_then(Value::as_str) != Some("item") {
            continue;
        }
        let Some(item) = event.get("item").filter(|item| item.is_object()) else { continue };
        let Some(id) = item.get("id").and_then(Value::as_str).map(str::to_owned) else { continue };
        if !latest.contains_key(&id) {
            order.push(id.clone());
        }
        latest.insert(id, item.clone());
    }
    Ok(order.iter().filter_map(|id| latest.get(id)).filter_map(item_of).collect())
}

/// Setup, run and archive scripts from a repository's project config:
/// `.paradev/` (or the older `.superset/`), `config.toml` over `config.json`.
pub fn project_scripts(repo: &Path) -> Option<Scripts> {
    let dir = [".paradev", ".superset"].iter().map(|name| repo.join(name)).find(|dir| dir.join("config.toml").is_file() || dir.join("config.json").is_file())?;
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok().filter(|raw| raw.len() <= 256 * 1024);
    read("config.toml").and_then(|raw| toml_scripts(&raw)).or_else(|| read("config.json").and_then(|raw| json_scripts(&raw)))
}

fn joined(value: Option<&Value>) -> Option<String> {
    let lines: Vec<&str> = match value? {
        Value::String(text) => vec![text.as_str()],
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let lines: Vec<&str> = lines.into_iter().map(str::trim).filter(|line| !line.is_empty()).collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// `config.json`: `setup`, `teardown` and `run`, each commands.
pub fn json_scripts(raw: &str) -> Option<Scripts> {
    let config: Value = serde_json::from_str(raw).ok()?;
    let scripts = Scripts {
        setup: joined(config.get("setup")),
        run: joined(config.get("run")),
        run_mode: None,
        archive: joined(config.get("teardown")),
    };
    (!scripts.is_empty()).then_some(scripts)
}

/// `config.toml`: `[install] run` then `[setup] run` as setup, `[archive]
/// run`, and the default terminal's `run` as the run script.
pub fn toml_scripts(raw: &str) -> Option<Scripts> {
    let config: toml::Value = toml::from_str(raw).ok()?;
    let config = serde_json::to_value(config).ok()?;
    let run = config.get("terminals").and_then(Value::as_object).and_then(|terminals| {
        let mut named: Vec<(&String, &Value)> = terminals.iter().collect();
        named.sort_by_key(|(name, terminal)| (terminal.get("default").and_then(Value::as_bool) != Some(true), name.as_str() != "dev", (*name).clone()));
        named.first().and_then(|(_, terminal)| joined(terminal.get("run")))
    });
    let setup: Vec<String> = [joined(config.pointer("/install/run")), joined(config.pointer("/setup/run"))].into_iter().flatten().collect();
    let scripts = Scripts {
        setup: (!setup.is_empty()).then(|| setup.join("\n")),
        run,
        run_mode: None,
        archive: joined(config.pointer("/archive/run")),
    };
    (!scripts.is_empty()).then_some(scripts)
}
