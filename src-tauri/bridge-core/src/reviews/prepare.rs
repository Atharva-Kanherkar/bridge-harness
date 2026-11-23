//! Preparing a review: the pull request's head in a detached checkout of its
//! own, and a context folder with what the reviewer needs, so the reviewer
//! reads the PR's real code with no network, no `gh` and no shell.
//!
//! The checkout is `<worktrees>/reviews/pr-<n>-<job12>-<attempt>`: per attempt,
//! so a driver that lost its job can only ever remove its own checkout. It is
//! detached, fetched into Bridge's ref namespace, and registers no workspace,
//! so the person's checkout, branches and sidebar never change.
//!
//! Four layers remove it, so a leak needs all four to fail:
//! 1. the [`Guard`] in [`Prepared`], dropped when the step that holds it ends,
//!    panics included;
//! 2. [`cleanup`], which the runner calls after the review step and on every
//!    terminal transition;
//! 3. [`reap`], at startup, every ten minutes and whenever the budget is full;
//! 4. the budget itself ([`has_capacity`]): a job stays queued rather than cut
//!    a checkout past `maxReviewWorktrees`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use serde_json::json;

use super::steps::{
    CancelToken, Guard, PassOutcome, PassRequest, PostedReview, Prepared, ReviewSteps, StepError, UnavailableSteps,
};
use super::store::{self, JobRow};
use crate::github_surface::GithubSurfaceError;
use crate::runtime::BridgeCore;
use crate::worktree_registry::{self, canonical_key, NewWorktree};
use crate::{git, BridgeError};

/// The largest `diff.patch` a reviewer is handed. Whole files only: a file
/// that does not fit is left out and named in `pr.json`.
pub const MAX_DIFF_BYTES: usize = 400 * 1024;
/// How much of the full patch is read to fill it.
const PATCH_READ_LIMIT: usize = 16 * 1024 * 1024;

/// Where review checkouts and job folders live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `<worktrees>/reviews`: one checkout per job attempt, directly inside.
    pub reviews: PathBuf,
    /// `<data dir>/review-jobs`: one folder per job, outside the worktree
    /// namespace so the inventory never mistakes it for a checkout.
    pub jobs: PathBuf,
}

impl Layout {
    pub fn for_core(core: &BridgeCore) -> Self {
        let data = core.worktrees.parent().map(Path::to_path_buf).unwrap_or_else(|| core.worktrees.clone());
        Self { reviews: core.worktrees.join("reviews"), jobs: data.join("review-jobs") }
    }

    pub fn checkout(&self, job: &JobRow) -> PathBuf {
        self.reviews.join(format!("pr-{}-{}-{}", job.number, job_tag(&job.id), job.attempts.max(1)))
    }

    pub fn job_dir(&self, job_id: &str) -> PathBuf {
        self.jobs.join(job_id)
    }
}

/// The first 12 hex digits of a job id: enough to tell jobs apart in a name.
pub fn job_tag(job_id: &str) -> String {
    job_id.chars().filter(char::is_ascii_hexdigit).take(12).collect()
}

