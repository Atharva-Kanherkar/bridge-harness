use crate::worktree_registry::{self, NewWorktree, WorktreeRetention};
use crate::{git, store, BridgeError};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

pub struct WorktreeCoordinator;

/// The outcome of checking a PR head branch out into a task worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestCheckout {
    /// The workspace node registered for the worktree — always a new node (or
    /// the one a previous checkout registered), never the source workspace.
    pub workspace_id: String,
    pub path: PathBuf,
    pub branch: String,
    pub reused: bool,
}

impl WorktreeCoordinator {
    pub fn prepare_isolated_worker(
        db: &std::sync::Mutex<Connection>,
        namespace_root: &Path,
        workspace_id: &str,
        task_worktree: &Path,
        task_branch: &str,
        session_id: &str,
        owned_paths: &[String],
    ) -> Result<(PathBuf, String), BridgeError> {
        let active_writers = store::worker_leases(&db.lock().unwrap(), workspace_id)?
            .into_iter()
            .filter(|lease| lease.session_id != session_id && lease.lease_status == "active")
            .filter(|lease| lease.write_mode != "read_only")
            .map(|lease| git::ActiveWriter {
                session_id: lease.session_id,
                owned_paths: serde_json::from_value(lease.owned_paths).unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        // A checkout is cheap to cut and expensive to forget. The gate is the
        // policy engine's, so an over-budget repository queues the delegation
        // rather than growing the disk; this refusal is the backstop for the
        // paths that reach creation anyway.
        let repo_root = task_worktree.to_string_lossy().to_string();
        if let Some(reason) =
            worktree_registry::over_capacity(&db.lock().unwrap(), &repo_root, &WorktreeRetention::default())?
        {
            return Err(BridgeError::Invalid(format!(
                "cannot create an isolated worker worktree: {reason}"
            )));
        }
        let branch = format!("{}-worker-{}", task_branch, git::slug(session_id));
        let intended = git::worker_worktree_path(namespace_root, task_worktree, session_id)?;
        if intended.exists() {
            return Err(BridgeError::Invalid("worker checkout already exists".into()));
        }
        if let Some(parent) = intended.parent() { std::fs::create_dir_all(parent)?; }
        let canonical_path = worktree_registry::canonical_key(&intended);
        // Claim the path before Git starts: a concurrent reconciliation can see
        // the directory mid-creation and must not adopt it as expendable output.
        let reserved = db.lock().unwrap().execute(
            "UPDATE worker_runtime SET worktree_path=?2,worktree_branch=?3 WHERE session_id=?1 AND result_status='pending' AND lifecycle_state IN ('starting','working','resuming')",
            params![session_id, canonical_path, branch],
        )?;
        if reserved != 1 { return Err(BridgeError::Invalid("worker is no longer awaiting a checkout".into())); }
        let worktree = git::create_child_worktree(
            task_worktree,
            namespace_root,
            session_id,
            &branch,
            owned_paths,
            &active_writers,
        )?;
        let connection = db.lock().unwrap();
        connection.execute(
            "UPDATE worker_runtime SET worktree_path=?2,worktree_branch=?3,updated_at=?4 WHERE session_id=?1",
            rusqlite::params![session_id, canonical_path, worktree.branch, chrono::Utc::now().to_rfc3339()],
        )?;
        worktree_registry::register(
            &connection,
            &NewWorktree {
                kind: worktree_registry::KIND_WORKER.to_owned(),
                repo_root,
                path: worktree.path.to_string_lossy().to_string(),
                branch: Some(worktree.branch.clone()),
                owner_session_id: Some(session_id.to_owned()),
                owner_workspace_id: Some(workspace_id.to_owned()),
                base_commit: Some(worktree.base_commit.clone()),
            },
        )?;
        drop(connection);
        let seed = crate::dependency_seed::seed(task_worktree, &worktree.path);
        let db = db.lock().unwrap();
        store::event(&db, "storage", "worktree.dependencies", session_id, seed)?;
        let pending: bool = db.query_row(
            "SELECT result_status='pending' AND lifecycle_state IN ('starting','working','resuming') FROM worker_runtime WHERE session_id=?1",
            params![session_id], |row| row.get(0),
        )?;
        if !pending { return Err(BridgeError::Invalid("worker was stopped while its checkout was being prepared".into())); }
        Ok((worktree.path, worktree.branch))
    }

    /// Whether a child worktree may be cut for this task checkout.
    ///
    /// This used to ask only whether the namespace root had a parent directory
    /// — true for every path that is not the filesystem root, so the policy
    /// engine's `ChildWorktreeUnavailable` refusal could never fire. It now
    /// asks the inventory whether the repository is inside its budget.
    pub fn child_worktrees_available(
        db: &Connection,
        namespace_root: &Path,
        task_worktree: &Path,
    ) -> bool {
        if namespace_root.parent().is_none() {
            return false;
        }
        worktree_registry::has_capacity(
            db,
            &task_worktree.to_string_lossy(),
            &WorktreeRetention::default(),
        )
        .unwrap_or(true)
    }

    /// Check a PR head branch out into a task worktree of its own and register
    /// it as a new workspace node. The source workspace is never mutated: the
    /// worktree is cut beside it under the worktrees namespace, and repeating
    /// the checkout reuses the node a previous call registered instead of
    /// stacking duplicates.
    ///
    /// Takes the db mutex rather than a held connection so the network fetch
    /// and worktree creation run outside any database lock.
    pub fn checkout_pull_request(
        db: &std::sync::Mutex<Connection>,
        namespace_root: &Path,
        source_repo: &Path,
        remote: &str,
        number: u64,
        head_branch: &str,
        title: &str,
        project_id: Option<&str>,
    ) -> Result<PullRequestCheckout, BridgeError> {
        let slug = {
            let value = git::slug(head_branch);
            if value.is_empty() { "branch".to_owned() } else { value }
        };
        let path = namespace_root
            .join("github")
            .join(format!("pr-{number}-{slug}"));
        let path_text = path.to_string_lossy().to_string();

        let existing: Option<String> = db.lock().unwrap().query_row(
            "SELECT id FROM workspaces WHERE path=?1",
            params![path_text],
            |row| row.get(0),
        ).map(Some).or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(BridgeError::from(other)),
        })?;
        if let Some(workspace_id) = existing {
            if !path.is_dir() {
                // The node survived a reclaimed tree (cache wipe, manual rm):
                // restore the worktree it names rather than failing the reuse.
                git::fetch_branch(source_repo, remote, head_branch)?;
                git::create_worktree_on_branch(source_repo, &path, head_branch, remote)?;
            }
            worktree_registry::register(
                &db.lock().unwrap(),
                &NewWorktree {
                    kind: worktree_registry::KIND_GITHUB.to_owned(),
                    repo_root: source_repo.to_string_lossy().to_string(),
                    path: path_text.clone(),
                    branch: Some(head_branch.to_owned()),
                    owner_session_id: None,
                    owner_workspace_id: Some(workspace_id.clone()),
                    base_commit: None,
                },
            )?;
            return Ok(PullRequestCheckout {
                workspace_id,
                path,
                branch: head_branch.to_owned(),
                reused: true,
            });
        }

        if path.is_dir() {
            // A worktree without its workspace row (a crash between the two
            // steps). Adopt it only if it really is this PR's branch.
            if git::current_branch(&path).as_deref() != Some(head_branch) {
                return Err(BridgeError::Invalid(format!(
                    "{path_text} exists but is not on {head_branch}; remove it and retry"
                )));
            }
        } else {
            git::fetch_branch(source_repo, remote, head_branch)?;
            git::create_worktree_on_branch(source_repo, &path, head_branch, remote)?;
        }

        let workspace_id = uuid::Uuid::new_v4().to_string();
        let db = db.lock().unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,branch,path,status,created_at) VALUES(?1,?2,?3,?4,?5,'idle',?6)",
            params![
                workspace_id,
                project_id,
                format!("PR #{number}: {title}"),
                head_branch,
                path_text,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        worktree_registry::register(
            &db,
            &NewWorktree {
                kind: worktree_registry::KIND_GITHUB.to_owned(),
                repo_root: source_repo.to_string_lossy().to_string(),
                path: path_text.clone(),
                branch: Some(head_branch.to_owned()),
                owner_session_id: None,
                owner_workspace_id: Some(workspace_id.clone()),
                base_commit: None,
            },
        )?;
        store::event(
            &db,
            "github",
            "pr.checked_out",
            &workspace_id,
            &format!("Checked out PR #{number} ({head_branch}) into {path_text}"),
        )?;
        Ok(PullRequestCheckout {
            workspace_id,
            path,
            branch: head_branch.to_owned(),
            reused: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git").current_dir(cwd).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The cloned-repository shape the git tests use: a bare `origin` holding
    /// `main` plus a PR head branch, and a working clone that plays the source
    /// workspace.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let origin = fixture.path().join("origin.git");
        let seed = fixture.path().join("seed");
        std::fs::create_dir(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        git(&seed, &["config", "user.email", "bridge-test@example.invalid"]);
        git(&seed, &["config", "user.name", "Bridge Test"]);
        git(&seed, &["config", "commit.gpgsign", "false"]);
        std::fs::write(seed.join("shared.txt"), "base\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "base"]);
        git(&seed, &["checkout", "-q", "-b", "feat/pr-head"]);
        std::fs::write(seed.join("feature.txt"), "pr change\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "pr head"]);
        git(&seed, &["checkout", "-q", "main"]);
        git(&seed, &["clone", "-q", "--bare", ".", origin.to_str().unwrap()]);
        let clone = fixture.path().join("clone");
        git(fixture.path(), &["clone", "-q", origin.to_str().unwrap(), clone.to_str().unwrap()]);
        git(&clone, &["config", "user.email", "bridge-test@example.invalid"]);
        git(&clone, &["config", "user.name", "Bridge Test"]);
        git(&clone, &["config", "commit.gpgsign", "false"]);
        (fixture, clone)
    }

    fn database() -> Mutex<Connection> {
        Mutex::new(crate::store::open(Path::new(":memory:")).unwrap())
    }

    #[test]
    #[cfg(unix)]
    fn a_slow_worker_checkout_releases_the_database_but_reserves_its_path() {
        use std::os::unix::fs::PermissionsExt;
        let (scratch, clone) = fixture();
        let db = database();
        db.lock().unwrap().execute_batch(
            "INSERT INTO workspaces(id,title,status,created_at) VALUES('w','Work','idle','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('child','w','codex','Child','starting','reported');
             INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,retry_count,updated_at)
             VALUES('child','child','starting','implementation','key','pending',0,'now');"
        ).unwrap();
        let entered = scratch.path().join("entered");
        let release = scratch.path().join("release");
        let hook = clone.join(".git/hooks/post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\nwhile ! test -f '{}'; do sleep 0.02; done\n", entered.display(), release.display())).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        let namespace = scratch.path().join("workers");
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| WorktreeCoordinator::prepare_isolated_worker(&db, &namespace, "w", &clone, "main", "child", &["src/**".into()]));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !entered.exists() && std::time::Instant::now() < deadline { std::thread::sleep(std::time::Duration::from_millis(10)); }
            let observed = db.try_lock().ok().and_then(|db| db.query_row("SELECT worktree_path FROM worker_runtime WHERE session_id='child'", [], |row| row.get::<_, Option<String>>(0)).ok().flatten());
            std::fs::write(&release, "continue").unwrap();
            let checkout = handle.join().unwrap().unwrap();
            assert!(entered.exists(), "the slow Git fixture must run");
            assert_eq!(observed, Some(worktree_registry::canonical_key(&checkout.0)), "the DB is available and liveness protects the new path while Git is blocked");
        });
    }

