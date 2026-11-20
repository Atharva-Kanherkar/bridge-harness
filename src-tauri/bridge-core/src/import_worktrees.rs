//! Another tool's git worktrees, adopted as Bridge workspaces where they are
//! (#1056). Bridge never moves, changes or deletes an adopted folder: it is
//! marked as the other tool's, and archiving it in Bridge only forgets it.
//!
//! Three steps, kept apart so the middle one can be fuzzed without git:
//! [`probe`] asks git what each folder is (no database held), [`plan`] decides
//! what to do with every candidate (pure), and [`write`] records the result in
//! one transaction.

use crate::{git, store, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The table that marks a workspace as another tool's.
const TABLE: &str = "adopted_worktrees";

/// Setup, run and archive scripts a tool keeps for a repository. Kept with
/// the adopted workspace; Bridge never runs them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scripts {
    pub setup: Option<String>,
    pub run: Option<String>,
    pub run_mode: Option<String>,
    pub archive: Option<String>,
}

impl Scripts {
    pub fn is_empty(&self) -> bool {
        self.setup.is_none() && self.run.is_none() && self.archive.is_none()
    }
}

/// A folder another tool made a workspace of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// The tool's import id (`conductor`).
    pub tool: &'static str,
    pub path: PathBuf,
    /// The folder's own name, used when the tool gives none.
    pub name: String,
    /// What the tool called the workspace.
    pub title: Option<String>,
    /// Archived in the tool: left out.
    pub archived: bool,
    /// Where the record came from, for provenance (`conductor:<id>`, or the
    /// folder for one found on disk only).
    pub source_ref: String,
    pub scripts: Option<Scripts>,
}

/// What git says a candidate folder is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// No folder there.
    Missing,
    /// A folder git does not know.
    NotGit,
    /// The repository's main checkout, not a workspace.
    MainCheckout,
    /// A checkout its main repository does not list as a worktree.
    Unregistered,
    /// A worktree on a detached HEAD: it needs a branch first.
    Detached { root: PathBuf },
    OnBranch { root: PathBuf, branch: String },
}

/// Why a candidate was not adopted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ArchivedInTool,
    Missing,
    NotAWorktree,
    NeedsBranch,
    MainCheckout,
    Duplicate,
}

impl Reason {
    /// How the import summary says it.
    pub fn text(self) -> &'static str {
        match self {
            Self::ArchivedInTool => "workspaces archived there",
            Self::Missing => "workspaces whose folder is gone",
            Self::NotAWorktree => "folders that are not git worktrees",
            Self::NeedsBranch => "workspaces on a detached HEAD, which need a branch first",
            Self::MainCheckout => "repository checkouts, which are projects, not workspaces",
            Self::Duplicate => "workspaces listed twice",
        }
    }
}

/// What happens to one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Adopt { root: PathBuf, branch: String },
    AlreadyInBridge,
    LeftOut(Reason),
}

/// Ask git what `path` is. Runs git; never call it while the database is held.
pub fn probe(path: &Path) -> Probe {
    if !path.is_dir() {
        return Probe::Missing;
    }
    let Some(root) = git::main_worktree_root(path) else {
        return Probe::NotGit;
    };
    let real = real_path(path);
    let root = real_path(&root);
    if real == root {
        return Probe::MainCheckout;
    }
    let Ok(entries) = git::list_worktrees(&root) else {
        return Probe::NotGit;
    };
    match entries.iter().find(|entry| real_path(&entry.path) == real) {
        None => Probe::Unregistered,
        Some(entry) => match branch_of(entry) {
            Some(branch) => Probe::OnBranch { root, branch },
            None => Probe::Detached { root },
        },
    }
}

/// The branch a worktree is on, by `git worktree list --porcelain`: none when
/// detached, even if a branch line came with it.
fn branch_of(entry: &git::GitWorktreeEntry) -> Option<String> {
    if entry.detached {
        return None;
    }
    entry.branch.clone().filter(|branch| !branch.is_empty())
}

/// A folder's real path, or the path as given when it does not resolve.
pub fn real_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Decide every candidate's outcome, given what git said about it and the
/// folders Bridge already has. Pure: every candidate gets exactly one outcome,
/// no folder is adopted twice, and nothing archived in the tool is adopted.
pub fn plan(candidates: &[(Candidate, Probe)], in_bridge: &HashSet<PathBuf>) -> Vec<Outcome> {
    let mut seen = HashSet::new();
    candidates
        .iter()
        .map(|(candidate, probe)| {
            let path = real_path(&candidate.path);
            if !seen.insert(path.clone()) {
                return Outcome::LeftOut(Reason::Duplicate);
            }
            if candidate.archived {
                return Outcome::LeftOut(Reason::ArchivedInTool);
            }
            match probe {
                Probe::Missing => Outcome::LeftOut(Reason::Missing),
                Probe::NotGit | Probe::Unregistered => Outcome::LeftOut(Reason::NotAWorktree),
                Probe::MainCheckout => Outcome::LeftOut(Reason::MainCheckout),
                Probe::Detached { .. } => Outcome::LeftOut(Reason::NeedsBranch),
                Probe::OnBranch { .. } if in_bridge.contains(&path) => Outcome::AlreadyInBridge,
                Probe::OnBranch { root, branch } => Outcome::Adopt { root: root.clone(), branch: branch.clone() },
            }
        })
        .collect()
}

