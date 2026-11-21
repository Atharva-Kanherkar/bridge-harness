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

/// Sessions removed per transaction. Each batch holds Bridge's database for
/// a moment, then lets go for [`PURGE_PAUSE`] so other work gets it.
const PURGE_BATCH: usize = 200;
const PURGE_PAUSE: std::time::Duration = std::time::Duration::from_millis(50);

/// Triggers on `session_entries` lifted inside a purge batch and put back
/// before it commits: the guard on imported history, and the search index's
/// per-entry delete, which the batch does once for all its entries instead
/// (`entry_id` is not indexed in the search table, so each per-entry delete
/// reads the whole index).
const LIFTED_TRIGGERS: &[&str] = &["imported_session_entries_immutable_delete", "session_entries_ad_fts"];

/// Run `work` with Bridge's database held, and keep the longest hold.
fn held<T>(db: &Mutex<Connection>, longest: &mut std::time::Duration, work: impl FnOnce(&Connection) -> T) -> T {
    let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let started = std::time::Instant::now();
    let out = work(&db);
    *longest = (*longest).max(started.elapsed());
    out
}

/// What [`purge_background`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Purge {
    /// Chats removed (subagents not counted).
    pub removed: usize,
    pub batches: usize,
    /// The longest any step held Bridge's database.
    pub longest_hold: std::time::Duration,
}

/// Remove imported copies of runs that are not chats (Bridge's own runs and
/// command probes), with their subagents and history. Imported history is
/// otherwise immutable: only these are ever deleted, the guard on their
/// entries is lifted inside each batch's transaction and put back before it
/// commits, and their provenance records stay as the audit of what was
/// imported.
///
/// Fast, so it never stalls Bridge: a batch removes a run's own rows
/// (session, history, head, native id, search rows) with foreign key checks
/// off, because with them on, every deleted entry makes SQLite read the whole
/// history table (`parent_entry_id` has no index); on a 1.5 million entry
/// history that was a second per entry. A batch whose sessions anything else
/// points at is removed with the checks on instead.
pub fn purge_background(db: &Mutex<Connection>, rules: &Rules) -> Result<Purge, BridgeError> {
    let mut out = Purge::default();
    let doomed = background_sessions(db, rules, &mut out.longest_hold)?;
    if doomed.is_empty() {
        return Ok(out);
    }
    // Found once, up front, in short locked steps, so a batch only deletes
    // by key: each run with its subagents, their search rows, their native
    // ids.
    let families = held(db, &mut out.longest_hold, |db| families_of(db, &doomed))?;
    let search = search_rows_of(db, &families, &mut out.longest_hold)?;
    let natives = held(db, &mut out.longest_hold, |db| native_keys_of(db, &families))?;
    for (index, batch) in doomed.chunks(PURGE_BATCH).enumerate() {
        if index > 0 {
            std::thread::sleep(PURGE_PAUSE);
        }
        let ids: Vec<&str> = batch.iter().flat_map(|root| families.get(root).into_iter().flatten()).map(String::as_str).collect();
        let rows: Vec<i64> = ids.iter().flat_map(|id| search.get(*id).into_iter().flatten()).copied().collect();
        let keys: Vec<&str> = ids.iter().flat_map(|id| natives.get(*id).into_iter().flatten()).map(String::as_str).collect();
        out.removed += held(db, &mut out.longest_hold, |db| purge_batch(db, &ids, &rows, &keys))?;
        out.batches += 1;
    }
    Ok(out)
}

/// Each doomed run and its imported subagents, however deep, by run.
fn families_of(db: &Connection, doomed: &[String]) -> Result<HashMap<String, Vec<String>>, BridgeError> {
    db.execute_batch("CREATE TEMP TABLE IF NOT EXISTS import_doomed(id TEXT PRIMARY KEY); DELETE FROM temp.import_doomed;")?;
    {
        let mut insert = db.prepare("INSERT OR IGNORE INTO temp.import_doomed(id) VALUES(?1)")?;
        for id in doomed {
            insert.execute(params![id])?;
        }
    }
    let mut statement = db.prepare(
        "WITH RECURSIVE tree(root, id) AS (
             SELECT id, id FROM temp.import_doomed
             UNION SELECT tree.root, s.id FROM sessions s JOIN tree ON s.parent_session_id = tree.id WHERE s.kind='imported'
         )
         SELECT root, id FROM tree",
    )?;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? {
        let (root, id) = row?;
        out.entry(root).or_default().push(id);
    }
    drop(statement);
    db.execute_batch("DELETE FROM temp.import_doomed;")?;
    Ok(out)
}

/// Rows of the search index read per locked step: the index can only be
/// filtered by session by reading it, so it is read once, in steps.
const SEARCH_STEP: i64 = 20_000;

