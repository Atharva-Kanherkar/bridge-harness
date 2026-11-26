//! Conductor's workspaces into Bridge (#1057), adopted in place through
//! [`crate::import_worktrees`]. Conductor's chats are Claude Code and Codex
//! sessions; they come over with those imports and land under the adopted
//! workspace by its folder.
//!
//! Built from Conductor 0.97.0's installed bundle, the structure of its
//! database and the rollback SQL it keeps for 98 of its 178 migrations
//! (fixtures: `testing/fixtures/import/conductor/`), its bundled agent guide,
//! and its public docs.
//!
//! - **Path A** always runs. The workspaces are the git worktrees under
//!   `~/conductor/workspaces/<repo>/`, and under `<repo>/.conductor/` for the
//!   repositories it knows. Each is named by its folder.
//! - **Path B** runs when `conductor.db` is a version Bridge reads: from the
//!   oldest to the newest checked, or newer with every column of
//!   [`NEWEST_READS`]. It reads names, archived state and scripts from
//!   Conductor's database, opened read-only. Only named columns of
//!   [`ALLOWED_TABLES`] are queried, never `env_vars`, `settings` or a
//!   message table. Any other version, or a database that cannot be read,
//!   falls back to Path A with a note.

use crate::import_worktrees::{self, Candidate, Scripts};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The import id the Import page and the job use.
pub const PROVIDER: &str = "conductor";
/// The `_sqlx_migrations` versions Bridge reads. 80 is the oldest
/// Conductor's rollbacks rebuild; 181 is Conductor 0.97.0, the newest Bridge
/// was checked against. A newer database is read while it has all of
/// [`NEWEST_READS`].
pub const MIGRATIONS: Versions = Versions { oldest: 80, newest_checked: 181 };

/// Every column Bridge reads at [`MIGRATIONS`]`.newest_checked` (the
/// project era: workspaces name a project, not a repository).
pub const NEWEST_READS: Reads = &[
    ("_sqlx_migrations", &["version", "success"]),
    ("repos", &["id", "name", "root_path", "storage_version", "setup_script", "run_script", "run_script_mode", "archive_script"]),
    ("project_repositories", &["project_id", "repository_id", "position"]),
    ("workspace_checkouts", &["workspace_id", "repository_id", "path", "position"]),
    (
        "workspaces",
        &["local_id", "id", "project_id", "directory_name", "workspace_name", "branch", "state", "workspace_path", "DEPRECATED_archived"],
    ),
    ("sessions", &["workspace_id", "agent_type"]),
];

/// The only tables Bridge reads from Conductor's database.
pub const ALLOWED_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "repos",
    "projects",
    "project_repositories",
    "workspaces",
    "workspace_checkouts",
    "sessions",
];

/// Tables that hold secrets, settings or chat content. Never read.
pub const DENIED_TABLES: &[&str] = &[
    "env_vars",
    "settings",
    "identity_associations",
    "session_messages",
    "session_messages_inbox",
    "session_messages_outbox",
    "session_events",
    "session_state",
    "attachments",
    "review_comments",
    "terminal_sessions",
    "favorite_models",
];

/// Where Conductor keeps things for a home folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    /// `~/Library/Application Support/com.conductor.app/conductor.db`.
    pub store: PathBuf,
    /// Where workspaces go by default (storage version 3).
    pub workspaces_root: PathBuf,
    /// Clones Conductor made; main checkouts, not workspaces.
    pub repos_root: PathBuf,
}

impl Paths {
    pub fn for_home(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            store: home.join("Library/Application Support/com.conductor.app/conductor.db"),
            workspaces_root: home.join("conductor/workspaces"),
            repos_root: home.join("conductor/repos"),
        }
    }

    /// Conductor has been used on this Mac.
    pub fn installed(&self) -> bool {
        self.store.is_file() || self.workspaces_root.is_dir()
    }
}

/// A repository Conductor knows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Repo {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub storage_version: i64,
    pub scripts: Scripts,
}

/// One repository's checkout in a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub repo_id: String,
    pub path: Option<PathBuf>,
    pub position: i64,
}

