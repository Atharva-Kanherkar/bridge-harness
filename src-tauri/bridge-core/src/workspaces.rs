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
    /// in one transaction and remove the worktree. Returns without building a
    /// snapshot so the host can emit its state-changed notification as soon
    /// as the mutation lands — a snapshot failure afterwards must not
    /// suppress the notification for an archive that already happened.
    pub fn archive_workspace(&self, workspace_id: &str) -> Result<(), BridgeError> {
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
        // The archive is committed; publish only now so a rolled-back
        // transaction can never announce itself.
        self.events.publish(crate::events::CoreEvent::StateChanged);
        Ok(())
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
    // The Work fact cache is keyed by workspace id but carries no foreign key,
    // because its key is kind-defined. Clean it up here so an archived
    // workspace's last observation does not linger.
    transaction.execute(
        "DELETE FROM work_fact_cache WHERE kind=?1 AND cache_key=?2",
        params![crate::work::FACT_CACHE_BASE_DIVERGENCE, workspace_id],
    )?;
    transaction.execute("DELETE FROM workspaces WHERE id=?1", params![workspace_id])?;
    store::event(
        &transaction,
        "supervisor",
        "workspace.archived",
        workspace_id,
        "Archived clean workspace; branch preserved",
    )?;
    remove_worktree()?;
    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository(root: &Path) -> PathBuf {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        git(&repo, &["config", "user.name", "Bridge Test"]);
        std::fs::write(repo.join("shared.txt"), "base\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "fixture", "-q"]);
        repo
    }

    fn fixture() -> (tempfile::TempDir, BridgeCore) {
        let scratch = tempfile::tempdir().unwrap();
        let core = BridgeCore::for_tests(scratch.path());
        (scratch, core)
    }

    fn count(core: &BridgeCore, table: &str) -> i64 {
        core.db
            .lock()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn event_exists(core: &BridgeCore, kind: &str, entity_id: &str) -> bool {
        core.db
            .lock()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE kind=?1 AND entity_id=?2)",
                params![kind, entity_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn add_project_registers_a_repository_and_rejects_plain_folders() {
        let (scratch, core) = fixture();
        let repo = repository(scratch.path());

        let plain = scratch.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert!(core.add_project(plain.to_str().unwrap()).is_err());
        assert_eq!(count(&core, "projects"), 0);

        let snapshot = core.add_project(repo.to_str().unwrap()).unwrap();
        assert_eq!(snapshot.projects.len(), 1);
        assert_eq!(snapshot.projects[0].name, "repo");
        // Registering the same repository again is idempotent.
        let snapshot = core.add_project(repo.to_str().unwrap()).unwrap();
        assert_eq!(snapshot.projects.len(), 1);
    }

    #[test]
    fn create_workspace_requires_a_name_and_persists_the_row() {
        let (_scratch, core) = fixture();
        assert!(core.create_workspace("   ").is_err());
        assert_eq!(count(&core, "workspaces"), 0);

        let snapshot = core.create_workspace("  Payments  ").unwrap();
        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.workspaces[0].title, "Payments");
        let workspace_id = snapshot.workspaces[0].id.clone();
        assert!(event_exists(&core, "workspace.created", &workspace_id));
    }

    #[test]
    fn connect_workspace_folder_links_repos_and_accepts_plain_folders() {
        let (scratch, core) = fixture();
        let repo = repository(scratch.path());
        core.create_workspace("Task").unwrap();
        let workspace_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM workspaces", [], |row| row.get(0))
            .unwrap();

        assert!(core
            .connect_workspace_folder(&workspace_id, scratch.path().join("gone").to_str().unwrap())
            .is_err());

        // A plain folder connects as-is: no project link, no branch.
        let plain = scratch.path().join("plain");
        std::fs::create_dir(&plain).unwrap();
        core.connect_workspace_folder(&workspace_id, plain.to_str().unwrap())
            .unwrap();
        let (path, project_id, branch): (String, Option<String>, Option<String>) = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT path,project_id,branch FROM workspaces WHERE id=?1",
                params![workspace_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(path, plain.to_string_lossy());
        assert_eq!((project_id, branch), (None, None));

        // A repository resolves to its root, registers a project, records the branch.
        core.connect_workspace_folder(&workspace_id, repo.to_str().unwrap())
            .unwrap();
        let (project_id, branch): (Option<String>, Option<String>) = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT project_id,branch FROM workspaces WHERE id=?1",
                params![workspace_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(project_id.is_some());
        assert!(branch.is_some());
        assert_eq!(count(&core, "projects"), 1);
        assert!(event_exists(&core, "workspace.connected", &workspace_id));
    }

    #[test]
    fn session_workspace_root_prefers_cwd_and_requires_the_directory_to_exist() {
        let (scratch, core) = fixture();
        let existing = scratch.path().join("cwd");
        std::fs::create_dir(&existing).unwrap();
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO workspaces(id,title,status,created_at) VALUES('w','Task','idle','now')", []).unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd) VALUES('with-cwd','w','codex','S','idle','reported',?1)",
                params![existing.to_string_lossy()],
            ).unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,cwd) VALUES('missing-cwd','w','codex','S','idle','reported','/nonexistent/path')",
                [],
            ).unwrap();
        }
        assert_eq!(core.session_workspace_root("with-cwd"), Some(existing));
        assert_eq!(core.session_workspace_root("missing-cwd"), None);
        assert_eq!(core.session_workspace_root("unknown-session"), None);
    }

    #[test]
    fn workspace_path_errors_for_unknown_workspaces() {
        let (_scratch, core) = fixture();
        assert!(matches!(
            core.workspace_path("missing"),
            Err(BridgeError::Db(_))
        ));
    }

    #[test]
    fn recording_git_stats_updates_counts_and_tolerates_a_vanished_workspace() {
        let (_scratch, core) = fixture();
        core.create_workspace("Task").unwrap();
        let workspace_id: String = core
            .db
            .lock()
            .unwrap()
            .query_row("SELECT id FROM workspaces", [], |row| row.get(0))
            .unwrap();
        core.record_workspace_git_stats(&workspace_id, (3, 10, 2))
            .unwrap();
        let (dirty, adds, dels): (i64, i64, i64) = core
            .db
            .lock()
            .unwrap()
            .query_row(
                "SELECT dirty_files,additions,deletions FROM workspaces WHERE id=?1",
                params![workspace_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((dirty, adds, dels), (3, 10, 2));
        // The refresh race: a workspace archived between the path resolve and
        // the stats write updates zero rows and still returns a snapshot.
        assert!(core
            .record_workspace_git_stats("already-archived", (1, 1, 1))
            .is_ok());
    }

    /// Workspace 'w' backed by a real worktree of a real repository.
    fn archive_fixture(core: &BridgeCore, root: &Path) -> (PathBuf, PathBuf) {
        let repo = repository(root);
        let worktree = root.join("worktree");
        git::create_worktree(&repo, &worktree, "bridge/task").unwrap();
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')",
            params![repo.to_string_lossy()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,branch,path,status,created_at) VALUES('w','p','Task','bridge/task',?1,'stopped','now')",
            params![worktree.to_string_lossy()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,ended_at) VALUES('s','w','codex','S','stopped','reported','now')",
            [],
        )
        .unwrap();
        (repo, worktree)
    }

    #[test]
    fn archive_workspace_removes_records_and_the_worktree() {
        let (scratch, core) = fixture();
        let (_repo, worktree) = archive_fixture(&core, scratch.path());
        let mut events = core.events.subscribe();
        core.archive_workspace("w").unwrap();
        // Exactly one state-changed refetch hint, published after the commit.
        assert!(matches!(
            events.try_recv().unwrap(),
            crate::events::CoreEvent::StateChanged
        ));
        assert!(events.try_recv().is_err(), "exactly one event per archive");
        assert_eq!(count(&core, "workspaces"), 0);
        assert_eq!(count(&core, "sessions"), 0);
        assert!(!worktree.exists(), "worktree directory must be removed");
        assert!(event_exists(&core, "workspace.archived", "w"));
    }

    #[test]
    fn archive_workspace_refuses_active_sessions_and_dirty_worktrees() {
        let (scratch, core) = fixture();
        let (_repo, worktree) = archive_fixture(&core, scratch.path());

        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET status='working',ended_at=NULL WHERE id='s'",
                [],
            )
            .unwrap();
        let error = core.archive_workspace("w").unwrap_err();
        assert!(
            error.to_string().contains("Stop every running session"),
            "{error}"
        );

        core.db
            .lock()
            .unwrap()
            .execute(
                "UPDATE sessions SET status='stopped',ended_at='now' WHERE id='s'",
                [],
            )
            .unwrap();
        std::fs::write(worktree.join("shared.txt"), "dirty\n").unwrap();
        let error = core.archive_workspace("w").unwrap_err();
        assert!(error.to_string().contains("uncommitted"), "{error}");
        assert_eq!(
            count(&core, "workspaces"),
            1,
            "refusals must not delete anything"
        );
        assert_eq!(count(&core, "sessions"), 1);
        assert!(worktree.exists());
    }

    #[test]
    fn archive_workspace_rolls_back_when_worktree_removal_fails() {
        let (scratch, core) = fixture();
        let (_repo, worktree) = archive_fixture(&core, scratch.path());
        // Replace the worktree with a clean plain repository at the same
        // path: the pre-checks pass, but `git worktree remove` fails because
        // the path is not a worktree of the project repository.
        std::fs::remove_dir_all(&worktree).unwrap();
        let imposter = repository(scratch.path().join("elsewhere").as_path());
        std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
        std::fs::rename(&imposter, &worktree).unwrap();

        let mut events = core.events.subscribe();
        let error = core.archive_workspace("w").unwrap_err();
        assert!(matches!(error, BridgeError::Git(_)), "{error}");
        assert!(
            events.try_recv().is_err(),
            "a rolled-back archive must emit nothing"
        );
        assert_eq!(
            count(&core, "workspaces"),
            1,
            "failed archive must roll back"
        );
        assert_eq!(count(&core, "sessions"), 1);
        assert!(!event_exists(&core, "workspace.archived", "w"));
    }

    #[test]
    fn archive_workspace_does_not_remove_the_worktree_when_audit_fails() {
        let (scratch, core) = fixture();
        let (_repo, worktree) = archive_fixture(&core, scratch.path());
        core.db
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_archive_audit BEFORE INSERT ON events
                 WHEN NEW.kind='workspace.archived'
                 BEGIN SELECT RAISE(FAIL, 'injected audit failure'); END;",
            )
            .unwrap();

        let mut events = core.events.subscribe();
        assert!(core.archive_workspace("w").is_err());
        assert!(
            events.try_recv().is_err(),
            "a rolled-back archive must publish nothing"
        );
        assert_eq!(count(&core, "workspaces"), 1);
        assert_eq!(count(&core, "sessions"), 1);
        assert!(
            worktree.exists(),
            "audit failure must happen before worktree removal"
        );
    }
}