/// The search index's rows for the doomed sessions, by session.
fn search_rows_of(
    db: &Mutex<Connection>,
    families: &HashMap<String, Vec<String>>,
    longest: &mut std::time::Duration,
) -> Result<HashMap<String, Vec<i64>>, BridgeError> {
    let wanted: std::collections::HashSet<&str> = families.values().flatten().map(String::as_str).collect();
    let mut out: HashMap<String, Vec<i64>> = HashMap::new();
    let last: Option<i64> = held(db, longest, |db| -> Result<Option<i64>, BridgeError> {
        let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='session_entry_fts')", [], |row| row.get(0))?;
        if !exists {
            return Ok(None);
        }
        Ok(db.query_row("SELECT max(rowid) FROM session_entry_fts", [], |row| row.get(0))?)
    })?;
    let Some(last) = last else { return Ok(out) };
    let mut from = 0;
    while from < last {
        held(db, longest, |db| -> Result<(), BridgeError> {
            let mut statement = db.prepare_cached("SELECT rowid, session_id FROM session_entry_fts WHERE rowid > ?1 AND rowid <= ?2")?;
            for row in statement.query_map(params![from, from + SEARCH_STEP], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?)))? {
                let (rowid, session) = row?;
                if let Some(session) = session.filter(|session| wanted.contains(session.as_str())) {
                    out.entry(session).or_default().push(rowid);
                }
            }
            Ok(())
        })?;
        from += SEARCH_STEP;
    }
    Ok(out)
}

/// The native ids recorded for the doomed sessions, by session.
fn native_keys_of(db: &Connection, families: &HashMap<String, Vec<String>>) -> Result<HashMap<String, Vec<String>>, BridgeError> {
    let wanted: std::collections::HashSet<&str> = families.values().flatten().map(String::as_str).collect();
    let mut statement = db.prepare("SELECT native_key, session_id FROM import_native_sessions")?;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for row in statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))? {
        let (key, session) = row?;
        if wanted.contains(session.as_str()) {
            out.entry(session).or_default().push(key);
        }
    }
    Ok(out)
}

/// Remove one batch (runs with their subagents, their search rows and native
/// ids), with foreign key checks off when nothing but the runs' own rows
/// refers to them, else with them on.
fn purge_batch(db: &Connection, ids: &[&str], search_rows: &[i64], native_keys: &[&str]) -> Result<usize, BridgeError> {
    db.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS import_purge(id TEXT PRIMARY KEY);
         CREATE TEMP TABLE IF NOT EXISTS import_purge_entries(id TEXT PRIMARY KEY);
         CREATE TEMP TABLE IF NOT EXISTS import_purge_search(row INTEGER PRIMARY KEY);
         CREATE TEMP TABLE IF NOT EXISTS import_purge_natives(key TEXT PRIMARY KEY);
         DELETE FROM temp.import_purge; DELETE FROM temp.import_purge_entries;
         DELETE FROM temp.import_purge_search; DELETE FROM temp.import_purge_natives;",
    )?;
    {
        let mut insert = db.prepare("INSERT OR IGNORE INTO temp.import_purge(id) VALUES(?1)")?;
        for id in ids {
            insert.execute(params![id])?;
        }
        let mut insert = db.prepare("INSERT OR IGNORE INTO temp.import_purge_search(row) VALUES(?1)")?;
        for row in search_rows {
            insert.execute(params![row])?;
        }
        let mut insert = db.prepare("INSERT OR IGNORE INTO temp.import_purge_natives(key) VALUES(?1)")?;
        for key in native_keys {
            insert.execute(params![key])?;
        }
    }
    db.execute_batch(
        "INSERT OR IGNORE INTO temp.import_purge_entries(id)
             SELECT id FROM session_entries WHERE session_id IN (SELECT id FROM temp.import_purge);",
    )?;
    let fast = !referenced_elsewhere(db)?;
    if fast {
        db.execute_batch("PRAGMA foreign_keys = OFF;")?;
    }
    let removed = remove_batch(db, fast);
    if fast {
        db.execute_batch("PRAGMA foreign_keys = ON;")?;
    }
    db.execute_batch(
        "DELETE FROM temp.import_purge; DELETE FROM temp.import_purge_entries;
         DELETE FROM temp.import_purge_search; DELETE FROM temp.import_purge_natives;",
    )?;
    removed
}