/// A workspace row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    /// `local_id`, else `id`; the other is kept too, since tables that point at
    /// a workspace use one or the other.
    pub keys: Vec<String>,
    pub repo_id: Option<String>,
    pub project_id: Option<String>,
    pub directory: String,
    pub name: Option<String>,
    pub branch: Option<String>,
    pub archived: bool,
    pub path: Option<PathBuf>,
    pub checkouts: Vec<Checkout>,
}

/// What Bridge read from `conductor.db`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    pub version: i64,
    pub repos: BTreeMap<String, Repo>,
    pub workspaces: Vec<Workspace>,
    /// Chats Conductor ran, by agent (`claude`, `codex`, ...).
    pub chats: BTreeMap<String, u64>,
}

#[cfg(test)]
pub(crate) use crate::import_sqlite::STATEMENTS;
use crate::import_sqlite::{int, missing_reason, text, Allowed, Cell, Gate, Reads, Versions};

const ALLOWED: Allowed = Allowed { tool: "Conductor", tables: ALLOWED_TABLES };

/// Open `conductor.db` without writing anything next to it (see
/// [`crate::import_sqlite::open`]).
fn open(path: &Path) -> rusqlite::Result<Connection> {
    crate::import_sqlite::open(path)
}

/// A table's columns, for an allowed table only.
fn columns(conn: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    ALLOWED.columns(conn, table)
}

/// Rows of an allowed table, as the wanted columns that exist; a missing
/// column reads as `None`.
fn select(conn: &Connection, table: &str, wanted: &[&str]) -> rusqlite::Result<Vec<HashMap<String, Cell>>> {
    ALLOWED.select(conn, table, wanted)
}

/// Why Bridge did not read Conductor's database, for the person.
fn closed(reason: impl Into<String>) -> String {
    reason.into()
}

/// Read `conductor.db`, or say why not. Fails closed: a version older than
/// [`MIGRATIONS`], a missing column Bridge needs (for a version newer than
/// the newest checked: any column of [`NEWEST_READS`]), or any read error
/// returns the reason instead of a partial store.
pub fn read(path: &Path) -> Result<Store, String> {
    let conn = open(path).map_err(|error| closed(format!("Bridge could not open Conductor's database: {error}")))?;
    read_from(&conn)
}

