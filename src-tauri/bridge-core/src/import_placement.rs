//! Where imported chats belong, and which ones are not chats at all.
//!
//! - **Not chats.** Bridge runs Claude Code, Codex and OpenCode itself, for
//!   its own work (Slack inbox checks, chat titles, notes, orchestrators and
//!   workers), in folders under its data folder. Those agents save a
//!   transcript of every run, so an import would bring Bridge's own runs back
//!   as chats. A run whose only content is slash commands (`/usage`, with no
//!   reply and no tool use) is a probe, not a conversation. Both are left out
//!   of imports, and [`purge_background`] removes the copies an earlier import
//!   brought.
//! - **Projects.** A chat belongs to the repository it ran in. Its folder is
//!   resolved to the repository's main checkout (worktrees, including Claude
//!   Code's `<repo>/.claude/worktrees/<name>` once deleted, belong to their
//!   repository), and the chat is filed under the Bridge workspace for that
//!   folder, made when Bridge has none ([`place`]). A folder that is not a
//!   repository is its own project. Folders that are gone, temporary,
//!   Bridge's own, the home folder or a hidden folder in it are left alone.
//!
//! No git command runs while Bridge's database is held: folders are read
//! first, resolved without the lock, then written under it.

use crate::store;
use crate::BridgeError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// What decides where chats go: Bridge's own data folders and the home
/// folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    /// Bridge's data folder (and its real path, when it differs).
    pub bridge_dirs: Vec<PathBuf>,
    pub home: PathBuf,
    /// Temporary folders: nothing in them is a project.
    pub temp_dirs: Vec<PathBuf>,
}

/// The OS temporary folders. Unit tests build their repositories there, so
/// they name these themselves when they test them.
fn default_temp_dirs() -> Vec<PathBuf> {
    if cfg!(test) {
        return Vec::new();
    }
    os_temp_dirs()
}

/// `/tmp`, `/var/folders` and this process's temporary folder.
pub fn os_temp_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["/tmp", "/private/tmp", "/var/folders", "/private/var/folders"].iter().map(PathBuf::from).collect();
    let temp = std::env::temp_dir();
    dirs.push(temp.canonicalize().unwrap_or(temp));
    dirs
}

impl Rules {
    pub fn new(data_dir: &Path, home: &Path) -> Self {
        let mut bridge_dirs = vec![data_dir.to_path_buf()];
        if let Ok(real) = data_dir.canonicalize() {
            if real != data_dir {
                bridge_dirs.push(real);
            }
        }
        Self { bridge_dirs, home: home.to_path_buf(), temp_dirs: default_temp_dirs() }
    }

    /// For a running Bridge: its data folder is the database's folder.
    pub fn for_core(core: &crate::runtime::BridgeCore) -> Self {
        let data_dir = core.database_path.parent().map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        Self::new(&data_dir, &home)
    }

    /// A folder inside one of Bridge's data folders: a run Bridge made. This
    /// Bridge's own, or any other Bridge's (a development build, a test run
    /// with its own `--data-dir`), known by its layout: a run's folder is in
    /// `chats/` or `worktrees/` beside the data folder's `bridge.db`.
    pub fn is_bridge_folder(&self, cwd: &str) -> bool {
        let cwd = Path::new(cwd.trim());
        cwd.is_absolute() && (self.bridge_dirs.iter().any(|dir| cwd.starts_with(dir)) || in_a_bridge_data_folder(cwd))
    }

    /// A Claude Code project folder (`projects/<slug>`) for a folder inside
    /// Bridge's data folder. Claude Code names it after the folder, every
    /// character but letters and digits turned into `-`.
    pub fn is_bridge_claude_project(&self, slug: &str) -> bool {
        is_bridge_claude_project(slug, &self.bridge_dirs)
    }
}

/// [`Rules::is_bridge_claude_project`] for named data folders.
pub fn is_bridge_claude_project(slug: &str, bridge_dirs: &[PathBuf]) -> bool {
    bridge_dirs.iter().map(|dir| claude_slug(dir)).any(|prefix| {
        slug == prefix || slug.strip_prefix(&prefix).is_some_and(|rest| rest.starts_with('-'))
    })
}

