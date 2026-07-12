use crate::{policy, BridgeError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub const CITIES: &[&str] = &[
    "Kyoto",
    "Lisbon",
    "Reykjavik",
    "Oslo",
    "Seoul",
    "Tallinn",
    "Nairobi",
    "Prague",
    "Jaipur",
    "Helsinki",
    "Medellin",
    "Valencia",
    "Taipei",
    "Dublin",
    "Zurich",
    "Austin",
    "Kigali",
    "Naples",
    "Vienna",
    "Busan",
];

pub fn validate_repo(path: &Path) -> Result<String, BridgeError> {
    let root = run(path, ["rev-parse", "--show-toplevel"])?;
    let canonical = std::fs::canonicalize(root.trim())?;
    if canonical != std::fs::canonicalize(path)? {
        return Err(BridgeError::Invalid(
            "Choose the repository root, not a subdirectory".into(),
        ));
    }
    Ok(canonical.to_string_lossy().to_string())
}
pub fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true
        }
    }
    out.trim_matches('-').chars().take(42).collect()
}
pub fn create_worktree(repo: &Path, path: &Path, branch: &str) -> Result<(), BridgeError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    run(
        repo,
        [
            "worktree",
            "add",
            "-b",
            branch,
            &path.to_string_lossy(),
            "HEAD",
        ],
    )?;
    Ok(())
}