/// Whether a checkout name belongs to the job tagged `tag`.
fn belongs_to(name: &str, tag: &str) -> bool {
    name.starts_with("pr-") && name.contains(&format!("-{tag}-"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareConfig {
    pub max_diff_bytes: usize,
}

impl Default for PrepareConfig {
    fn default() -> Self {
        Self { max_diff_bytes: MAX_DIFF_BYTES }
    }
}

/// The steps a host runs: real preparation and cleanup; the reviewer and
/// posting are not in this build yet.
pub struct LiveSteps {
    core: Weak<BridgeCore>,
    config: PrepareConfig,
}

impl LiveSteps {
    pub fn new(core: &Arc<BridgeCore>) -> Self {
        Self::with(core, PrepareConfig::default())
    }

    pub fn with(core: &Arc<BridgeCore>, config: PrepareConfig) -> Self {
        Self { core: Arc::downgrade(core), config }
    }

    fn core(&self) -> Option<Arc<BridgeCore>> {
        self.core.upgrade()
    }
}

impl ReviewSteps for LiveSteps {
    fn prepare(&self, job: &JobRow, cancel: &CancelToken) -> Result<Prepared, StepError> {
        let core = self.core().ok_or_else(|| StepError::Fatal("Bridge is shutting down".into()))?;
        prepare(&core, &self.config, job, cancel)
    }

    fn review(&self, request: PassRequest<'_>, cancel: &CancelToken) -> Result<PassOutcome, StepError> {
        UnavailableSteps.review(request, cancel)
    }

    fn find_posted(&self, job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        UnavailableSteps.find_posted(job)
    }

    fn post(&self, job: &JobRow, cancel: &CancelToken) -> Result<PostedReview, StepError> {
        UnavailableSteps.post(job, cancel)
    }

    fn cleanup(&self, job_id: &str) {
        if let Some(core) = self.core() {
            cleanup(&core, job_id);
        }
    }

    fn has_capacity(&self) -> bool {
        self.core().is_some_and(|core| has_capacity(&core))
    }

    fn reap(&self) {
        if let Some(core) = self.core() {
            if let Err(error) = reap(&core) {
                eprintln!("[reviews] reaping review checkouts failed: {error}");
            }
        }
    }
}

fn surface_error(error: GithubSurfaceError) -> StepError {
    match error {
        // The network or GitHub hiccuped: worth another try.
        GithubSurfaceError::CommandFailed { .. } | GithubSurfaceError::Io(_) => StepError::Transient(error.to_string()),
        // Signed out, no repository, a response Bridge cannot read: trying
        // again changes nothing.
        other => StepError::Fatal(other.to_string()),
    }
}

fn fetch_error(what: &str, error: BridgeError) -> StepError {
    let message = error.to_string();
    if message.contains("couldn't find remote ref") {
        StepError::Fatal(format!("{what} is not on GitHub any more: {message}"))
    } else {
        StepError::Transient(format!("Could not fetch {what}: {message}"))
    }
}

/// The remote whose URL is the pull request's repository, so a fork setup
/// (origin = your fork, upstream = the PR's repository) fetches from the
/// right one. Falls back to the branch's push/upstream remote, then `origin`.
fn remote_for(workspace: &Path, owner: &str, name: &str) -> String {
    let matches = |url: &str| {
        crate::github_surface::parse_remote_url(url)
            .is_some_and(|repo| repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name))
    };
    git::remote_urls(workspace)
        .ok()
        .and_then(|remotes| remotes.into_iter().find(|(_, url)| matches(url)).map(|(remote, _)| remote))
        .unwrap_or_else(|| crate::github_surface::resolve_remote_name(workspace))
}

/// Check the PR head out for `job` and write its context folder. See the
/// module docs.
pub fn prepare(
    core: &Arc<BridgeCore>,
    config: &PrepareConfig,
    job: &JobRow,
    cancel: &CancelToken,
) -> Result<Prepared, StepError> {
    let claim = job.lease_owner.clone().ok_or(StepError::Cancelled)?;
    let workspace = PathBuf::from(core.workspace_path(&job.workspace_id).map_err(|error| StepError::Fatal(error.to_string()))?);
    if !git::is_repository(&workspace) {
        return Err(StepError::Fatal(format!("{} is not a git repository", workspace.display())));
    }
    let remote = remote_for(&workspace, &job.repo_owner, &job.repo_name);
    let number = job.number;

    let head = git::fetch_ref(&workspace, &remote, &format!("refs/pull/{number}/head"), &format!("refs/bridge/reviews/pr-{number}"))
        .map_err(|error| fetch_error(&format!("pull request #{number}"), error))?;
    if cancel.is_cancelled() {
        return Err(StepError::Cancelled);
    }
    let detail = core.github_surface.pr_detail(&workspace, number).map_err(surface_error)?;
    let base_ref = detail.base_branch.clone();
    let base_tip = git::fetch_ref(&workspace, &remote, &format!("refs/heads/{base_ref}"), &format!("refs/bridge/reviews/base-{number}"))
        .map_err(|error| fetch_error(&format!("the base branch {base_ref}"), error))?;
    let merge_base = git::merge_base(&workspace, &base_tip, &head)
        .map_err(|error| StepError::Fatal(format!("The pull request shares no history with {base_ref}: {error}")))?;
    let threads = core.github_surface.pr_review_threads(&workspace, number).map_err(surface_error)?;
    if cancel.is_cancelled() {
        return Err(StepError::Cancelled);
    }

    let layout = Layout::for_core(core);
    let checkout = layout.checkout(job);
    let job_dir = layout.job_dir(&job.id);
    // An earlier try of this very attempt may have left its checkout behind.
    remove_checkout(core, &layout, &checkout);
    git::add_detached_worktree(&workspace, &checkout, &head)
        .map_err(|error| StepError::Transient(format!("Could not check out the pull request: {error}")))?;
    let guard = {
        let core = Arc::downgrade(core);
        let layout = layout.clone();
        let checkout = checkout.clone();
        let job_dir = job_dir.clone();
        Arc::new(Guard::new(move || {
            if let Some(core) = core.upgrade() {
                remove_checkout(&core, &layout, &checkout);
            }
            remove_job_dir(&layout, &job_dir);
        }))
    };
    let recorded = {
        let db = super::lock_db(core);
        let recorded = store::set_worktree(&db, &job.id, &claim, &canonical_key(&checkout)).map_err(fatal)?;
        if recorded {
            worktree_registry::register(
                &db,
                &NewWorktree {
                    kind: worktree_registry::KIND_REVIEW.to_owned(),
                    repo_root: workspace.to_string_lossy().into_owned(),
                    path: checkout.to_string_lossy().into_owned(),
                    branch: None,
                    owner_session_id: None,
                    owner_workspace_id: Some(job.workspace_id.clone()),
                    base_commit: Some(head.clone()),
                },
            )
            .map_err(fatal)?;
        }
        recorded
    };
    if !recorded {
        // The claim lost the job; the guard removes the checkout on return.
        return Err(StepError::Cancelled);
    }

    let context = job_dir.join("context");
    write_context(&workspace, &context, config, job, &detail, &threads, &head, &base_tip, &merge_base)?;
    Ok(Prepared { worktree: Some(checkout), context: Some(context), head_sha: head, guard: Some(guard) })
}

fn fatal(error: BridgeError) -> StepError {
    StepError::Fatal(error.to_string())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), StepError> {
    let text = serde_json::to_vec_pretty(value).map_err(|error| StepError::Fatal(error.to_string()))?;
    std::fs::write(path, text).map_err(|error| StepError::Fatal(format!("Could not write {}: {error}", path.display())))
}

#[allow(clippy::too_many_arguments)]
fn write_context(
    workspace: &Path,
    context: &Path,
    config: &PrepareConfig,
    job: &JobRow,
    detail: &crate::github_surface::PullRequestDetail,
    threads: &[crate::github_surface::ReviewThread],
    head: &str,
    base_tip: &str,
    merge_base: &str,
) -> Result<(), StepError> {
    std::fs::create_dir_all(context).map_err(|error| StepError::Fatal(error.to_string()))?;
    let files = git::diff_files(workspace, merge_base, head).map_err(fatal)?;
    let (patch, omitted) = capped_patch(workspace, merge_base, head, &files, config.max_diff_bytes)?;
    std::fs::write(context.join("diff.patch"), &patch).map_err(|error| StepError::Fatal(error.to_string()))?;
    write_json(
        &context.join("files.json"),
        &json!(files
            .iter()
            .map(|file| json!({
                "path": file.path,
                "previousPath": file.previous_path,
                "status": file.status,
                "additions": file.additions,
                "deletions": file.deletions,
            }))
            .collect::<Vec<_>>()),
    )?;
    write_json(
        &context.join("pr.json"),
        &json!({
            "repoOwner": job.repo_owner,
            "repoName": job.repo_name,
            "number": job.number,
            "title": detail.summary.title,
            "body": detail.body,
            "author": detail.summary.author.as_ref().map(|author| author.login.clone()),
            "url": detail.summary.url,
            "labels": detail.labels.iter().map(|label| label.name.clone()).collect::<Vec<_>>(),
            "isDraft": detail.summary.is_draft,
            "baseRef": detail.base_branch,
            "baseSha": base_tip,
            "mergeBase": merge_base,
            "headRef": detail.summary.head_branch,
            "headSha": head,
            "changedFiles": files.len(),
            "diffBytes": patch.len(),
            "diffTruncated": !omitted.is_empty(),
            "omittedFiles": omitted,
        }),
    )?;
    write_json(&context.join("threads.json"), &serde_json::to_value(threads).map_err(|error| StepError::Fatal(error.to_string()))?)?;
    Ok(())
}

/// The patch, whole files only, within `max` bytes, and the files left out.
/// `git diff` prints files in the order `diff_files` lists them, so the n-th
/// `diff --git` section is the n-th file.
fn capped_patch(
    workspace: &Path,
    merge_base: &str,
    head: &str,
    files: &[git::DiffFile],
    max: usize,
) -> Result<(Vec<u8>, Vec<String>), StepError> {
    let (full, read_truncated) = git::diff_patch(workspace, merge_base, head, PATCH_READ_LIMIT).map_err(fatal)?;
    let mut starts = Vec::new();
    let mut line = 0;
    while line < full.len() {
        if full[line..].starts_with(b"diff --git ") {
            starts.push(line);
        }
        match full[line..].iter().position(|byte| *byte == b'\n') {
            Some(newline) => line += newline + 1,
            None => break,
        }
    }
    let mut sections: Vec<&[u8]> = starts.windows(2).map(|pair| &full[pair[0]..pair[1]]).collect();
    // The last section is cut short when the read was.
    if let (Some(&last), false) = (starts.last(), read_truncated) {
        sections.push(&full[last..]);
    }
    let mut patch = Vec::new();
    let mut omitted = Vec::new();
    for (index, file) in files.iter().enumerate() {
        match sections.get(index) {
            Some(section) if patch.len() + section.len() <= max => patch.extend_from_slice(section),
            _ => omitted.push(file.path.clone()),
        }
    }
    Ok((patch, omitted))
}

/// Remove one review checkout and what git knows of it. Only ever touches a
/// directory directly inside `layout.reviews`.
fn remove_checkout(core: &BridgeCore, layout: &Layout, checkout: &Path) {
    let inside = checkout.parent().is_some_and(|parent| canonical_key(parent) == canonical_key(&layout.reviews));
    if !inside {
        return;
    }
    let repo = git::main_worktree_root(checkout).or_else(|| git::linked_worktree_repo_root(checkout));
    if let Some(repo) = &repo {
        if checkout.exists() {
            if let Err(error) = git::force_remove_worktree(repo, checkout) {
                eprintln!("[reviews] git could not remove {}: {error}", checkout.display());
            }
        }
    }
    // Git could not read it, or refused: it is still Bridge's own checkout.
    if std::fs::symlink_metadata(checkout).is_ok_and(|metadata| metadata.is_dir()) {
        if let Err(error) = std::fs::remove_dir_all(checkout) {
            eprintln!("[reviews] could not remove {}: {error}", checkout.display());
        }
    }
    if let Some(repo) = &repo {
        if let Err(error) = git::prune_worktrees(repo) {
            eprintln!("[reviews] git worktree prune failed in {}: {error}", repo.display());
        }
    }
    let db = super::lock_db(core);
    if let Err(error) = worktree_registry::mark_removed(&db, checkout, "the review finished with it") {
        eprintln!("[reviews] could not record {} as removed: {error}", checkout.display());
    }
}

fn remove_job_dir(layout: &Layout, job_dir: &Path) {
    let inside = job_dir.parent().is_some_and(|parent| canonical_key(parent) == canonical_key(&layout.jobs));
    if inside && job_dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(job_dir) {
            eprintln!("[reviews] could not remove {}: {error}", job_dir.display());
        }
    }
}

fn subdirectories(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

/// Remove every checkout of `job_id`, all attempts, and its job folder.
/// Idempotent.
pub fn cleanup(core: &BridgeCore, job_id: &str) {
    let layout = Layout::for_core(core);
    let tag = job_tag(job_id);
    for checkout in subdirectories(&layout.reviews) {
        if checkout.file_name().is_some_and(|name| belongs_to(&name.to_string_lossy(), &tag)) {
            remove_checkout(core, &layout, &checkout);
        }
    }
    remove_job_dir(&layout, &layout.job_dir(job_id));
}

/// What a reap removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reaped {
    pub checkouts: usize,
    pub job_dirs: usize,
    pub registrations: usize,
}

/// Remove every review checkout and job folder no live job owns, and every
/// registration git still holds for a review checkout that is gone.
pub fn reap(core: &BridgeCore) -> Result<Reaped, BridgeError> {
    let layout = Layout::for_core(core);
    let (live, live_jobs, repos) = {
        let db = super::lock_db(core);
        let live = store::live_worktrees(&db)?;
        let live_jobs = store::live_job_ids(&db)?;
        let mut statement = db.prepare(
            "SELECT path FROM workspaces WHERE COALESCE(path,'')<>'' UNION SELECT path FROM projects WHERE COALESCE(path,'')<>''",
        )?;
        let repos = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        (live, live_jobs, repos)
    };
    let mut reaped = Reaped::default();
    for checkout in subdirectories(&layout.reviews) {
        if !live.contains(&canonical_key(&checkout)) {
            remove_checkout(core, &layout, &checkout);
            reaped.checkouts += 1;
        }
    }
    for job_dir in subdirectories(&layout.jobs) {
        let owned = job_dir.file_name().is_some_and(|name| live_jobs.iter().any(|id| name.to_string_lossy() == *id));
        if !owned {
            remove_job_dir(&layout, &job_dir);
            reaped.job_dirs += 1;
        }
    }
    // Review registrations git still holds that no live job owns, including
    // those whose directory someone else deleted.
    let reviews_key = canonical_key(&layout.reviews);
    for repo in repos.iter().map(PathBuf::from).filter(|repo| repo.is_dir()) {
        let Ok(entries) = git::list_worktrees(&repo) else { continue };
        let stale: Vec<PathBuf> = entries
            .into_iter()
            .map(|entry| entry.path)
            .filter(|path| path.parent().is_some_and(|parent| canonical_key(parent) == reviews_key))
            .filter(|path| !live.contains(&canonical_key(path)))
            .collect();
        if stale.is_empty() {
            continue;
        }
        for path in stale.iter().filter(|path| path.exists()) {
            remove_checkout(core, &layout, path);
        }
        if let Err(error) = git::prune_worktrees(&repo) {
            eprintln!("[reviews] git worktree prune failed in {}: {error}", repo.display());
        }
        reaped.registrations += stale.len();
    }
    Ok(reaped)
}

/// Review checkouts in use: those on disk, plus claimed jobs that have not
/// cut theirs yet.
fn in_use(core: &BridgeCore, layout: &Layout) -> usize {
    let pending = store::claimed_without_worktree(&super::lock_db(core)).unwrap_or(0);
    subdirectories(&layout.reviews).len() + pending
}

/// Whether one more job may cut a checkout without passing
/// `maxReviewWorktrees`. Sweeps first when the budget looks full.
pub fn has_capacity(core: &BridgeCore) -> bool {
    let budget = usize::from(super::settings::load(&super::lock_db(core)).map(|settings| settings.max_review_worktrees).unwrap_or(
        bridge_protocol::messages::DEFAULT_MAX_REVIEW_WORKTREES,
    ));
    let layout = Layout::for_core(core);
    if in_use(core, &layout) < budget {
        return true;
    }
    if let Err(error) = reap(core) {
        eprintln!("[reviews] reaping review checkouts failed: {error}");
    }
    in_use(core, &layout) < budget
}

#[cfg(test)]
#[path = "prepare_tests.rs"]
mod tests;