/// `<data>/chats/...` or `<data>/worktrees/...` where `<data>` holds a
/// `bridge.db`. When `<data>` is gone altogether, Bridge's exact layout is
/// enough: `chats/<chat id>`, `chats/title-runs`, `chats/note-runs`,
/// `worktrees/orchestrators` or `worktrees/workers`.
fn in_a_bridge_data_folder(cwd: &Path) -> bool {
    cwd.ancestors().skip(1).any(|ancestor| {
        let Some(name) = ancestor.file_name().and_then(|name| name.to_str()) else { return false };
        if name != "chats" && name != "worktrees" {
            return false;
        }
        let Some(data) = ancestor.parent() else { return false };
        if data.join("bridge.db").is_file() {
            return true;
        }
        if data.exists() {
            return false;
        }
        let next = cwd.strip_prefix(ancestor).ok().and_then(|rest| rest.components().next()).and_then(|part| part.as_os_str().to_str().map(str::to_owned));
        match (name, next.as_deref()) {
            ("chats", Some(run)) => run == "title-runs" || run == "note-runs" || uuid::Uuid::parse_str(run).is_ok_and(|id| id.hyphenated().to_string() == run),
            ("worktrees", Some(kind)) => kind == "orchestrators" || kind == "workers",
            _ => false,
        }
    })
}

/// Claude Code's project folder name for a folder.
pub fn claude_slug(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// `/usage`, `/model sonnet`: a slash command, not something said.
pub fn is_slash_command(text: &str) -> bool {
    let text = text.trim();
    let Some(rest) = text.strip_prefix('/') else { return false };
    let name = rest.split_whitespace().next().unwrap_or_default();
    !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
        && !text.contains('\n')
}

/// What Claude Code records as a local command's output, in the user's turn.
pub fn is_command_output(text: &str) -> bool {
    let text = text.trim_start();
    ["<local-command-stdout>", "<local-command-stderr>", "<local-command-caveat>"].iter().any(|tag| text.starts_with(tag))
}

/// A run made only of slash commands the person typed and their local
/// output, with no reply, tool call or thinking: a probe such as `/usage`,
/// or `/login` then `/mcp`, not a conversation. `messages` are an import's
/// normalized messages.
pub fn is_command_run(messages: &[Value]) -> bool {
    command_run(messages.iter().map(|message| (message.get("kind").and_then(Value::as_str), message.get("text").and_then(Value::as_str))))
}

/// [`is_command_run`] over (kind, text) pairs.
fn command_run<'a>(entries: impl Iterator<Item = (Option<&'a str>, Option<&'a str>)>) -> bool {
    let mut commands = 0;
    for (kind, text) in entries {
        let Some(text) = text.filter(|_| kind == Some("user.message")) else { return false };
        if is_slash_command(text) {
            commands += 1;
        } else if !is_command_output(text) {
            return false;
        }
    }
    commands > 0
}

/// An import candidate that is a run, not a chat: one Bridge made, or a
/// command probe.
pub fn is_background_candidate(payload: &Value, rules: &Rules) -> bool {
    let cwd = payload.get("cwd").and_then(Value::as_str).unwrap_or_default();
    let messages = payload.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    rules.is_bridge_folder(cwd) || is_command_run(messages)
}

/// A folder Bridge never makes a project of: the home folder or anything
/// above it, a hidden folder in it (`~/.claude`, `~/.cache`), a temporary
/// folder or one of Bridge's own.
fn is_unsuitable(root: &Path, rules: &Rules) -> bool {
    if rules.temp_dirs.iter().any(|dir| root.starts_with(dir)) || rules.is_bridge_folder(&root.to_string_lossy()) || rules.home.starts_with(root) {
        return true;
    }
    match root.strip_prefix(&rules.home) {
        Ok(inside) => inside
            .components()
            .next()
            .is_some_and(|first| matches!(first, Component::Normal(name) if name.to_string_lossy().starts_with('.'))),
        Err(_) => false,
    }
}

/// The folder a chat that ran in `cwd` belongs to: its repository's main
/// checkout, else the folder itself. A Claude Code worktree whose folder is
/// gone belongs to the repository it was made in. Runs git; call it without
/// Bridge's database held.
pub fn project_folder(cwd: &str, rules: &Rules) -> Option<PathBuf> {
    let cwd = cwd.trim();
    if cwd.is_empty() || !Path::new(cwd).is_absolute() || rules.is_bridge_folder(cwd) {
        return None;
    }
    let mut folder = PathBuf::from(cwd);
    if !folder.is_dir() {
        let (repo, _) = cwd.split_once("/.claude/worktrees/")?;
        folder = PathBuf::from(repo);
        if !folder.is_dir() {
            return None;
        }
    }
    let folder = folder.canonicalize().unwrap_or(folder);
    let root = crate::git::main_worktree_root(&folder).map(|root| root.canonicalize().unwrap_or(root)).unwrap_or(folder);
    (!is_unsuitable(&root, rules)).then_some(root)
}