/// Whether any row besides a run's own (its session, history and head, and
/// subagents in the batch) points at the batch: then foreign key checks
/// must stay on to carry out each reference's action.
fn referenced_elsewhere(db: &Connection) -> Result<bool, BridgeError> {
    for (parent, keys) in [("sessions", "temp.import_purge"), ("session_entries", "temp.import_purge_entries")] {
        for (table, column) in references_to(db, parent)? {
            if table == "session_entries" || table == "session_heads" || table == "sessions" {
                continue;
            }
            let found: bool = db.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM \"{table}\" WHERE \"{column}\" IN (SELECT id FROM {keys}))"),
                [],
                |row| row.get(0),
            )?;
            if found {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Delete the batch in one transaction. `fast`: foreign key checks are off,
/// so every reference to the batch is removed here.
fn remove_batch(db: &Connection, fast: bool) -> Result<usize, BridgeError> {
    let transaction = db.unchecked_transaction()?;
    let lifted: Vec<String> = {
        let mut statement = transaction.prepare("SELECT name, sql FROM sqlite_master WHERE type='trigger' AND tbl_name='session_entries'")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|(name, _)| LIFTED_TRIGGERS.contains(&name.as_str()))
            .filter_map(|(name, sql)| sql.map(|sql| {
                (name, sql)
            }))
            .map(|(name, sql)| {
                transaction.execute_batch(&format!("DROP TRIGGER \"{name}\";")).map(|()| sql)
            })
            .collect::<Result<_, _>>()?
    };
    if !fast {
        transaction.execute_batch("PRAGMA defer_foreign_keys = ON;")?;
        // With the checks on, other references are carried out by them; the
        // rows that cannot outlive the session still go first.
        for (parent, keys) in [("session_entries", "temp.import_purge_entries"), ("sessions", "temp.import_purge")] {
            for (table, column) in dependents_of(&transaction, parent)? {
                transaction.execute(&format!("DELETE FROM \"{table}\" WHERE \"{column}\" IN (SELECT id FROM {keys})"), [])?;
            }
        }
    }
    // The batch's search rows, found up front, by row number.
    let has_search: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='session_entry_fts')", [], |row| row.get(0))?;
    if has_search {
        transaction.execute("DELETE FROM session_entry_fts WHERE rowid IN (SELECT row FROM temp.import_purge_search)", [])?;
    }
    transaction.execute_batch(
        "DELETE FROM session_heads WHERE session_id IN (SELECT id FROM temp.import_purge);
         DELETE FROM session_entries WHERE id IN (SELECT id FROM temp.import_purge_entries);
         DELETE FROM import_native_sessions WHERE native_key IN (SELECT key FROM temp.import_purge_natives);",
    )?;
    let removed = transaction.query_row(
        "SELECT COUNT(*) FROM sessions WHERE kind='imported' AND parent_session_id IS NULL AND id IN (SELECT id FROM temp.import_purge)",
        [],
        |row| row.get::<_, i64>(0),
    )? as usize;
    // Parents and subagents in one statement, so the parent link holds.
    transaction.execute("DELETE FROM sessions WHERE kind='imported' AND id IN (SELECT id FROM temp.import_purge)", [])?;
    for sql in lifted {
        transaction.execute_batch(&sql)?;
    }
    crate::external_import::install_import_foundation(&transaction)?;
    transaction.commit()?;
    Ok(removed)
}

/// Every foreign key column that points at `parent`, whatever its action.
fn references_to(db: &Connection, parent: &str) -> Result<Vec<(String, String)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT m.name, f.\"from\" FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) f
         WHERE m.type='table' AND f.\"table\"=?1 ORDER BY m.name",
    )?;
    let rows = statement.query_map(params![parent], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
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

/// Top-level imported sessions that are runs, not chats. Folders are
/// checked without Bridge's database held (they may touch the disk).
fn background_sessions(db: &Mutex<Connection>, rules: &Rules, longest: &mut std::time::Duration) -> Result<Vec<String>, BridgeError> {
    let folders: Vec<(String, String)> = held(db, longest, |db| -> Result<Vec<(String, String)>, BridgeError> {
        let mut statement = db.prepare("SELECT id, cwd FROM sessions WHERE kind='imported' AND parent_session_id IS NULL AND cwd IS NOT NULL")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    })?;
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut checked: HashMap<String, bool> = HashMap::new();
    for (id, cwd) in folders {
        let bridge = *checked.entry(cwd.clone()).or_insert_with(|| rules.is_bridge_folder(&cwd));
        if bridge && seen.insert(id.clone()) {
            out.push(id);
        }
    }
    // Probes: every entry a user message, and every one a slash command.
    let probes = held(db, longest, |db| -> Result<Vec<String>, BridgeError> {
        let mut statement = db.prepare(
            "SELECT s.id FROM sessions s
             WHERE s.kind='imported' AND s.parent_session_id IS NULL
               AND EXISTS (SELECT 1 FROM session_entries e WHERE e.session_id=s.id)
               AND NOT EXISTS (SELECT 1 FROM session_entries e WHERE e.session_id=s.id AND e.kind <> 'user.message')",
        )?;
        let only_user: Vec<String> = statement.query_map([], |row| row.get(0))?.collect::<Result<_, _>>()?;
        let mut texts = db.prepare("SELECT json_extract(payload,'$.text') FROM session_entries WHERE session_id=?1")?;
        let mut found = Vec::new();
        for id in only_user {
            let all: Vec<Option<String>> = texts.query_map(params![id], |row| row.get(0))?.collect::<Result<_, _>>()?;
            if command_run(all.iter().map(|text| (Some("user.message"), text.as_deref()))) {
                found.push(id);
            }
        }
        Ok(found)
    })?;
    for id in probes {
        if seen.insert(id.clone()) {
            out.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "import_placement/tests.rs"]
mod tests;
