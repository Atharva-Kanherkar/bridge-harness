//! Projects & workspaces domain: the state logic behind the `projects/*`
//! and `workspaces/*` protocol methods, as [`BridgeCore`] methods.
//!
//! Host shells keep only transport concerns — which threads run blocking Git
//! or filesystem scans, and event emission after a mutation. Everything that
//! reads or writes runtime state lives here.

use crate::model::BridgeState;
use crate::runtime::BridgeCore;
use crate::{git, store, BridgeError};
use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use uuid::Uuid;

impl BridgeCore {
    pub fn add_project(&self, path: &str) -> Result<BridgeState, BridgeError> {
        let clean = git::validate_repo(Path::new(path))?;
        let name = Path::new(&clean)
            .file_name()
            .and_then(|x| x.to_str())
            .unwrap_or("Repository")
            .to_string();
        let id = Uuid::new_v4().to_string();
        let db = self.db.lock().unwrap();
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

    /// Create a repo-less workspace. A folder/git repo can be connected later.
    pub fn create_workspace(&self, title: &str) -> Result<BridgeState, BridgeError> {
        let name = title.trim();
        if name.is_empty() {
            return Err(BridgeError::Invalid("Workspace name is required".into()));
        }
        let id = Uuid::new_v4().to_string();
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO workspaces(id,title,status,created_at) VALUES(?1,?2,'idle',?3)",
            params![id, name, Utc::now().to_rfc3339()],
        )?;
        store::event(
            &db,
            "supervisor",
            "workspace.created",
            &id,
            &format!("Created workspace {name}"),
        )?;
        store::state(&db)
    }

    pub fn connect_workspace_folder(
        &self,
        workspace_id: &str,
        path: &str,
    ) -> Result<BridgeState, BridgeError> {
        let folder = Path::new(path);
        if !folder.is_dir() {
            return Err(BridgeError::Invalid("That folder no longer exists".into()));
        }
        let db = self.db.lock().unwrap();
        let (resolved_path, project_id, branch) = match git::validate_repo(folder) {
            Ok(root) => {
                let name = Path::new(&root)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("Repository")
                    .to_string();
                db.execute(
                    "INSERT OR IGNORE INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)",
                    params![
                        Uuid::new_v4().to_string(),
                        name,
                        root,
                        Utc::now().to_rfc3339()
                    ],
                )?;
                let project_id: Option<String> = db
                    .query_row(
                        "SELECT id FROM projects WHERE path=?1",
                        params![root],
                        |r| r.get(0),
                    )
                    .ok();
                (root.clone(), project_id, git::current_branch(folder))
            }
            Err(_) => (folder.to_string_lossy().to_string(), None, None),
        };
        db.execute(
            "UPDATE workspaces SET path=?2,project_id=?3,branch=?4 WHERE id=?1",
            params![workspace_id, resolved_path, project_id, branch],
        )?;
        store::event(
            &db,
            "supervisor",
            "workspace.connected",
            workspace_id,
            &format!("Connected {resolved_path}"),
        )?;
        store::state(&db)
    }

    /// Resolve a session's workspace root (`s.cwd` falling back to the
    /// connected workspace's `path`). Returns `None` for chats with no folder
    /// — e.g. direct chats — or when the recorded path no longer exists on
    /// disk.
    pub fn session_workspace_root(&self, session_id: &str) -> Option<PathBuf> {
        let path: Option<String> = self
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT COALESCE(s.cwd, w.path) FROM sessions s LEFT JOIN workspaces w ON w.id=s.workspace_id WHERE s.id=?1",
                params![session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        let candidate = PathBuf::from(path?);
        candidate.is_dir().then_some(candidate)
    }

    /// The workspace's recorded path. Hosts resolve this under the lock, then
    /// run Git scans outside it so a slow status scan cannot delay message
    /// submission or streaming writes.
    pub fn workspace_path(&self, workspace_id: &str) -> Result<String, BridgeError> {
        let db = self.db.lock().unwrap();
        Ok(db.query_row(
            "SELECT path FROM workspaces WHERE id=?1",
            params![workspace_id],
            |r| r.get(0),
        )?)
    }

    /// Record the result of a Git status scan produced by [`git::stats`].
    pub fn record_workspace_git_stats(
        &self,
        workspace_id: &str,
        (dirty, additions, deletions): (i64, i64, i64),
    ) -> Result<BridgeState, BridgeError> {
        let db = self.db.lock().unwrap();
        db.execute(
            "UPDATE workspaces SET dirty_files=?2,additions=?3,deletions=?4 WHERE id=?1",
            params![workspace_id, dirty, additions, deletions],
        )?;
        store::state(&db)
    }

    /// Archive a clean, fully-stopped workspace: delete its dependent records
    /// in one transaction and remove the worktree. The host emits its
    /// state-changed notification after this returns.
    pub fn archive_workspace(&self, workspace_id: &str) -> Result<BridgeState, BridgeError> {
        let db = self.db.lock().unwrap();
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
        archive_workspace_records(&db, workspace_id, || {
            git::remove_worktree(Path::new(&repo), Path::new(&path))
        })?;
        store::event(
            &db,
            "supervisor",
            "workspace.archived",
            workspace_id,
            "Archived clean workspace; branch preserved",
        )?;
        store::state(&db)
    }
}

/// Delete every record that depends on a workspace, then the workspace row
/// itself, in one transaction; `remove_worktree` runs inside it so a failed
/// worktree removal rolls everything back.
pub fn archive_workspace_records(
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
    transaction.execute("DELETE FROM workspaces WHERE id=?1", params![workspace_id])?;
    remove_worktree()?;
    transaction.commit()?;
    Ok(())
}