/// What [`place`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// Chats now filed under a project.
    pub placed: usize,
    /// Projects Bridge made for them.
    pub created: Vec<PathBuf>,
}

/// File every imported chat that has no project under the workspace for the
/// folder it ran in, making the workspace (and its project, for a
/// repository) when Bridge has none. Chats Bridge cannot place stay where
/// they are.
pub fn place(db: &Mutex<Connection>, rules: &Rules) -> Result<Placement, BridgeError> {
    let (folders, workspaces) = {
        let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut statement = db.prepare(
            "SELECT DISTINCT cwd FROM sessions WHERE kind='imported' AND workspace_id IS NULL AND cwd IS NOT NULL AND trim(cwd) <> ''",
        )?;
        let folders: Vec<String> = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
        let mut statement = db.prepare("SELECT id, path FROM workspaces WHERE path IS NOT NULL AND trim(path) <> '' ORDER BY created_at, id")?;
        let mut workspaces: HashMap<PathBuf, String> = HashMap::new();
        for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? {
            let (id, path) = row?;
            workspaces.entry(PathBuf::from(path)).or_insert(id);
        }
        (folders, workspaces)
    };
    // Without the lock: a workspace at the chat's own folder wins (an adopted
    // worktree, say); else the folder's project.
    let mut plan: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    for cwd in folders {
        let own = PathBuf::from(cwd.trim());
        let target = if workspaces.contains_key(&own) {
            Some(own)
        } else {
            let real = own.canonicalize().ok();
            match real.filter(|real| workspaces.contains_key(real)) {
                Some(real) => Some(real),
                None => project_folder(&cwd, rules),
            }
        };
        if let Some(target) = target {
            plan.entry(target).or_default().push(cwd);
        }
    }
    let branches: HashMap<PathBuf, Option<String>> = plan
        .keys()
        .filter(|folder| !workspaces.contains_key(*folder))
        .map(|folder| (folder.clone(), crate::git::current_branch(folder)))
        .collect();
    let is_repo: HashMap<PathBuf, bool> = branches.keys().map(|folder| (folder.clone(), crate::git::main_worktree_root(folder).is_some())).collect();

    let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let transaction = db.unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();
    let mut out = Placement::default();
    for (folder, cwds) in plan {
        let path = folder.to_string_lossy().into_owned();
        let existing: Option<String> = transaction
            .query_row("SELECT id FROM workspaces WHERE path=?1 ORDER BY created_at, id LIMIT 1", params![path], |row| row.get(0))
            .optional()?;
        let workspace = match existing {
            Some(id) => id,
            None => {
                let name = folder.file_name().map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
                let project_id: Option<String> = if is_repo.get(&folder).copied().unwrap_or(false) {
                    transaction.execute(
                        "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
                        params![uuid::Uuid::new_v4().to_string(), name, path, now],
                    )?;
                    transaction.query_row("SELECT id FROM projects WHERE path=?1", params![path], |row| row.get(0)).optional()?
                } else {
                    None
                };
                let id = uuid::Uuid::new_v4().to_string();
                transaction.execute(
                    "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,'idle',?7)",
                    params![id, project_id, name, name, branches.get(&folder).cloned().flatten(), path, now],
                )?;
                store::event(&transaction, "import", "workspace.created", &id, &format!("Made a project for imported chats in {name}"))?;
                out.created.push(folder.clone());
                id
            }
        };
        for cwd in cwds {
            out.placed += transaction.execute(
                "UPDATE sessions SET workspace_id=?1 WHERE kind='imported' AND workspace_id IS NULL AND cwd=?2",
                params![workspace, cwd],
            )?;
        }
    }
    transaction.commit()?;
    Ok(out)
}

/// Sessions removed per transaction, so Bridge's database is never held for
/// long.
const PURGE_BATCH: usize = 500;