fn read_from(conn: &Connection) -> Result<Store, String> {
    let fail = |error: rusqlite::Error| closed(format!("Bridge could not read Conductor's database: {error}"));
    let migrations = select(conn, "_sqlx_migrations", &["version", "success"]).map_err(fail)?;
    let version = migrations
        .iter()
        .filter(|row| int(row, "success").is_none_or(|success| success != 0))
        .filter_map(|row| int(row, "version"))
        .max()
        .ok_or_else(|| closed("Conductor's database has no version Bridge can read"))?;
    let what = "Conductor's database";
    let gate = MIGRATIONS.gate(what, version).map_err(closed)?;
    let workspace_columns = columns(conn, "workspaces").map_err(fail)?;
    let repo_columns = columns(conn, "repos").map_err(fail)?;
    let has = |set: &HashSet<String>, column: &str| set.contains(column);
    let needed = ["directory_name", "branch", "state"].iter().all(|column| has(&workspace_columns, column))
        && (has(&workspace_columns, "local_id") || has(&workspace_columns, "id"))
        && (has(&workspace_columns, "repository_id") || has(&workspace_columns, "project_id"))
        && ["id", "name", "root_path"].iter().all(|column| has(&repo_columns, column));
    let newer_missing = if gate == Gate::Newer { ALLOWED.missing(conn, NEWEST_READS).map_err(fail)? } else { Vec::new() };
    if !needed || !newer_missing.is_empty() {
        return Err(closed(missing_reason(what, version, gate, &newer_missing)));
    }
    let project_era = !has(&workspace_columns, "repository_id");
    let mut store = Store { version, ..Store::default() };
    for row in select(conn, "repos", &["id", "name", "root_path", "storage_version", "setup_script", "run_script", "run_script_mode", "archive_script"]).map_err(fail)? {
        let (Some(id), Some(root)) = (text(&row, "id"), text(&row, "root_path")) else { continue };
        let name = text(&row, "name").unwrap_or_else(|| Path::new(&root).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
        store.repos.insert(
            id.clone(),
            Repo {
                id,
                name,
                root: PathBuf::from(root),
                storage_version: int(&row, "storage_version").unwrap_or(1),
                scripts: Scripts {
                    setup: text(&row, "setup_script"),
                    run: text(&row, "run_script"),
                    run_mode: text(&row, "run_script_mode"),
                    archive: text(&row, "archive_script"),
                },
            },
        );
    }
    // A project's repositories, first by position (migration 179 on).
    let mut project_repos: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    if project_era {
        for row in select(conn, "project_repositories", &["project_id", "repository_id", "position"]).map_err(fail)? {
            if let (Some(project), Some(repo)) = (text(&row, "project_id"), text(&row, "repository_id")) {
                project_repos.entry(project).or_default().push((int(&row, "position").unwrap_or(0), repo));
            }
        }
    }
    let mut checkouts: HashMap<String, Vec<Checkout>> = HashMap::new();
    for row in select(conn, "workspace_checkouts", &["workspace_id", "repository_id", "path", "position"]).map_err(fail)? {
        if let (Some(workspace), Some(repo)) = (text(&row, "workspace_id"), text(&row, "repository_id")) {
            checkouts.entry(workspace).or_default().push(Checkout {
                repo_id: repo,
                path: text(&row, "path").map(PathBuf::from),
                position: int(&row, "position").unwrap_or(0),
            });
        }
    }
    let wanted = ["local_id", "id", "repository_id", "project_id", "directory_name", "workspace_name", "branch", "state", "workspace_path", "DEPRECATED_archived"];
    for row in select(conn, "workspaces", &wanted).map_err(fail)? {
        let keys: Vec<String> = ["local_id", "id"].iter().filter_map(|column| text(&row, column)).collect();
        let Some(directory) = text(&row, "directory_name") else { continue };
        if keys.is_empty() {
            continue;
        }
        let state = text(&row, "state").unwrap_or_default();
        // `state` says it since the flag was retired; older rows set only the
        // flag. Either one is enough.
        let archived = matches!(state.trim(), "archived" | "deleted")
            || int(&row, "DEPRECATED_archived").is_some_and(|flag| flag != 0);
        let project_id = text(&row, "project_id");
        let repo_id = text(&row, "repository_id").or_else(|| {
            project_id.as_ref().and_then(|project| project_repos.get(project)).and_then(|repos| repos.iter().min().map(|(_, repo)| repo.clone()))
        });
        let mut own: Vec<Checkout> = keys.iter().filter_map(|key| checkouts.get(key)).flatten().cloned().collect();
        own.sort_by(|a, b| (a.position, &a.repo_id).cmp(&(b.position, &b.repo_id)));
        own.dedup_by(|a, b| a.repo_id == b.repo_id);
        store.workspaces.push(Workspace {
            keys,
            repo_id,
            project_id,
            directory,
            name: text(&row, "workspace_name"),
            branch: text(&row, "branch"),
            archived,
            path: text(&row, "workspace_path").map(PathBuf::from),
            checkouts: own,
        });
    }
    for row in select(conn, "sessions", &["workspace_id", "agent_type"]).map_err(fail)? {
        if text(&row, "workspace_id").is_some() {
            *store.chats.entry(text(&row, "agent_type").unwrap_or_else(|| "claude".into())).or_insert(0) += 1;
        }
    }
    Ok(store)
}

/// Where Conductor puts a workspace's folder, by its own rule:
/// - storage 1: `<root>/<directory>`;
/// - storage 2: `<root>/.conductor/<directory>`;
/// - storage 3: `<workspaces root>/<repo name>/<directory>`.
///
/// A directory of `.` is the repository itself, not a workspace.
pub fn workspace_folder(repo: &Repo, directory: &str, workspaces_root: &Path) -> Option<PathBuf> {
    let directory = directory.trim();
    if directory.is_empty() || directory == "." || directory.contains('/') || directory == ".." {
        return None;
    }
    Some(match repo.storage_version {
        1 => repo.root.join(directory),
        2 => repo.root.join(".conductor").join(directory),
        _ => workspaces_root.join(&repo.name).join(directory),
    })
}

/// Everything found for Conductor: the workspaces to adopt, and what was
/// read about them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub candidates: Vec<Candidate>,
    /// Why Conductor's database was not used, when it was not.
    pub note: Option<String>,
    /// Chats Conductor ran, by agent, when its database was read.
    pub chats: BTreeMap<String, u64>,
    pub repos: usize,
}