pub fn remove_worktree(repo: &Path, path: &Path) -> Result<(), BridgeError> {
    run(repo, ["worktree", "remove", &path.to_string_lossy()])?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCheckpoint {
    pub session_id: String,
    pub forest_entry_id: String,
    pub commit: String,
    pub branch: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWriter {
    pub session_id: String,
    pub owned_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerWorktree {
    pub session_id: String,
    pub branch: String,
    pub path: PathBuf,
    pub owned_paths: Vec<String>,
    pub base_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerChangeSet {
    pub base_commit: String,
    pub worker_commit: String,
    pub commits: Vec<String>,
    pub changed_paths: Vec<String>,
    pub patch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationResult {
    Integrated,
    AlreadyIntegrated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceOperation {
    ConversationBranchOnly,
    ConversationBranchWithChildWorktree,
    RestoreRecordedGitCheckpoint,
    ExtractWorkerChanges,
}

impl WorkspaceOperation {
    pub fn mutates_filesystem(self) -> bool {
        matches!(
            self,
            Self::ConversationBranchWithChildWorktree | Self::RestoreRecordedGitCheckpoint
        )
    }
}

/// Marks the explicit conversation-only operation. This deliberately performs no
/// repository lookup or filesystem operation.
pub fn branch_conversation_only() -> WorkspaceOperation {
    WorkspaceOperation::ConversationBranchOnly
}

pub fn owned_paths_overlap(left: &[String], right: &[String]) -> Result<bool, BridgeError> {
    policy::owned_path_sets_overlap(left, right).map_err(BridgeError::Invalid)
}

pub fn authorize_disjoint_writer(
    requested_paths: &[String],
    active_writers: &[ActiveWriter],
) -> Result<(), BridgeError> {
    policy::normalize_owned_paths(requested_paths).map_err(BridgeError::Invalid)?;
    for writer in active_writers {
        if owned_paths_overlap(requested_paths, &writer.owned_paths)? {
            return Err(BridgeError::Invalid(format!(
                "owned paths overlap active writer session {}",
                writer.session_id
            )));
        }
    }
    Ok(())
}

pub fn worker_worktree_path(
    namespace_root: &Path,
    task_worktree: &Path,
    session_id: &str,
) -> Result<PathBuf, BridgeError> {
    let task = task_worktree
        .file_name()
        .and_then(|value| value.to_str())
        .map(slug)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| BridgeError::Invalid("task worktree needs a safe name".into()))?;
    let worker = slug(session_id);
    if worker.is_empty() {
        return Err(BridgeError::Invalid(
            "worker session needs a safe identifier".into(),
        ));
    }
    Ok(namespace_root.join(task).join(worker))
}

pub fn create_child_worktree(
    task_worktree: &Path,
    namespace_root: &Path,
    session_id: &str,
    branch: &str,
    requested_paths: &[String],
    active_writers: &[ActiveWriter],
) -> Result<WorkerWorktree, BridgeError> {
    authorize_disjoint_writer(requested_paths, active_writers)?;
    if branch.trim().is_empty() || branch.starts_with('-') {
        return Err(BridgeError::Invalid("worker branch is invalid".into()));
    }
    let path = worker_worktree_path(namespace_root, task_worktree, session_id)?;
    if path.exists() {
        return Err(BridgeError::Invalid(format!(
            "worker worktree already exists: {}",
            path.display()
        )));
    }
    let base_commit = run(task_worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
    create_worktree(task_worktree, &path, branch)?;
    Ok(WorkerWorktree {
        session_id: session_id.to_owned(),
        branch: branch.to_owned(),
        path,
        owned_paths: policy::normalize_owned_paths(requested_paths)
            .map_err(BridgeError::Invalid)?,
        base_commit,
    })
}

pub fn record_clean_checkpoint(
    worktree: &Path,
    session_id: &str,
    forest_entry_id: &str,
) -> Result<GitCheckpoint, BridgeError> {
    if session_id.trim().is_empty() || forest_entry_id.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "checkpoint requires session and forest entry identifiers".into(),
        ));
    }
    ensure_clean(worktree, "record Git checkpoint")?;
    Ok(GitCheckpoint {
        session_id: session_id.to_owned(),
        forest_entry_id: forest_entry_id.to_owned(),
        commit: run(worktree, ["rev-parse", "HEAD"])?.trim().to_owned(),
        branch: run(worktree, ["branch", "--show-current"])?
            .trim()
            .to_owned(),
        created_at: Utc::now().to_rfc3339(),
    })
}

pub fn extract_worker_changes(
    task_worktree: &Path,
    worker_worktree: &Path,
) -> Result<WorkerChangeSet, BridgeError> {
    ensure_clean(worker_worktree, "extract worker changes")?;
    let task_commit = run(task_worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
    let worker_commit = run(worker_worktree, ["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let base_commit = run(
        task_worktree,
        ["merge-base", task_commit.as_str(), worker_commit.as_str()],
    )?
    .trim()
    .to_owned();
    let range = format!("{base_commit}..{worker_commit}");
    let commits = nonempty_lines(&run(
        task_worktree,
        ["rev-list", "--reverse", range.as_str()],
    )?);
    let changed_paths = nonempty_lines(&run(
        task_worktree,
        ["diff", "--name-only", range.as_str()],
    )?);
    let patch = run(task_worktree, ["diff", "--binary", range.as_str()])?;
    Ok(WorkerChangeSet {
        base_commit,
        worker_commit,
        commits,
        changed_paths,
        patch,
    })
}

pub fn integrate_worker_changes(
    task_worktree: &Path,
    worker_worktree: &Path,
    task_has_active_session: bool,
) -> Result<IntegrationResult, BridgeError> {
    ensure_inactive(task_has_active_session, "integrate worker changes")?;
    ensure_clean(task_worktree, "integrate worker changes")?;
    ensure_clean(worker_worktree, "integrate worker changes")?;
    let worker_commit = run(worker_worktree, ["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    if is_ancestor(task_worktree, &worker_commit, "HEAD")? {
        return Ok(IntegrationResult::AlreadyIntegrated);
    }
    let output = Command::new("git")
        .args(["merge", "--no-ff", "--no-edit", worker_commit.as_str()])
        .current_dir(task_worktree)
        .output()?;
    if !output.status.success() {
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(task_worktree)
            .output();
        return Err(BridgeError::Git(format!(
            "worker integration failed and was aborted: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(IntegrationResult::Integrated)
}

fn is_ancestor(worktree: &Path, ancestor: &str, descendant: &str) -> Result<bool, BridgeError> {
    let output = Command::new("git")
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .current_dir(worktree)
        .output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(BridgeError::Git(format!(
            "could not compare commits: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

pub fn restore_recorded_checkpoint(
    task_worktree: &Path,
    checkpoint: &GitCheckpoint,
    task_has_active_session: bool,
) -> Result<(), BridgeError> {
    ensure_inactive(task_has_active_session, "restore Git checkpoint")?;
    ensure_clean(task_worktree, "restore Git checkpoint")?;
    let object = format!("{}^{{commit}}", checkpoint.commit);
    run(task_worktree, ["cat-file", "-e", object.as_str()])?;
    run(
        task_worktree,
        [
            "restore",
            "--source",
            checkpoint.commit.as_str(),
            "--staged",
            "--worktree",
            "--",
            ".",
        ],
    )?;
    Ok(())
}

pub fn safe_remove_worker_worktree(
    repo: &Path,
    worker_worktree: &Path,
    worker_session_active: bool,
) -> Result<(), BridgeError> {
    ensure_inactive(worker_session_active, "remove worker worktree")?;
    ensure_clean(worker_worktree, "remove worker worktree")?;
    remove_worktree(repo, worker_worktree)
}

fn ensure_inactive(active: bool, operation: &str) -> Result<(), BridgeError> {
    if active {
        return Err(BridgeError::Invalid(format!(
            "cannot {operation} while a session is active"
        )));
    }
    Ok(())
}

fn ensure_clean(worktree: &Path, operation: &str) -> Result<(), BridgeError> {
    if !run(worktree, ["status", "--porcelain"])?.trim().is_empty() {
        return Err(BridgeError::Invalid(format!(
            "cannot {operation} with a dirty worktree"
        )));
    }
    Ok(())
}

fn nonempty_lines(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}
pub fn stats(path: &Path) -> Result<(i64, i64, i64), BridgeError> {
    let porcelain = run(path, ["status", "--porcelain"])?;
    let dirty = porcelain.lines().count() as i64;
    let diff = run(path, ["diff", "--numstat", "HEAD"])?;
    let mut adds = 0;
    let mut dels = 0;
    for l in diff.lines() {
        let p: Vec<_> = l.split('\t').collect();
        if p.len() > 1 {
            adds += p[0].parse::<i64>().unwrap_or(0);
            dels += p[1].parse::<i64>().unwrap_or(0)
        }
    }
    Ok((dirty, adds, dels))
}
fn run<'a, I>(cwd: &Path, args: I) -> Result<String, BridgeError>
where
    I: IntoIterator<Item = &'a str>,
{
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        return Err(BridgeError::Git(
            String::from_utf8_lossy(&output.stderr).trim().into(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}
pub fn workspace_path(base: &Path, project: &str, city: &str) -> PathBuf {
    base.join(slug(project)).join(city)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(cwd: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn repository() -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let repo = fixture.path().join("task");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(
            &repo,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        git(&repo, &["config", "user.name", "Bridge Test"]);
        std::fs::write(repo.join("shared.txt"), "base\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-m", "fixture", "-q"]);
        (fixture, repo)
    }

    fn commit_file(worktree: &Path, path: &str, contents: &str, message: &str) {
        let file = worktree.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(file, contents).unwrap();
        git(worktree, &["add", "."]);
        git(worktree, &["commit", "-m", message, "-q"]);
    }

    fn paths(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn creates_safe_slug() {
        assert_eq!(slug(" Add OAuth / callbacks! "), "add-oauth-callbacks");
    }
    #[test]
    fn city_pool_is_unique() {
        let mut v = CITIES.to_vec();
        v.sort();
        v.dedup();
        assert_eq!(v.len(), CITIES.len());
    }
    #[test]
    fn clean_worktree_can_be_created_and_archived() {
        let (fixture, repo) = repository();
        let worktree = fixture.path().join("Kyoto");
        create_worktree(&repo, &worktree, "bridge/archive-test").unwrap();
        assert!(worktree.exists());
        remove_worktree(&repo, &worktree).unwrap();
        assert!(!worktree.exists());
    }

    #[test]
    fn normalized_overlap_is_shared_with_policy() {
        assert!(owned_paths_overlap(&paths(&["src/**"]), &paths(&["src/auth/store.rs"])).unwrap());
        assert!(owned_paths_overlap(&paths(&["src/auth/*.rs"]), &paths(&["src/auth/**"])).unwrap());
        assert!(!owned_paths_overlap(&paths(&["src/**"]), &paths(&["tests/**"])).unwrap());
        assert!(owned_paths_overlap(&paths(&["../secret"]), &paths(&["src/**"])).is_err());
    }

    #[test]
    fn checkpoint_metadata_requires_clean_tree_and_forest_entry() {
        let (_fixture, repo) = repository();
        let checkpoint = record_clean_checkpoint(&repo, "session-1", "entry-checkpoint").unwrap();
        assert_eq!(checkpoint.session_id, "session-1");
        assert_eq!(checkpoint.forest_entry_id, "entry-checkpoint");
        assert_eq!(checkpoint.commit, git(&repo, &["rev-parse", "HEAD"]));
        assert!(!checkpoint.branch.is_empty());
        assert!(record_clean_checkpoint(&repo, "session-1", "").is_err());

        std::fs::write(repo.join("dirty.txt"), "dirty\n").unwrap();
        assert!(record_clean_checkpoint(&repo, "session-1", "entry-2").is_err());
    }

    #[test]
    fn conversation_branch_only_has_no_filesystem_effect() {
        let (_fixture, repo) = repository();
        let head = git(&repo, &["rev-parse", "HEAD"]);
        let status = git(&repo, &["status", "--porcelain"]);
        let contents = std::fs::read_to_string(repo.join("shared.txt")).unwrap();

        let operation = branch_conversation_only();

        assert_eq!(operation, WorkspaceOperation::ConversationBranchOnly);
        assert!(!operation.mutates_filesystem());
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
        assert_eq!(git(&repo, &["status", "--porcelain"]), status);
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            contents
        );
        assert!(WorkspaceOperation::ConversationBranchWithChildWorktree.mutates_filesystem());
        assert!(WorkspaceOperation::RestoreRecordedGitCheckpoint.mutates_filesystem());
        assert!(!WorkspaceOperation::ExtractWorkerChanges.mutates_filesystem());
    }

    #[test]
    fn disjoint_workers_create_integrate_and_remove_round_trip() {
        let (fixture, repo) = repository();
        let namespace = fixture.path().join("worker-worktrees");
        let worker_a = create_child_worktree(
            &repo,
            &namespace,
            "session-a",
            "bridge/worker-a",
            &paths(&["src/a/**"]),
            &[],
        )
        .unwrap();
        let worker_b = create_child_worktree(
            &repo,
            &namespace,
            "session-b",
            "bridge/worker-b",
            &paths(&["src/b/**"]),
            &[ActiveWriter {
                session_id: worker_a.session_id.clone(),
                owned_paths: worker_a.owned_paths.clone(),
            }],
        )
        .unwrap();
        assert_eq!(worker_a.base_commit, worker_b.base_commit);
        assert_ne!(worker_a.path, worker_b.path);

        commit_file(&worker_a.path, "src/a/result.txt", "worker a\n", "worker a");
        commit_file(&worker_b.path, "src/b/result.txt", "worker b\n", "worker b");
        let extracted_a = extract_worker_changes(&repo, &worker_a.path).unwrap();
        let extracted_b = extract_worker_changes(&repo, &worker_b.path).unwrap();
        assert_eq!(extracted_a.changed_paths, vec!["src/a/result.txt"]);
        assert_eq!(extracted_b.changed_paths, vec!["src/b/result.txt"]);
        assert_eq!(extracted_a.commits.len(), 1);
        assert!(extracted_a.patch.contains("worker a"));

        assert_eq!(
            integrate_worker_changes(&repo, &worker_a.path, false).unwrap(),
            IntegrationResult::Integrated
        );
        assert_eq!(
            integrate_worker_changes(&repo, &worker_b.path, false).unwrap(),
            IntegrationResult::Integrated
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("src/a/result.txt")).unwrap(),
            "worker a\n"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("src/b/result.txt")).unwrap(),
            "worker b\n"
        );

        safe_remove_worker_worktree(&repo, &worker_a.path, false).unwrap();
        safe_remove_worker_worktree(&repo, &worker_b.path, false).unwrap();
        assert!(!worker_a.path.exists());
        assert!(!worker_b.path.exists());
    }

    #[test]
    fn overlapping_workers_are_rejected_before_creation() {
        let (fixture, repo) = repository();
        let namespace = fixture.path().join("worker-worktrees");
        let active = ActiveWriter {
            session_id: "active-writer".into(),
            owned_paths: paths(&["src/auth/**"]),
        };
        let rejected_path = worker_worktree_path(&namespace, &repo, "overlap").unwrap();
        let result = create_child_worktree(
            &repo,
            &namespace,
            "overlap",
            "bridge/overlap",
            &paths(&["src/auth/store.rs"]),
            std::slice::from_ref(&active),
        );
        assert!(result.is_err());
        assert!(!rejected_path.exists());

        let disjoint = create_child_worktree(
            &repo,
            &namespace,
            "disjoint",
            "bridge/disjoint",
            &paths(&["tests/**"]),
            &[active],
        )
        .unwrap();
        safe_remove_worker_worktree(&repo, &disjoint.path, false).unwrap();
    }

    #[test]
    fn dirty_or_active_worker_removal_is_blocked() {
        let (fixture, repo) = repository();
        let worker = create_child_worktree(
            &repo,
            &fixture.path().join("workers"),
            "worker",
            "bridge/removal-worker",
            &paths(&["src/**"]),
            &[],
        )
        .unwrap();
        assert!(safe_remove_worker_worktree(&repo, &worker.path, true).is_err());
        assert!(worker.path.exists());

        std::fs::write(worker.path.join("dirty.txt"), "dirty\n").unwrap();
        assert!(safe_remove_worker_worktree(&repo, &worker.path, false).is_err());
        assert!(worker.path.exists());
        std::fs::remove_file(worker.path.join("dirty.txt")).unwrap();
        safe_remove_worker_worktree(&repo, &worker.path, false).unwrap();
    }

    #[test]
    fn restore_checkpoint_requires_clean_inactive_tree() {
        let (_fixture, repo) = repository();
        let checkpoint = record_clean_checkpoint(&repo, "session", "checkpoint-entry").unwrap();
        commit_file(&repo, "shared.txt", "new state\n", "new state");
        let current_head = git(&repo, &["rev-parse", "HEAD"]);
        assert!(restore_recorded_checkpoint(&repo, &checkpoint, true).is_err());
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            "new state\n"
        );

        std::fs::write(repo.join("dirty.txt"), "dirty\n").unwrap();
        assert!(restore_recorded_checkpoint(&repo, &checkpoint, false).is_err());
        std::fs::remove_file(repo.join("dirty.txt")).unwrap();

        restore_recorded_checkpoint(&repo, &checkpoint, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), current_head);
        assert!(!git(&repo, &["status", "--porcelain"]).is_empty());
    }

    #[test]
    fn conflicting_integration_aborts_cleanly() {
        let (fixture, repo) = repository();
        let worker = create_child_worktree(
            &repo,
            &fixture.path().join("workers"),
            "conflict-worker",
            "bridge/conflict-worker",
            &paths(&["shared.txt"]),
            &[],
        )
        .unwrap();
        commit_file(
            &worker.path,
            "shared.txt",
            "worker version\n",
            "worker edit",
        );
        commit_file(&repo, "shared.txt", "task version\n", "task edit");
        let original_head = git(&repo, &["rev-parse", "HEAD"]);

        assert!(integrate_worker_changes(&repo, &worker.path, false).is_err());
        assert_eq!(git(&repo, &["rev-parse", "HEAD"]), original_head);
        assert_eq!(
            std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
            "task version\n"
        );
        assert!(git(&repo, &["status", "--porcelain"]).is_empty());
        assert!(!repo.join(".git/MERGE_HEAD").exists());
    }
}