/// Remove imported copies of runs that are not chats (Bridge's own runs and
/// command probes), with their subagents and history. Imported history is
/// otherwise immutable: only these are ever deleted, the guard on their
/// entries is lifted inside each batch's transaction and put back before it
/// commits, and their provenance records stay as the audit of what was
/// imported. Returns how many chats were removed.
pub fn purge_background(db: &Mutex<Connection>, rules: &Rules) -> Result<usize, BridgeError> {
    let doomed = {
        let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        background_sessions(&db, rules)?
    };
    let mut removed = 0;
    for batch in doomed.chunks(PURGE_BATCH) {
        let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let transaction = db.unchecked_transaction()?;
        // Foreign keys are checked when the batch commits, so the order rows
        // go in does not matter, only that nothing is left pointing at them.
        transaction.execute_batch(
            "PRAGMA defer_foreign_keys = ON;
             CREATE TEMP TABLE IF NOT EXISTS import_purge(id TEXT PRIMARY KEY);
             DELETE FROM temp.import_purge;",
        )?;
        for id in batch {
            transaction.execute("INSERT OR IGNORE INTO temp.import_purge(id) VALUES(?1)", params![id])?;
        }
        // Their subagents go with them, however deep.
        transaction.execute_batch(
            "WITH RECURSIVE tree(id) AS (
                 SELECT id FROM temp.import_purge
                 UNION SELECT s.id FROM sessions s JOIN tree ON s.parent_session_id = tree.id WHERE s.kind='imported'
             )
             INSERT OR IGNORE INTO temp.import_purge(id) SELECT id FROM tree;
             DROP TRIGGER IF EXISTS imported_session_entries_immutable_delete;",
        )?;
        // Rows that cannot outlive their session or its history: every table
        // whose foreign key to `sessions` or `session_entries` does not
        // cascade (history, heads, events).
        for (table, column) in dependents_of(&transaction, "session_entries")? {
            transaction.execute(
                &format!(
                    "DELETE FROM \"{table}\" WHERE \"{column}\" IN
                     (SELECT id FROM session_entries WHERE session_id IN (SELECT id FROM temp.import_purge))"
                ),
                [],
            )?;
        }
        for (table, column) in dependents_of(&transaction, "sessions")? {
            transaction.execute(
                &format!("DELETE FROM \"{table}\" WHERE \"{column}\" IN (SELECT id FROM temp.import_purge)"),
                [],
            )?;
        }
        transaction.execute("DELETE FROM import_native_sessions WHERE session_id IN (SELECT id FROM temp.import_purge)", [])?;
        removed += transaction.query_row(
            "SELECT COUNT(*) FROM sessions WHERE kind='imported' AND parent_session_id IS NULL AND id IN (SELECT id FROM temp.import_purge)",
            [],
            |row| row.get::<_, i64>(0),
        )? as usize;
        // Parents and subagents in one statement, so the parent link holds.
        transaction.execute("DELETE FROM sessions WHERE kind='imported' AND id IN (SELECT id FROM temp.import_purge)", [])?;
        crate::external_import::install_import_foundation(&transaction)?;
        transaction.execute_batch("DELETE FROM temp.import_purge;")?;
        transaction.commit()?;
    }
    Ok(removed)
}

/// Tables with a foreign key to `parent` that does not cascade, and the
/// column: their rows cannot outlive the row they point at. The parent's own
/// self-references are left to the delete itself.
fn dependents_of(db: &Connection, parent: &str) -> Result<Vec<(String, String)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT m.name, f.\"from\" FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) f
         WHERE m.type='table' AND m.name <> ?1 AND f.\"table\"=?1
           AND upper(f.on_delete) NOT IN ('CASCADE','SET NULL','SET DEFAULT')
         ORDER BY m.name",
    )?;
    let rows = statement.query_map(params![parent], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Top-level imported sessions that are runs, not chats.
fn background_sessions(db: &Connection, rules: &Rules) -> Result<Vec<String>, BridgeError> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut statement = db.prepare("SELECT id, cwd FROM sessions WHERE kind='imported' AND parent_session_id IS NULL AND cwd IS NOT NULL")?;
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? {
        let (id, cwd) = row?;
        if rules.is_bridge_folder(&cwd) && seen.insert(id.clone()) {
            out.push(id);
        }
    }
    // Probes: every entry a user message, and every one a slash command.
    let mut statement = db.prepare(
        "SELECT s.id FROM sessions s
         WHERE s.kind='imported' AND s.parent_session_id IS NULL
           AND EXISTS (SELECT 1 FROM session_entries e WHERE e.session_id=s.id)
           AND NOT EXISTS (SELECT 1 FROM session_entries e WHERE e.session_id=s.id AND e.kind <> 'user.message')",
    )?;
    let only_user: Vec<String> = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
    let mut texts = db.prepare("SELECT json_extract(payload,'$.text') FROM session_entries WHERE session_id=?1")?;
    for id in only_user {
        let all: Vec<Option<String>> = texts.query_map(params![id], |row| row.get(0))?.collect::<Result<_, _>>()?;
        if command_run(all.iter().map(|text| (Some("user.message"), text.as_deref()))) && seen.insert(id.clone()) {
            out.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "import_placement/tests.rs"]
mod tests;