fn candidate(path: PathBuf, title: Option<String>, archived: bool, source_ref: String, scripts: Option<Scripts>) -> Candidate {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    Candidate { tool: PROVIDER, path, name, title, archived, source_ref, scripts }
}

/// The candidates from Conductor's database.
pub fn from_store(store: &Store, paths: &Paths) -> Vec<Candidate> {
    let mut out = Vec::new();
    for workspace in &store.workspaces {
        let key = workspace.keys.first().cloned().unwrap_or_default();
        if workspace.checkouts.is_empty() {
            let Some(repo) = workspace.repo_id.as_ref().and_then(|id| store.repos.get(id)) else { continue };
            let path = workspace.path.clone().or_else(|| workspace_folder(repo, &workspace.directory, &paths.workspaces_root));
            if let Some(path) = path.filter(|path| path.is_absolute()) {
                out.push(candidate(path, workspace.name.clone(), workspace.archived, format!("conductor:{key}"), Some(repo.scripts.clone())));
            }
            continue;
        }
        for (index, checkout) in workspace.checkouts.iter().enumerate() {
            let Some(repo) = store.repos.get(&checkout.repo_id) else { continue };
            let path = checkout
                .path
                .clone()
                .or_else(|| (index == 0).then(|| workspace.path.clone()).flatten())
                .or_else(|| workspace_folder(repo, &workspace.directory, &paths.workspaces_root));
            // Conductor writes absolute paths; anything else would resolve
            // against Bridge's own working folder.
            let Some(path) = path.filter(|path| path.is_absolute()) else { continue };
            // One Bridge workspace per folder: a multi-repo workspace's other
            // checkouts are named for their repository.
            let title = match (&workspace.name, index > 0) {
                (Some(name), true) => Some(format!("{name} ({})", repo.name)),
                (None, true) => Some(format!("{} ({})", workspace.directory, repo.name)),
                (name, false) => name.clone(),
            };
            out.push(candidate(path, title, workspace.archived, format!("conductor:{key}:{}", checkout.repo_id), Some(repo.scripts.clone())));
        }
    }
    out
}

fn subdirs(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(path) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .map(|entry| entry.path())
        .collect();
    out.sort();
    out
}

/// The workspace folders on disk (Path A): `<workspaces root>/<repo>/<dir>`,
/// and `<root>/.conductor/<dir>` for each repository root Conductor knows.
pub fn on_disk(paths: &Paths, repo_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = subdirs(&paths.workspaces_root).iter().flat_map(|repo| subdirs(repo)).collect();
    let mut roots: Vec<PathBuf> = repo_roots.to_vec();
    roots.extend(subdirs(&paths.repos_root));
    roots.sort();
    roots.dedup();
    for root in roots {
        out.extend(subdirs(&root.join(".conductor")));
    }
    out
}

/// Every Conductor workspace to adopt, from its database when Bridge can read
/// it and from the folders on disk always. A folder in both is the
/// database's (it has the name).
pub fn discover(paths: &Paths) -> Discovery {
    let mut discovery = Discovery::default();
    let store = if paths.store.is_file() { Some(read(&paths.store)) } else { None };
    let mut candidates = Vec::new();
    let mut repo_roots = Vec::new();
    match store {
        Some(Ok(store)) => {
            candidates = from_store(&store, paths);
            repo_roots = store.repos.values().map(|repo| repo.root.clone()).collect();
            discovery.repos = store.repos.len();
            discovery.chats = store.chats;
        }
        Some(Err(reason)) => discovery.note = Some(reason),
        None => {}
    }
    let known: HashSet<PathBuf> = candidates.iter().map(|candidate| import_worktrees::real_path(&candidate.path)).collect();
    for folder in on_disk(paths, &repo_roots) {
        if !known.contains(&import_worktrees::real_path(&folder)) {
            let source = format!("conductor-folder:{}", folder.display());
            candidates.push(candidate(folder, None, false, source, None));
        }
    }
    if discovery.repos == 0 {
        discovery.repos = subdirs(&paths.workspaces_root).len();
    }
    discovery.candidates = candidates;
    discovery
}

#[cfg(test)]
pub(crate) mod fixture;

#[cfg(test)]
mod fuzz;

#[cfg(test)]
#[path = "conductor_import/tests.rs"]
mod tests;
