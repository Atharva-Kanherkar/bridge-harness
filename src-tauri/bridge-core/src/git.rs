use crate::{completion, completion::RiskTier, policy, BridgeError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
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
/// Current branch name for a repo, if resolvable and not detached.
pub fn current_branch(path: &Path) -> Option<String> {
    run(path, ["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value != "HEAD")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBranches {
    pub current: Option<String>,
    pub branches: Vec<String>,
}

/// Local branches available to the workspace checkout. Remote-only refs are
/// intentionally excluded: choosing one would need explicit tracking-branch
/// creation rather than silently inventing local state.
pub fn list_branches(path: &Path) -> Result<WorkspaceBranches, BridgeError> {
    if !is_repository(path) {
        return Err(BridgeError::Invalid(
            "this workspace directory is not a Git repository".into(),
        ));
    }
    let mut branches = nonempty_lines(&run(
        path,
        ["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )?);
    branches.sort();
    branches.dedup();
    Ok(WorkspaceBranches {
        current: current_branch(path),
        branches,
    })
}

/// Switch a clean, inactive workspace to an existing local branch. The branch
/// must come from Git's own ref list, which also prevents option injection.
pub fn checkout_branch(
    worktree: &Path,
    branch: &str,
    session_active: bool,
) -> Result<WorkspaceBranches, BridgeError> {
    ensure_inactive(session_active, "switch branches")?;
    if !is_repository(worktree) {
        return Err(BridgeError::Invalid(
            "this workspace directory is not a Git repository".into(),
        ));
    }
    ensure_clean(worktree, "switch branches")?;
    let available = list_branches(worktree)?;
    if !available.branches.iter().any(|candidate| candidate == branch) {
        return Err(BridgeError::Invalid(format!(
            "branch '{branch}' is not an existing local branch"
        )));
    }
    if available.current.as_deref() != Some(branch) {
        run(worktree, ["switch", "--", branch])?;
    }
    list_branches(worktree)
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

/// One file's working-tree diff against `HEAD`, with the importance signal an
/// importance-first review UI needs to triage it without opening every file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileChange {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
    pub patch: String,
    pub binary: bool,
    pub importance: RiskTier,
    pub labels: Vec<String>,
    /// Lockfiles, generated output, vendored trees: real changes, low review
    /// signal. A UI may collapse these by default, but never omit them.
    pub low_signal: bool,
}

/// The workspace's uncommitted changeset: every path that differs from
/// `HEAD`, tracked or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeset {
    pub base_commit: Option<String>,
    pub files: Vec<WorkspaceFileChange>,
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

/// How far a checkout has drifted from the branch it is meant to build on.
///
/// This is *not* the same question as `RepositoryDivergence`, which compares a
/// conversation entry's saved local stamp with the current local tree: both sides
/// of that comparison can be months behind the default branch and still look
/// "aligned". This measures the checkout against the best available fetched ref
/// for the default branch, and reports how stale that ref itself is, so an
/// offline stale ref is never presented as current truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseBranchDivergence {
    /// The ref compared against, e.g. `origin/main`. `None` when the repository
    /// has no upstream or default branch to compare with.
    pub base_ref: Option<String>,
    pub base_commit: Option<String>,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    /// Age of the compared ref's newest commit, in seconds. A large value means
    /// the local copy of the base branch is itself stale.
    pub ref_age_seconds: Option<i64>,
    pub fetch_attempted: bool,
    pub fetched: bool,
    pub dirty: bool,
    /// Why no comparison was possible, when `base_ref` is `None`.
    pub unavailable_reason: Option<String>,
}

impl BaseBranchDivergence {
    /// How far behind is far enough to be worth interrupting for. One or two
    /// commits behind is normal; dozens means the work is being done against
    /// code nobody else is running.
    pub const WARN_BEHIND: i64 = 20;

    pub fn should_warn(&self) -> bool {
        self.base_ref.is_some() && self.behind >= Self::WARN_BEHIND
    }

    pub fn summary(&self) -> String {
        let Some(base_ref) = &self.base_ref else {
            return self
                .unavailable_reason
                .clone()
                .unwrap_or_else(|| "no base branch could be resolved".into());
        };
        let mut summary = format!(
            "this workspace is {} commit(s) behind and {} ahead of {base_ref}",
            self.behind, self.ahead
        );
        match (self.fetch_attempted, self.fetched) {
            (true, true) => summary.push_str(", measured against a freshly fetched ref"),
            (true, false) => summary.push_str(
                ", measured against the last fetched ref because fetching failed (possibly offline)",
            ),
            (false, _) => summary.push_str(", measured against the last fetched ref"),
        }
        if let Some(age) = self.ref_age_seconds {
            summary.push_str(&format!(
                "; that ref's newest commit is {} day(s) old",
                age / 86_400
            ));
        }
        summary
    }
}

/// Measure `worktree` against its default/upstream branch.
///
/// `allow_fetch` controls whether a network fetch is attempted; when it fails or
/// is skipped, the comparison still happens against the last fetched ref and says
/// so. This never mutates the working tree.
pub fn base_branch_divergence(worktree: &Path, allow_fetch: bool) -> BaseBranchDivergence {
    let unavailable = |reason: &str| BaseBranchDivergence {
        base_ref: None,
        base_commit: None,
        head: None,
        branch: None,
        ahead: 0,
        behind: 0,
        ref_age_seconds: None,
        fetch_attempted: false,
        fetched: false,
        dirty: false,
        unavailable_reason: Some(reason.to_owned()),
    };
    let Ok(head) = run(worktree, ["rev-parse", "HEAD"]) else {
        return unavailable("the workspace has no resolvable Git HEAD");
    };
    let head = head.trim().to_owned();
    let dirty = run(worktree, ["status", "--porcelain"])
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let branch = current_branch(worktree);
    let (fetch_attempted, fetched) = if allow_fetch {
        (true, run(worktree, ["fetch", "--quiet"]).is_ok())
    } else {
        (false, false)
    };
    let Some(base_ref) = resolve_base_ref(worktree) else {
        let mut result = unavailable("no upstream or default branch ref is available to compare against; fetch the remote or set an upstream");
        result.head = Some(head);
        result.branch = branch;
        result.dirty = dirty;
        result.fetch_attempted = fetch_attempted;
        result.fetched = fetched;
        return result;
    };
    let base_commit = run(worktree, ["rev-parse", base_ref.as_str()])
        .ok()
        .map(|value| value.trim().to_owned());
    let range = format!("{base_ref}...HEAD");
    let (behind, ahead) = run(
        worktree,
        ["rev-list", "--left-right", "--count", range.as_str()],
    )
    .ok()
    .and_then(|counts| {
        let mut parts = counts.split_whitespace();
        Some((
            parts.next()?.parse::<i64>().ok()?,
            parts.next()?.parse::<i64>().ok()?,
        ))
    })
    .unwrap_or((0, 0));
    let ref_age_seconds = run(worktree, ["log", "-1", "--format=%ct", base_ref.as_str()])
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .map(|committed| (Utc::now().timestamp() - committed).max(0));
    BaseBranchDivergence {
        base_ref: Some(base_ref),
        base_commit,
        head: Some(head),
        branch,
        ahead,
        behind,
        ref_age_seconds,
        fetch_attempted,
        fetched,
        dirty,
        unavailable_reason: None,
    }
}

/// The "refresh" half of the stale-base choice: fast-forward the workspace onto
/// its base ref.
///
/// Deliberately conservative. A dirty tree, an active session, or any history
/// that is not a pure fast-forward is refused with an explanation instead of
/// rebased or reset — losing uncommitted work to a background warning would be
/// far worse than staying behind.
pub fn fast_forward_to_base(
    worktree: &Path,
    session_active: bool,
) -> Result<BaseBranchDivergence, BridgeError> {
    ensure_inactive(session_active, "refresh the workspace")?;
    if !is_repository(worktree) {
        // Before ensure_clean, whose raw git stderr ("fatal: not a git
        // repository…") is the error a user should never have to read. The
        // workspace the board described is gone or was never a checkout.
        return Err(BridgeError::Invalid(
            "this workspace directory is not a Git repository, so there is nothing to refresh. Re-create the workspace from its repository, or remove it from Work.".into(),
        ));
    }
    ensure_clean(worktree, "refresh the workspace")?;
    let divergence = base_branch_divergence(worktree, true);
    let Some(base_ref) = divergence.base_ref.clone() else {
        return Err(BridgeError::Invalid(
            divergence
                .unavailable_reason
                .unwrap_or_else(|| "no base branch could be resolved".into()),
        ));
    };
    if divergence.behind == 0 {
        return Ok(divergence);
    }
    if divergence.ahead > 0 {
        return Err(BridgeError::Invalid(format!(
            "this workspace has {} local commit(s) that {base_ref} does not, so it cannot be fast-forwarded. Rebase or merge deliberately instead.",
            divergence.ahead
        )));
    }
    run(worktree, ["merge", "--ff-only", base_ref.as_str()])?;
    Ok(base_branch_divergence(worktree, false))
}

/// Best available ref for "the branch this work should build on": the tracked
/// upstream first, then the remote's advertised default branch, then a local
/// conventional default. Only refs that actually exist are returned.
fn resolve_base_ref(worktree: &Path) -> Option<String> {
    let exists = |candidate: &str| {
        run(worktree, ["rev-parse", "--verify", "--quiet", candidate])
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    };
    let upstream = run(worktree, ["rev-parse", "--abbrev-ref", "@{upstream}"])
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(upstream) = upstream.filter(|candidate| exists(candidate)) {
        return Some(upstream);
    }
    let remote_head = run(
        worktree,
        ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .map(|value| value.trim().to_owned())
    .filter(|value| !value.is_empty());
    if let Some(remote_head) = remote_head.filter(|candidate| exists(candidate)) {
        return Some(remote_head);
    }
    ["origin/main", "origin/master", "main", "master"]
        .into_iter()
        .find(|candidate| exists(candidate))
        .map(str::to_owned)
}

/// What the repository actually shows for a worker's checkout: the revision it
/// produced, what it is relative to, and every path it touched — committed or
/// still dirty. This is derived from Git, never from what a worker claimed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryEvidence {
    pub head: String,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    pub commits: Vec<String>,
    pub committed_paths: Vec<String>,
    pub dirty_paths: Vec<String>,
    pub files_changed: i64,
    pub insertions: i64,
    pub deletions: i64,
}

impl RepositoryEvidence {
    /// Union of committed and uncommitted paths, sorted and deduplicated.
    pub fn changed_paths(&self) -> Vec<String> {
        let mut paths = self.committed_paths.clone();
        paths.extend(self.dirty_paths.iter().cloned());
        paths.sort();
        paths.dedup();
        paths
    }

    /// True when Git shows no change at all: no commits past the base and a
    /// clean tree. A `completed` write-mode result with empty evidence is a
    /// claim the repository does not support.
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty() && self.dirty_paths.is_empty() && self.committed_paths.is_empty()
    }

    pub fn dirty(&self) -> bool {
        !self.dirty_paths.is_empty()
    }

    pub fn diffstat(&self) -> String {
        format!(
            "{} file(s) changed, {} insertion(s), {} deletion(s)",
            self.files_changed, self.insertions, self.deletions
        )
    }
}

/// Derive [`RepositoryEvidence`] for a checkout. `base_commit` is the revision
/// the worker started from; when known, commits and the diffstat cover
/// `base..HEAD` plus the working tree, so committed work is not invisible.
pub fn derive_repository_evidence(
    worktree: &Path,
    base_commit: Option<&str>,
) -> Result<RepositoryEvidence, BridgeError> {
    let head = run(worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
    // `--untracked-files=all` matters: the default collapses a new directory to
    // `src/`, which is neither a path the worker reported nor one the owned-path
    // lease can be checked against file by file.
    let dirty_paths = porcelain_paths(&run(
        worktree,
        [
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain",
            "--untracked-files=all",
        ],
    )?);
    let base = base_commit
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| commit_exists(worktree, value))
        .map(str::to_owned);
    let (commits, committed_paths) = match base.as_deref() {
        Some(base) if base != head => {
            let range = format!("{base}..{head}");
            (
                nonempty_lines(&run(worktree, ["rev-list", "--reverse", range.as_str()])?),
                nonempty_lines(&run(
                    worktree,
                    [
                        "-c",
                        "core.quotePath=false",
                        "diff",
                        "--name-only",
                        range.as_str(),
                    ],
                )?),
            )
        }
        _ => (Vec::new(), Vec::new()),
    };
    // Numstat against the base when known, otherwise against HEAD, so the
    // diffstat matches the same range the paths came from.
    let numstat = run(
        worktree,
        ["diff", "--numstat", base.as_deref().unwrap_or("HEAD")],
    )?;
    let mut insertions = 0;
    let mut deletions = 0;
    let mut files = 0;
    for line in numstat.lines().filter(|line| !line.trim().is_empty()) {
        let columns: Vec<_> = line.split('\t').collect();
        if columns.len() > 1 {
            files += 1;
            insertions += columns[0].parse::<i64>().unwrap_or(0);
            deletions += columns[1].parse::<i64>().unwrap_or(0);
        }
    }
    let mut evidence = RepositoryEvidence {
        head,
        branch: current_branch(worktree),
        base_commit: base,
        commits,
        committed_paths,
        dirty_paths,
        files_changed: files,
        insertions,
        deletions,
    };
    // Untracked files never appear in `git diff --numstat`; count them so the
    // file total matches the path list callers compare against `filesChanged`.
    let counted = evidence.changed_paths().len() as i64;
    evidence.files_changed = evidence.files_changed.max(counted);
    Ok(evidence)
}

fn commit_exists(worktree: &Path, commit: &str) -> bool {
    run(
        worktree,
        ["cat-file", "-e", &format!("{commit}^{{commit}}")],
    )
    .is_ok()
}

/// Paths from `git status --porcelain`, including the destination half of a
/// rename (`R old -> new`).
fn porcelain_paths(porcelain: &str) -> Vec<String> {
    let mut paths = porcelain
        .lines()
        .filter(|line| line.len() > 3)
        .map(|line| {
            let path = line[3..].trim();
            path.rsplit(" -> ").next().unwrap_or(path).trim()
        })
        .map(|path| path.trim_matches('"').to_owned())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
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

/// Commit whatever the worker left uncommitted onto its own branch, so its work
/// can be integrated. A coding harness routinely edits without committing, and
/// `integrate_worker_changes` requires a clean worker tree — without this, the
/// normal case would be permanently unadoptable and the only exit would be
/// discarding verified work. Returns the new commit, or `None` if already clean.
pub fn commit_worker_worktree(
    worker_worktree: &Path,
    message: &str,
) -> Result<Option<String>, BridgeError> {
    if run(worker_worktree, ["status", "--porcelain"])?
        .trim()
        .is_empty()
    {
        return Ok(None);
    }
    run(worker_worktree, ["add", "--all", "."])?;
    // `--no-verify` keeps a repository hook from blocking adoption of work the
    // user has already chosen to keep; the checks that matter are the completion
    // gate's, which ran against this same tree.
    run(
        worker_worktree,
        ["commit", "--no-verify", "--no-gpg-sign", "-m", message],
    )?;
    Ok(Some(
        run(worker_worktree, ["rev-parse", "HEAD"])?
            .trim()
            .to_owned(),
    ))
}

/// Would integrating this worker be a pure fast-forward? True when the task
/// worktree's HEAD is already an ancestor of the worker's commit, i.e. the task
/// branch has not advanced since the worker branched.
pub fn integration_is_fast_forward(
    task_worktree: &Path,
    worker_worktree: &Path,
) -> Result<bool, BridgeError> {
    let worker_commit = run(worker_worktree, ["rev-parse", "HEAD"])?
        .trim()
        .to_owned();
    let task_commit = run(task_worktree, ["rev-parse", "HEAD"])?.trim().to_owned();
    is_ancestor(task_worktree, &task_commit, &worker_commit)
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
    let output = git_command(task_worktree)
        .args(["merge", "--no-ff", "--no-edit", worker_commit.as_str()])
        .output()?;
    if !output.status.success() {
        let _ = git_command(task_worktree).args(["merge", "--abort"]).output();
        return Err(BridgeError::Git(format!(
            "worker integration failed and was aborted: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(IntegrationResult::Integrated)
}

fn is_ancestor(worktree: &Path, ancestor: &str, descendant: &str) -> Result<bool, BridgeError> {
    let output = git_command(worktree)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
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

/// Whether git recognizes the directory as a repository at all — the cheapest
/// question to answer before any operation that would otherwise fail with raw
/// stderr deep in its first command.
fn is_repository(path: &Path) -> bool {
    run(path, ["rev-parse", "--git-dir"]).is_ok()
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
/// The workspace's uncommitted changeset — every path that differs from
/// `HEAD`, tracked or not — with per-file importance for an importance-first
/// review UI. Working-tree diff vs `HEAD`, the same scope [`stats`] covers.
pub fn workspace_changeset(path: &Path) -> Result<WorkspaceChangeset, BridgeError> {
    let base_commit = run(path, ["rev-parse", "HEAD"])
        .ok()
        .map(|value| value.trim().to_owned());
    let mut files = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let numstat = run(
        path,
        [
            "-c",
            "core.quotePath=false",
            "diff",
            "--no-renames",
            "--numstat",
            "HEAD",
        ],
    )?;
    for line in numstat.lines().filter(|line| !line.trim().is_empty()) {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() < 3 {
            continue;
        }
        let file_path = columns[2].trim().to_owned();
        let binary = columns[0] == "-" || columns[1] == "-";
        let additions = columns[0].parse::<i64>().unwrap_or(0);
        let deletions = columns[1].parse::<i64>().unwrap_or(0);
        let patch = if binary {
            String::new()
        } else {
            run(
                path,
                [
                    "-c",
                    "core.quotePath=false",
                    "diff",
                    "--no-renames",
                    "HEAD",
                    "--",
                    file_path.as_str(),
                ],
            )
            .unwrap_or_default()
        };
        seen.insert(file_path.clone());
        files.push(workspace_file_change(
            file_path, additions, deletions, patch, binary,
        ));
    }

    let porcelain = run(
        path,
        [
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain",
            "--untracked-files=all",
        ],
    )?;
    for line in porcelain.lines() {
        if line.len() <= 3 || &line[0..2] != "??" {
            continue;
        }
        let file_path = line[3..].trim().trim_matches('"').to_owned();
        if file_path.is_empty() || seen.contains(&file_path) {
            continue;
        }
        let (patch, additions, binary) = untracked_file_patch(path, &file_path);
        files.push(workspace_file_change(
            file_path, additions, 0, patch, binary,
        ));
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(WorkspaceChangeset { base_commit, files })
}

fn workspace_file_change(
    path: String,
    additions: i64,
    deletions: i64,
    patch: String,
    binary: bool,
) -> WorkspaceFileChange {
    let importance = completion::risk_tier_for_path(&path);
    let labels = completion::labels_for_paths(std::slice::from_ref(&path));
    let low_signal = completion::is_low_signal_path(&path);
    WorkspaceFileChange {
        path,
        additions,
        deletions,
        patch,
        binary,
        importance,
        labels,
        low_signal,
    }
}

/// A new, untracked file has nothing in `HEAD` to diff against; synthesize an
/// "entirely added" patch via `git diff --no-index` instead of touching the
/// index (`--intent-to-add` would mutate state this read-only view must not).
fn untracked_file_patch(worktree: &Path, relative_path: &str) -> (String, i64, bool) {
    let full_path = worktree.join(relative_path);
    let bytes = match std::fs::read(&full_path) {
        Ok(bytes) => bytes,
        Err(_) => return (String::new(), 0, true),
    };
    if bytes.contains(&0) {
        return (String::new(), 0, true);
    }
    let additions = String::from_utf8_lossy(&bytes).lines().count() as i64;
    let output = git_command(worktree)
        .args([
            "diff",
            "--no-index",
            "--no-renames",
            "--",
            "/dev/null",
            relative_path,
        ])
        .output();
    let patch = match output {
        // `--no-index` exits 1 when it finds differences, which is the normal
        // case here; only a genuinely failed invocation has no usable status.
        Ok(result) if matches!(result.status.code(), Some(0) | Some(1)) => {
            String::from_utf8_lossy(&result.stdout).into_owned()
        }
        _ => String::new(),
    };
    (patch, additions, false)
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
thread_local! {
    /// How many `git` processes this thread has started. Per-thread rather than
    /// global so a test can assert on it while the rest of the suite runs in
    /// parallel and spawns git of its own.
    static GIT_PROCESSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Every `git` process Bridge starts goes through here, so "this code path runs
/// no git" is a thing a test can measure instead of a thing a comment asserts.
/// See `work::tests` for the read path that depends on it.
pub(crate) fn git_command(cwd: &Path) -> Command {
    GIT_PROCESSES.with(|count| count.set(count.get() + 1));
    let mut command = Command::new("git");
    command.current_dir(cwd);
    command
}

/// How many `git` processes the calling thread has started. Compare two readings
/// around a call to prove it shelled out — or that it did not.
pub fn git_processes_started_on_this_thread() -> u64 {
    GIT_PROCESSES.with(std::cell::Cell::get)
}

fn run<'a, I>(cwd: &Path, args: I) -> Result<String, BridgeError>
where
    I: IntoIterator<Item = &'a str>,
{
    let output = git_command(cwd).args(args).output()?;
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

    #[test]
    fn no_production_git_spawn_bypasses_the_counted_constructor() {
        // The counter in `work` is only as good as this being true, and it is the
        // kind of thing a later helper reintroduces by accident. Walking the
        // crate is cheap and catches the next one.
        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut pending = vec![source_root.clone()];
        while let Some(directory) = pending.pop() {
            for entry in std::fs::read_dir(&directory).expect("the crate source is readable") {
                let path = entry.expect("a readable entry").path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("a readable module");
                // Test scaffolding is allowed to shell out however it likes, so
                // only the text above `#[cfg(test)]` is held to this.
                let production = &source[..source.find("#[cfg(test)]").unwrap_or(source.len())];
                let is_the_funnel = path.file_name().is_some_and(|name| name == "git.rs");
                for (number, line) in production.lines().enumerate() {
                    if !line.contains(r#"Command::new("git")"#) {
                        continue;
                    }
                    // git.rs declares the one permitted spawn.
                    if is_the_funnel && line.contains("let mut command") {
                        continue;
                    }
                    offenders.push(format!(
                        "{}:{}",
                        path.file_name().unwrap().to_string_lossy(),
                        number + 1
                    ));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these spawn git outside git_command, so the store-only counter cannot see them: {offenders:?}"
        );
    }

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
        git(
            &repo,
            &["config", "commit.gpgsign", "false"],
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
    fn lists_and_switches_only_existing_local_branches() {
        let (_fixture, repo) = repository();
        let initial = current_branch(&repo).unwrap();
        git(&repo, &["branch", "feature"]);

        let listed = list_branches(&repo).unwrap();
        assert_eq!(listed.current.as_deref(), Some(initial.as_str()));
        assert!(listed.branches.contains(&initial));
        assert!(listed.branches.contains(&"feature".to_owned()));

        let switched = checkout_branch(&repo, "feature", false).unwrap();
        assert_eq!(switched.current.as_deref(), Some("feature"));
        assert!(checkout_branch(&repo, "--detach", false).is_err());
    }

    #[test]
    fn branch_switch_treats_an_option_shaped_ref_as_a_branch() {
        let (_fixture, repo) = repository();
        git(&repo, &["update-ref", "refs/heads/--detach", "HEAD"]);

        let switched = checkout_branch(&repo, "--detach", false).unwrap();
        assert_eq!(switched.current.as_deref(), Some("--detach"));
    }

    #[test]
    fn branch_switch_refuses_active_or_dirty_workspaces() {
        let (_fixture, repo) = repository();
        git(&repo, &["branch", "feature"]);

        let active = checkout_branch(&repo, "feature", true).unwrap_err();
        assert!(active.to_string().contains("session is active"), "{active}");

        std::fs::write(repo.join("shared.txt"), "dirty\n").unwrap();
        let dirty = checkout_branch(&repo, "feature", false).unwrap_err();
        assert!(dirty.to_string().contains("dirty worktree"), "{dirty}");
        assert_ne!(current_branch(&repo).as_deref(), Some("feature"));
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

    /// Git quotes non-ASCII paths by default (`"src/caf\303\251.rs"`), which would
    /// make every comparison against an owned-path lease fail and downgrade a
    /// correct result. Untracked directories are also collapsed to `dir/` unless
    /// asked for every file.
    #[test]
    fn derived_paths_are_unquoted_and_listed_file_by_file() {
        let (_fixture, repo) = repository();
        let base = git(&repo, &["rev-parse", "HEAD"]);
        std::fs::create_dir_all(repo.join("src/nested")).unwrap();
        std::fs::write(repo.join("src/café.rs").to_str().unwrap(), "unicode\n").unwrap();
        std::fs::write(repo.join("src/nested/deep.rs"), "deep\n").unwrap();

        let dirty = derive_repository_evidence(&repo, Some(&base)).unwrap();
        assert_eq!(
            dirty.dirty_paths,
            vec!["src/café.rs", "src/nested/deep.rs"],
            "untracked files must be listed individually and unquoted"
        );
        assert!(dirty.commits.is_empty());
        assert!(!dirty.is_empty());
        assert!(dirty.dirty());

        commit_file(&repo, "src/committed.rs", "committed\n", "add files");
        let committed = derive_repository_evidence(&repo, Some(&base)).unwrap();
        assert_eq!(committed.commits.len(), 1);
        assert!(committed
            .committed_paths
            .iter()
            .all(|path| !path.contains('\\') && !path.starts_with('"')));
        assert!(committed
            .committed_paths
            .contains(&"src/café.rs".to_owned()));
        assert!(committed
            .changed_paths()
            .contains(&"src/nested/deep.rs".to_owned()));
        assert_eq!(committed.base_commit.as_deref(), Some(base.as_str()));
        assert!(committed.diffstat().contains("insertion(s)"));
    }

    /// A "clone" whose `origin` is a local bare repository, so divergence can be
    /// exercised end to end without a network.
    fn cloned_repository() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let fixture = tempfile::tempdir().unwrap();
        let origin = fixture.path().join("origin.git");
        let seed = fixture.path().join("seed");
        std::fs::create_dir(&seed).unwrap();
        git(&seed, &["init", "-q", "-b", "main"]);
        git(
            &seed,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        git(
            &seed,
            &["config", "commit.gpgsign", "false"],
        );
        git(&seed, &["config", "user.name", "Bridge Test"]);
        std::fs::write(seed.join("shared.txt"), "base\n").unwrap();
        git(&seed, &["add", "."]);
        git(&seed, &["commit", "-q", "-m", "base"]);
        git(
            &seed,
            &["clone", "-q", "--bare", ".", origin.to_str().unwrap()],
        );
        // `seed` was the clone source, so it has no `origin` of its own.
        git(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        let clone = fixture.path().join("clone");
        git(
            fixture.path(),
            &[
                "clone",
                "-q",
                origin.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        git(
            &clone,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        git(
            &clone,
            &["config", "commit.gpgsign", "false"],
        );
        git(&clone, &["config", "user.name", "Bridge Test"]);
        (fixture, clone, seed)
    }

    /// The incident ran 67 commits behind `origin/main`, and the existing
    /// `RepositoryDivergence` could not see it: both sides of that comparison were
    /// equally stale.
    #[test]
    fn divergence_counts_commits_against_the_upstream_default_branch() {
        let (_fixture, clone, seed) = cloned_repository();
        let aligned = base_branch_divergence(&clone, false);
        assert_eq!(aligned.base_ref.as_deref(), Some("origin/main"));
        assert_eq!((aligned.behind, aligned.ahead), (0, 0));
        assert!(!aligned.should_warn());
        assert!(aligned.ref_age_seconds.is_some());

        // Upstream moves well past the workspace.
        for index in 0..BaseBranchDivergence::WARN_BEHIND + 5 {
            commit_file(
                &seed,
                "shared.txt",
                &format!("upstream {index}\n"),
                "upstream",
            );
        }
        git(&seed, &["push", "-q", "origin", "main"]);
        // And the workspace has one local commit of its own.
        commit_file(&clone, "local.txt", "local\n", "local work");

        let behind = base_branch_divergence(&clone, true);
        assert!(behind.fetch_attempted && behind.fetched);
        assert_eq!(behind.behind, BaseBranchDivergence::WARN_BEHIND + 5);
        assert_eq!(behind.ahead, 1);
        assert!(behind.should_warn());
        let summary = behind.summary();
        assert!(summary.contains("commit(s) behind"), "{summary}");
        assert!(summary.contains("origin/main"), "{summary}");
        assert!(summary.contains("freshly fetched"), "{summary}");
    }

    /// An offline check must still report numbers, and must say the ref it used
    /// was not refreshed rather than presenting it as current truth.
    #[test]
    fn a_failed_fetch_is_reported_instead_of_being_presented_as_current() {
        let (_fixture, clone, _seed) = cloned_repository();
        git(
            &clone,
            &[
                "remote",
                "set-url",
                "origin",
                "/bridge/definitely-not-a-remote",
            ],
        );
        let divergence = base_branch_divergence(&clone, true);
        assert!(divergence.fetch_attempted);
        assert!(!divergence.fetched);
        assert_eq!(divergence.base_ref.as_deref(), Some("origin/main"));
        assert!(divergence.summary().contains("fetching failed"));
    }

    #[test]
    fn a_repository_with_no_base_ref_explains_itself_instead_of_reporting_zero() {
        let fixture = tempfile::tempdir().unwrap();
        let repo = fixture.path().join("solo");
        std::fs::create_dir(&repo).unwrap();
        // No remote and no conventionally-named default branch: there is nothing
        // to compare against, and saying "0 behind" would be a false all-clear.
        git(&repo, &["init", "-q", "-b", "bridge/task"]);
        git(
            &repo,
            &["config", "user.email", "bridge-test@example.invalid"],
        );
        git(
            &repo,
            &["config", "commit.gpgsign", "false"],
        );
        git(&repo, &["config", "user.name", "Bridge Test"]);
        std::fs::write(repo.join("only.txt"), "solo\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-q", "-m", "solo"]);
        let divergence = base_branch_divergence(&repo, false);
        assert!(divergence.base_ref.is_none());
        assert!(!divergence.should_warn());
        assert!(divergence
            .unavailable_reason
            .unwrap()
            .contains("no upstream or default branch"));
    }

    #[test]
    fn refresh_fast_forwards_but_never_touches_dirty_active_or_diverged_work() {
        let (_fixture, clone, seed) = cloned_repository();
        commit_file(&seed, "shared.txt", "upstream\n", "upstream");
        git(&seed, &["push", "-q", "origin", "main"]);

        // An active session must never have its checkout moved underneath it.
        assert!(fast_forward_to_base(&clone, true).is_err());
        // Nor may uncommitted work be discarded by a background warning.
        std::fs::write(clone.join("dirty.txt"), "wip\n").unwrap();
        assert!(fast_forward_to_base(&clone, false).is_err());
        std::fs::remove_file(clone.join("dirty.txt")).unwrap();

        let refreshed = fast_forward_to_base(&clone, false).unwrap();
        assert_eq!((refreshed.behind, refreshed.ahead), (0, 0));
        assert_eq!(
            std::fs::read_to_string(clone.join("shared.txt")).unwrap(),
            "upstream\n"
        );

        // Local history that upstream does not have is a deliberate decision, not
        // something to fast-forward away.
        commit_file(&seed, "shared.txt", "upstream two\n", "upstream two");
        git(&seed, &["push", "-q", "origin", "main"]);
        commit_file(&clone, "local.txt", "local\n", "local work");
        let error = fast_forward_to_base(&clone, false).unwrap_err().to_string();
        assert!(error.contains("cannot be fast-forwarded"), "{error}");
        assert!(clone.join("local.txt").exists());
    }

    /// A workspace directory that is not a checkout must be refused with an
    /// explanation, not git's raw stderr (issue #306 showed both a fresh
    /// measurement and "fatal: not a git repository" in one card).
    #[test]
    fn a_fast_forward_refuses_a_directory_that_is_not_a_repository() {
        let fixture = tempfile::tempdir().unwrap();
        let plain = fixture.path().join("not-a-repo");
        std::fs::create_dir(&plain).unwrap();

        let error = fast_forward_to_base(&plain, false).unwrap_err();
        assert!(matches!(error, BridgeError::Invalid(_)), "{error}");
        assert!(
            error.to_string().contains("not a Git repository"),
            "the refusal should name the condition: {error}"
        );
        assert!(
            !error.to_string().contains("fatal:"),
            "raw git stderr must not reach the user: {error}"
        );
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

    #[test]
    fn workspace_changeset_covers_tracked_and_untracked_files_with_importance() {
        let (_fixture, repo) = repository();
        std::fs::write(repo.join("shared.txt"), "base\nedited\n").unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        std::fs::write(repo.join("src/policy.rs"), "fn guard() {}\n").unwrap();
        std::fs::write(repo.join("bun.lock"), "{}\n").unwrap();

        let changeset = workspace_changeset(&repo).unwrap();
        assert!(changeset.base_commit.is_some());
        let by_path = |path: &str| changeset.files.iter().find(|file| file.path == path).unwrap();

        let shared = by_path("shared.txt");
        assert_eq!(shared.additions, 1);
        assert!(shared.patch.contains("edited"));
        assert!(!shared.binary);

        let policy = by_path("src/policy.rs");
        assert_eq!(policy.importance, crate::completion::RiskTier::High);
        assert!(policy.patch.contains("fn guard"));
        assert!(!policy.low_signal);

        let lock = by_path("bun.lock");
        assert!(lock.low_signal);

        let mut sorted_paths: Vec<_> = changeset.files.iter().map(|file| file.path.clone()).collect();
        let mut expected = sorted_paths.clone();
        expected.sort();
        assert_eq!(sorted_paths, expected, "files are sorted by path");
        sorted_paths.dedup();
        assert_eq!(sorted_paths.len(), changeset.files.len(), "no duplicate paths");
    }

    #[test]
    fn workspace_changeset_reports_deletions_against_head() {
        let (_fixture, repo) = repository();
        std::fs::remove_file(repo.join("shared.txt")).unwrap();
        let changeset = workspace_changeset(&repo).unwrap();
        let shared = changeset
            .files
            .iter()
            .find(|file| file.path == "shared.txt")
            .unwrap();
        assert_eq!(shared.deletions, 1);
        assert_eq!(shared.additions, 0);
    }
}