/// What an adoption did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The new workspaces, with their folders.
    pub adopted: Vec<(String, PathBuf)>,
    pub already_in_bridge: usize,
    pub left_out: BTreeMap<Reason, usize>,
    /// Imported chats that moved under an adopted workspace.
    pub relinked_chats: usize,
    /// Adopted workspaces that brought scripts with them.
    pub with_scripts: usize,
}

impl Report {
    /// Bridge's state changed: a workspace was added or a chat moved.
    pub fn imported_any(&self) -> bool {
        !self.adopted.is_empty() || self.relinked_chats > 0
    }
}

fn install(db: &Connection) -> Result<(), BridgeError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS adopted_worktrees (
            workspace_id TEXT PRIMARY KEY,
            tool TEXT NOT NULL,
            path TEXT NOT NULL,
            source_ref TEXT NOT NULL,
            scripts TEXT,
            adopted_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

fn table_exists(db: &Connection) -> Result<bool, BridgeError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        params![TABLE],
        |row| row.get(0),
    )?)
}

/// Whether a workspace's folder belongs to another tool. Bridge must never
/// delete such a folder.
pub fn is_external(db: &Connection, workspace_id: &str) -> Result<bool, BridgeError> {
    if !table_exists(db)? {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM adopted_worktrees WHERE workspace_id=?1)",
        params![workspace_id],
        |row| row.get(0),
    )?)
}

/// Forget that a workspace was adopted, when Bridge archives it.
pub(crate) fn forget(db: &Connection, workspace_id: &str) -> Result<(), BridgeError> {
    if table_exists(db)? {
        db.execute("DELETE FROM adopted_worktrees WHERE workspace_id=?1", params![workspace_id])?;
    }
    Ok(())
}

/// The folders Bridge already has as workspaces, by real path.
pub fn workspace_paths(db: &Connection) -> Result<HashSet<PathBuf>, BridgeError> {
    let mut statement = db.prepare("SELECT path FROM workspaces WHERE path IS NOT NULL")?;
    let paths = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut out = HashSet::new();
    for path in paths {
        out.insert(real_path(Path::new(&path?)));
    }
    Ok(out)
}

/// Record the plan: a project per main checkout, a workspace per adopted
/// worktree, marked as the tool's, and imported chats from that folder moved
/// under it. One transaction.
pub fn write(db: &Connection, candidates: &[Candidate], outcomes: &[Outcome]) -> Result<Report, BridgeError> {
    install(db)?;
    let transaction = db.unchecked_transaction()?;
    let now = Utc::now().to_rfc3339();
    let mut report = Report::default();
    for (candidate, outcome) in candidates.iter().zip(outcomes) {
        let (root, branch) = match outcome {
            Outcome::Adopt { root, branch } => (root, branch),
            Outcome::AlreadyInBridge => {
                report.already_in_bridge += 1;
                continue;
            }
            Outcome::LeftOut(reason) => {
                *report.left_out.entry(*reason).or_insert(0) += 1;
                continue;
            }
        };
        // The plan was made under the same lock (see [`adopt`]), so a folder
        // it says to adopt is not Bridge's yet.
        let path = real_path(&candidate.path).to_string_lossy().into_owned();
        let root = root.to_string_lossy().into_owned();
        let project_name = Path::new(&root).file_name().and_then(|name| name.to_str()).unwrap_or("Repository").to_owned();
        transaction.execute(
            "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
            params![uuid::Uuid::new_v4().to_string(), project_name, root, now],
        )?;
        let project_id: Option<String> = transaction
            .query_row("SELECT id FROM projects WHERE path=?1", params![root], |row| row.get(0))
            .optional()?;
        let id = uuid::Uuid::new_v4().to_string();
        let title = candidate.title.as_deref().map(str::trim).filter(|title| !title.is_empty()).unwrap_or(&candidate.name);
        transaction.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,'idle',?7)",
            params![id, project_id, candidate.name, title, branch, path, now],
        )?;
        let scripts = candidate
            .scripts
            .as_ref()
            .filter(|scripts| !scripts.is_empty())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        if scripts.is_some() {
            report.with_scripts += 1;
        }
        transaction.execute(
            "INSERT INTO adopted_worktrees(workspace_id,tool,path,source_ref,scripts,adopted_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, candidate.tool, path, candidate.source_ref, scripts, now],
        )?;
        report.relinked_chats += transaction.execute(
            "UPDATE sessions SET workspace_id=?1 WHERE kind='imported' AND workspace_id IS NULL AND cwd=?2",
            params![id, path],
        )?;
        store::event(&transaction, "import", "workspace.adopted", &id, &format!("Adopted {title} from {}", candidate.tool))?;
        report.adopted.push((id, PathBuf::from(path)));
    }
    transaction.commit()?;
    Ok(report)
}

/// Probe every candidate without the database held, then plan and write
/// under it.
pub fn adopt(db: &std::sync::Mutex<Connection>, candidates: Vec<Candidate>) -> Result<Report, BridgeError> {
    let probed: Vec<(Candidate, Probe)> = candidates
        .into_iter()
        .map(|candidate| {
            let probe = probe(&candidate.path);
            (candidate, probe)
        })
        .collect();
    let db = db.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let in_bridge = workspace_paths(&db)?;
    let outcomes = plan(&probed, &in_bridge);
    let candidates: Vec<Candidate> = probed.into_iter().map(|(candidate, _)| candidate).collect();
    write(&db, &candidates, &outcomes)
}

#[cfg(test)]
#[path = "import_worktrees/tests.rs"]
pub(crate) mod tests;