    #[test]
    fn checkout_creates_a_worktree_on_the_head_branch_and_a_new_workspace_node() {
        let (scratch, clone) = fixture();
        let db = database();
        let namespace = scratch.path().join("worktrees");
        let checkout = WorktreeCoordinator::checkout_pull_request(
            &db, &namespace, &clone, "origin", 341, "feat/pr-head", "Ship the head", None,
        )
        .unwrap();
        assert!(!checkout.reused);
        assert_eq!(checkout.branch, "feat/pr-head");
        assert!(checkout.path.starts_with(namespace.join("github")));
        assert_eq!(git::current_branch(&checkout.path).as_deref(), Some("feat/pr-head"));
        assert_eq!(
            std::fs::read_to_string(checkout.path.join("feature.txt")).unwrap(),
            "pr change\n"
        );
        let (title, branch): (String, String) = db.lock().unwrap().query_row(
            "SELECT title, branch FROM workspaces WHERE id=?1",
            params![checkout.workspace_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
        assert_eq!(title, "PR #341: Ship the head");
        assert_eq!(branch, "feat/pr-head");
    }

    #[test]
    fn repeating_the_checkout_reuses_the_node_instead_of_duplicating_it() {
        let (scratch, clone) = fixture();
        let db = database();
        let namespace = scratch.path().join("worktrees");
        let first = WorktreeCoordinator::checkout_pull_request(
            &db, &namespace, &clone, "origin", 341, "feat/pr-head", "Ship the head", None,
        )
        .unwrap();
        let second = WorktreeCoordinator::checkout_pull_request(
            &db, &namespace, &clone, "origin", 341, "feat/pr-head", "Ship the head", None,
        )
        .unwrap();
        assert!(second.reused);
        assert_eq!(second.workspace_id, first.workspace_id);
        assert_eq!(second.path, first.path);
        let rows: i64 = db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM workspaces",
            [],
            |row| row.get(0),
        )
        .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn checkout_never_touches_the_source_worktree() {
        let (scratch, clone) = fixture();
        let db = database();
        std::fs::write(clone.join("shared.txt"), "uncommitted local work\n").unwrap();
        WorktreeCoordinator::checkout_pull_request(
            &db, &scratch.path().join("worktrees"), &clone, "origin", 341,
            "feat/pr-head", "Ship the head", None,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(clone.join("shared.txt")).unwrap(),
            "uncommitted local work\n",
            "the source workspace keeps its dirty state",
        );
        assert_eq!(git::current_branch(&clone).as_deref(), Some("main"));
    }

    #[test]
    fn a_missing_remote_branch_errors_and_creates_nothing() {
        let (scratch, clone) = fixture();
        let db = database();
        let namespace = scratch.path().join("worktrees");
        let error = WorktreeCoordinator::checkout_pull_request(
            &db, &namespace, &clone, "origin", 7, "feat/vanished", "Gone", None,
        )
        .unwrap_err();
        assert!(matches!(error, BridgeError::Git(_)), "unexpected error: {error:?}");
        assert!(!namespace.join("github").join("pr-7-feat-vanished").exists());
        let rows: i64 = db.lock().unwrap().query_row(
            "SELECT COUNT(*) FROM workspaces",
            [],
            |row| row.get(0),
        )
        .unwrap();
        assert_eq!(rows, 0);
    }
}
