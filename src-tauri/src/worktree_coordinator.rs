use crate::{git, store, BridgeError};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub struct WorktreeCoordinator;

impl WorktreeCoordinator {
    pub fn prepare_isolated_worker(
        db: &Connection,
        namespace_root: &Path,
        workspace_id: &str,
        task_worktree: &Path,
        task_branch: &str,
        session_id: &str,
        owned_paths: &[String],
    ) -> Result<(PathBuf, String), BridgeError> {
        let active_writers = store::worker_leases(db, workspace_id)?
            .into_iter()
            .filter(|lease| lease.session_id != session_id && lease.lease_status == "active")
            .filter(|lease| lease.write_mode != "read_only")
            .map(|lease| git::ActiveWriter {
                session_id: lease.session_id,
                owned_paths: serde_json::from_value(lease.owned_paths).unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        let branch = format!("{}-worker-{}", task_branch, git::slug(session_id));
        let worktree = git::create_child_worktree(
            task_worktree,
            namespace_root,
            session_id,
            &branch,
            owned_paths,
            &active_writers,
        )?;
        db.execute(
            "UPDATE worker_runtime SET worktree_path=?2,worktree_branch=?3,updated_at=?4 WHERE session_id=?1",
            rusqlite::params![session_id, worktree.path.to_string_lossy(), worktree.branch, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok((worktree.path, worktree.branch))
    }

    pub fn child_worktrees_available(namespace_root: &Path) -> bool {
        namespace_root.parent().is_some()
    }
}
