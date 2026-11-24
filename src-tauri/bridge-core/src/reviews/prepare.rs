//! Preparing a review: the pull request's head in a detached checkout of its
//! own, and a context folder with what the reviewer needs, so the reviewer
//! reads the PR's real code with no network, no `gh` and no shell.
//!
//! Everything an attempt makes is the attempt's own: the checkout
//! `<worktrees>/reviews/pr-<n>-<job12>-<attempt>`, the folder
//! `<data>/review-jobs/<job>/<attempt>/`, and the refs
//! `refs/bridge/reviews/<job12>-<attempt>/{head,base}`. A driver that lost its
//! job can only ever remove what it made. The checkout is detached and
//! registers no workspace, so the person's checkout, branches and sidebar
//! never change.
//!
//! Its ownership is reserved (`review_jobs.worktree_path`) before it exists,
//! and reserving, cutting, removing and sweeping all hold one lock, so a sweep
//! never takes a checkout being cut for an orphan.
//!
//! Four layers remove it, so a leak needs all four to fail:
//! 1. the [`Guard`] in [`Prepared`], dropped when the step that holds it ends,
//!    panics included;
//! 2. [`cleanup`], which the runner calls after the review step and on every
//!    terminal transition;
//! 3. [`reap`], at startup, every ten minutes and whenever the budget is full;
//! 4. the budget itself ([`has_capacity`]): a job stays queued rather than cut
//!    a checkout past `maxReviewWorktrees`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};

use serde_json::json;

use super::steps::{
    CancelToken, Guard, Incremental, PassOutcome, PassRequest, PostedReview, Prepared, ReviewSteps, StepError,
};
use super::store::{self, JobRow};
use crate::github_surface::{GithubRepository, GithubSurfaceError};
use crate::runtime::BridgeCore;
use crate::worktree_registry::{self, canonical_key, NewWorktree};
use crate::{git, BridgeError};

/// The largest `diff.patch` a reviewer is handed. Whole files only: a file
/// that does not fit is left out and named in `pr.json`.
pub const MAX_DIFF_BYTES: usize = 400 * 1024;
/// How much of the full patch is read to fill it.
const PATCH_READ_LIMIT: usize = 16 * 1024 * 1024;
/// How a shallow clone is deepened, in turn, until the PR and its base meet.
const DEEPEN_STEPS: [git::Deepen; 4] =
    [git::Deepen::By(64), git::Deepen::By(512), git::Deepen::By(4096), git::Deepen::Unshallow];
/// Where fetched review refs live.
const REF_ROOT: &str = "refs/bridge/reviews/";

/// Where review checkouts and job folders live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `<worktrees>/reviews`: one checkout per job attempt, directly inside.
    pub reviews: PathBuf,
    /// `<data dir>/review-jobs`: one folder per job, one per attempt inside,
    /// outside the worktree namespace so the inventory never mistakes it for a
    /// checkout.
    pub jobs: PathBuf,
}

impl Layout {
    pub fn for_core(core: &BridgeCore) -> Self {
        let data = core.worktrees.parent().map(Path::to_path_buf).unwrap_or_else(|| core.worktrees.clone());
        Self { reviews: core.worktrees.join("reviews"), jobs: data.join("review-jobs") }
    }

    pub fn checkout(&self, job: &JobRow) -> PathBuf {
        self.reviews.join(format!("pr-{}-{}-{}", job.number, job_tag(&job.id), attempt(job)))
    }

    pub fn job_dir(&self, job_id: &str) -> PathBuf {
        self.jobs.join(job_id)
    }

    pub fn attempt_dir(&self, job: &JobRow) -> PathBuf {
        self.job_dir(&job.id).join(attempt(job).to_string())
    }
}

fn attempt(job: &JobRow) -> u32 {
    job.attempts.max(1)
}

/// The refs one attempt fetches into.
fn attempt_refs(job: &JobRow) -> (String, String) {
    let prefix = format!("{REF_ROOT}{}-{}", job_tag(&job.id), attempt(job));
    (format!("{prefix}/head"), format!("{prefix}/base"))
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

/// The steps a host runs: real preparation, the hidden reviewer session,
/// posting and cleanup.
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
        let core = self.core().ok_or_else(|| StepError::Fatal("Bridge is shutting down".into()))?;
        super::session::review_pass(&core, request, cancel)
    }

    fn find_posted(&self, job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        let core = self.core().ok_or_else(|| StepError::Fatal("Bridge is shutting down".into()))?;
        super::post::find_posted(&core, job)
    }

    fn post(&self, job: &JobRow, cancel: &CancelToken) -> Result<PostedReview, StepError> {
        let core = self.core().ok_or_else(|| StepError::Fatal("Bridge is shutting down".into()))?;
        super::post::post(&core, job, cancel)
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
        GithubSurfaceError::RateLimited { ref message, retry_after, .. } => StepError::RateLimited(message.clone(), retry_after),
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

fn fatal(error: BridgeError) -> StepError {
    StepError::Fatal(error.to_string())
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

/// The repository the job was started for, on the host the workspace
/// resolves to: metadata and threads are read from it, never from whatever
/// the workspace points at by the time the job runs.
pub(crate) fn job_repository(core: &BridgeCore, workspace: &Path, job: &JobRow) -> GithubRepository {
    let host = core
        .github_surface
        .resolve_repository(workspace)
        .map(|repository| repository.host)
        .unwrap_or_else(|_| "github.com".to_owned());
    GithubRepository { host, owner: job.repo_owner.clone(), name: job.repo_name.clone() }
}

/// The best common ancestor of the PR and its base. A shallow clone may not
/// hold it yet: deepen the two refs step by step, and only call the PR
/// unrelated once the whole history is there.
fn find_merge_base(workspace: &Path, remote: &str, refspecs: &[String], base: &str, head: &str) -> Result<String, StepError> {
    let mut last = match git::merge_base(workspace, base, head) {
        Ok(found) => return Ok(found),
        Err(error) => error,
    };
    for deepen in DEEPEN_STEPS {
        if !git::is_shallow(workspace) {
            break;
        }
        git::fetch_deepening(workspace, remote, refspecs, deepen)
            .map_err(|error| StepError::Transient(format!("Could not fetch more history: {error}")))?;
        match git::merge_base(workspace, base, head) {
            Ok(found) => return Ok(found),
            Err(error) => last = error,
        }
    }
    Err(StepError::Fatal(format!("The pull request shares no history with its base branch: {last}")))
}

/// What one attempt left: its checkout, its folder (and the job's, once
/// empty) and its refs. Takes the checkout lock.
fn remove_attempt(core: &BridgeCore, layout: &Layout, workspace: &Path, checkout: &Path, attempt_dir: &Path, refs: &[String]) {
    {
        let _checkouts = core.reviews.checkouts();
        remove_checkout(core, layout, checkout);
    }
    remove_dir_inside(&layout.jobs, attempt_dir, 2);
    if let Some(job_dir) = attempt_dir.parent() {
        // Only when empty: another attempt may still be using its own, and
        // then this fails, which is the point.
        drop(std::fs::remove_dir(job_dir));
    }
    for name in refs {
        if let Err(error) = git::delete_ref(workspace, name) {
            eprintln!("[reviews] could not delete {name}: {error}");
        }
    }
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
    let layout = Layout::for_core(core);
    let checkout = layout.checkout(job);
    let attempt_dir = layout.attempt_dir(job);
    let (head_ref, base_ref) = attempt_refs(job);
    let from_ref = head_ref.replace("/head", "/from");
    // From here on, however this ends, what this attempt made goes with it.
    let guard = {
        let core = Arc::downgrade(core);
        let (layout, workspace, checkout, attempt_dir) = (layout.clone(), workspace.clone(), checkout.clone(), attempt_dir.clone());
        let refs = vec![head_ref.clone(), base_ref.clone(), from_ref.clone()];
        Arc::new(Guard::new(move || {
            if let Some(core) = core.upgrade() {
                remove_attempt(&core, &layout, &workspace, &checkout, &attempt_dir, &refs);
            }
        }))
    };

    let repository = job_repository(core, &workspace, job);
    let remote = remote_for(&workspace, &job.repo_owner, &job.repo_name);
    let number = job.number;
    let detail = core.github_surface.pr_detail_in(&workspace, &repository, number).map_err(surface_error)?;
    let head_spec = format!("+refs/pull/{number}/head:{head_ref}");
    let base_spec = format!("+refs/heads/{}:{base_ref}", detail.base_branch);
    let head = git::fetch_ref(&workspace, &remote, &format!("refs/pull/{number}/head"), &head_ref)
        .map_err(|error| fetch_error(&format!("pull request #{number}"), error))?;
    let base_tip = git::fetch_ref(&workspace, &remote, &format!("refs/heads/{}", detail.base_branch), &base_ref)
        .map_err(|error| fetch_error(&format!("the base branch {}", detail.base_branch), error))?;
    let merge_base = find_merge_base(&workspace, &remote, &[head_spec, base_spec], &base_ref, &head_ref)?;
    let incremental = match job.incremental_from_sha.as_deref() {
        Some(from) => settle_incremental(&workspace, &remote, from, &head, &from_ref),
        None => Incremental::Full,
    };
    let threads = core.github_surface.pr_review_threads_in(&workspace, &repository, number).map_err(surface_error)?;
    let prior = if job.incremental_from_sha.is_some() {
        let prior = store::prior_findings(&super::lock_db(core), store::PrKey::of(job), &job.id).map_err(fatal)?;
        before_head(&workspace, &remote, &head, &from_ref, prior)
    } else {
        Vec::new()
    };
    let fixable = prior
        .iter()
        .filter(|(_, relation)| *relation == Relation::Earlier)
        .map(|(finding, _)| finding.id.clone())
        .collect();
    if cancel.is_cancelled() {
        return Err(StepError::Cancelled);
    }

    // Reserve, then cut, under the checkout lock: a sweep that runs meanwhile
    // waits, and then finds the checkout owned.
    let cut = {
        let _checkouts = core.reviews.checkouts();
        cut_checkout(core, &workspace, job, &claim, &checkout, &head)
    };
    cut?;
    {
        let db = super::lock_db(core);
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

    let context = attempt_dir.join("context");
    let facts = ContextFacts {
        job,
        detail: &detail,
        threads: &threads,
        head: &head,
        base_tip: &base_tip,
        merge_base: &merge_base,
        incremental: &incremental,
        prior: &prior,
    };
    write_context(&workspace, &context, config, &facts)?;
    Ok(Prepared { worktree: Some(checkout), context: Some(context), head_sha: head, incremental, fixable, guard: Some(guard) })
}

/// How an earlier review's head stands to the head under review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// The same commit.
    Same,
    /// An ancestor: the commits since may have fixed what it found.
    Earlier,
    /// Neither ancestor nor descendant: the history was rewritten (a rebase or
    /// force-push). What a person decided still holds, but git cannot say it
    /// came first, so nothing of it is marked fixed.
    Rewritten,
}

impl Relation {
    pub fn as_str(self) -> &'static str {
        match self {
            Relation::Same => "same",
            Relation::Earlier => "earlier",
            Relation::Rewritten => "rewritten",
        }
    }
}

/// The earlier findings a review of `head` may learn from, each with how its
/// head stands to `head`. History never runs backward: a finding of a newer
/// head (one `head` is an ancestor of) is left out, so a review of an older
/// head never learns of, or marks fixed, what a newer one found. A head
/// missing here is fetched once; one that still cannot be found is left out,
/// since it cannot be placed.
fn before_head(
    workspace: &Path,
    remote: &str,
    head: &str,
    scratch_ref: &str,
    prior: Vec<store::PriorFinding>,
) -> Vec<(store::PriorFinding, Relation)> {
    let mut placed: HashMap<String, Option<Relation>> = HashMap::new();
    prior
        .into_iter()
        .filter_map(|finding| {
            let relation = *placed.entry(finding.head_sha.clone()).or_insert_with(|| {
                let theirs = finding.head_sha.as_str();
                if theirs == head {
                    return Some(Relation::Same);
                }
                let present = git::resolve_commit(workspace, &format!("{theirs}^{{commit}}")).is_ok()
                    || git::fetch_ref(workspace, remote, theirs, scratch_ref).is_ok();
                if !present {
                    return None;
                }
                match (git::is_ancestor(workspace, theirs, head), git::is_ancestor(workspace, head, theirs)) {
                    (Ok(true), _) => Some(Relation::Earlier),
                    (Ok(false), Ok(false)) => Some(Relation::Rewritten),
                    _ => None,
                }
            });
            relation.map(|relation| (finding, relation))
        })
        .collect()
}

/// Whether `from` can be the start of a re-review of `head`: it is fetched
/// (it may only be in the repository's object store from an earlier review,
/// or still on GitHub) and an ancestor of `head`. A force-push that dropped
/// it from the PR's history falls back to a full review.
fn settle_incremental(workspace: &Path, remote: &str, from: &str, head: &str, from_ref: &str) -> Incremental {
    let present = git::resolve_commit(workspace, &format!("{from}^{{commit}}")).is_ok()
        || git::fetch_ref(workspace, remote, from, from_ref).is_ok();
    if present && git::is_ancestor(workspace, from, head).unwrap_or(false) {
        Incremental::Since(from.to_owned())
    } else {
        Incremental::FellBack(from.to_owned())
    }
}

/// Reserve the checkout for the claim, then cut it. Runs under the checkout
/// lock. A reservation that cannot be cut is given back.
fn cut_checkout(core: &BridgeCore, workspace: &Path, job: &JobRow, claim: &str, checkout: &Path, head: &str) -> Result<(), StepError> {
    let reserved = store::set_worktree(&super::lock_db(core), &job.id, claim, &canonical_key(checkout)).map_err(fatal)?;
    if !reserved {
        return Err(StepError::Cancelled);
    }
    // An earlier try of this very attempt may have left its checkout behind.
    remove_checkout(core, &Layout::for_core(core), checkout);
    if let Err(error) = git::add_detached_worktree(workspace, checkout, head) {
        if let Err(clear) = store::clear_worktree(&super::lock_db(core), &job.id, claim) {
            eprintln!("[reviews] could not give back the reservation of {}: {clear}", job.id);
        }
        return Err(StepError::Transient(format!("Could not check out the pull request: {error}")));
    }
    Ok(())
}

struct ContextFacts<'a> {
    job: &'a JobRow,
    detail: &'a crate::github_surface::PullRequestDetail,
    threads: &'a [crate::github_surface::ReviewThread],
    head: &'a str,
    base_tip: &'a str,
    merge_base: &'a str,
    incremental: &'a Incremental,
    prior: &'a [(store::PriorFinding, Relation)],
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<(), StepError> {
    let text = serde_json::to_vec_pretty(value).map_err(|error| StepError::Fatal(error.to_string()))?;
    std::fs::write(path, text).map_err(|error| StepError::Fatal(format!("Could not write {}: {error}", path.display())))
}

fn write_context(workspace: &Path, context: &Path, config: &PrepareConfig, facts: &ContextFacts<'_>) -> Result<(), StepError> {
    let ContextFacts { job, detail, threads, head, base_tip, merge_base, incremental, prior } = facts;
    std::fs::create_dir_all(context).map_err(|error| StepError::Fatal(error.to_string()))?;
    let files = git::diff_files(workspace, merge_base, head).map_err(fatal)?;
    let (patch, omitted) = capped_patch(workspace, merge_base, head, &files, config.max_diff_bytes)?;
    // The new commits of a re-review, under the same cap. What does not fit
    // is named, never dropped in silence.
    let new_commits = match incremental {
        Incremental::Since(from) => {
            let new_files = git::diff_files(workspace, from, head).map_err(fatal)?;
            Some(capped_patch(workspace, from, head, &new_files, config.max_diff_bytes)?)
        }
        _ => None,
    };
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
            "incremental": incremental_json(incremental, head, new_commits.as_ref().map(|(_, omitted)| omitted.as_slice())),
        }),
    )?;
    write_json(&context.join("threads.json"), &serde_json::to_value(threads).map_err(|error| StepError::Fatal(error.to_string()))?)?;
    if let Some((new_patch, _)) = &new_commits {
        std::fs::write(context.join(INCREMENTAL_PATCH), new_patch).map_err(|error| StepError::Fatal(error.to_string()))?;
    }
    if job.incremental_from_sha.is_some() {
        write_json(&context.join(PRIOR_FINDINGS), &prior_findings_json(prior, threads))?;
    }
    write_conventions(workspace, &context.join(super::session::CONVENTIONS_DIR), base_tip)
}

/// The commits since the earlier review, when the job is a re-review.
pub const INCREMENTAL_PATCH: &str = "incremental.patch";
/// What earlier reviews of the PR said and what became of it.
pub const PRIOR_FINDINGS: &str = "prior_findings.json";
/// Longest reply text kept per thread reply in `prior_findings.json`.
const MAX_REPLY_CHARS: usize = 600;

fn incremental_json(incremental: &Incremental, head: &str, omitted: Option<&[String]>) -> serde_json::Value {
    let omitted = omitted.unwrap_or_default();
    match incremental {
        Incremental::Full => serde_json::Value::Null,
        Incremental::Since(from) => json!({
            "from": from, "to": head, "fellBack": false,
            "diffTruncated": !omitted.is_empty(), "omittedFiles": omitted,
        }),
        Incremental::FellBack(from) => json!({ "from": from, "to": head, "fellBack": true }),
    }
}

/// Each earlier finding with its decision, whether it was posted, how its
/// head stands to the one under review, and the state of its thread on
/// GitHub (found by the posted comment's id).
pub fn prior_findings_json(prior: &[(store::PriorFinding, Relation)], threads: &[crate::github_surface::ReviewThread]) -> serde_json::Value {
    json!(prior
        .iter()
        .map(|(finding, relation)| {
            let thread = finding.github_comment_id.as_deref().and_then(|comment| {
                threads.iter().find(|thread| {
                    thread.comments.iter().any(|item| item.database_id.is_some_and(|id| id.to_string() == comment))
                })
            });
            json!({
                "id": finding.id,
                "reviewedHead": finding.head_sha,
                "relation": relation.as_str(),
                "path": finding.path,
                "line": finding.line,
                "startLine": finding.start_line,
                "side": finding.side.as_str(),
                "severity": finding.severity.as_str(),
                "title": finding.title,
                "body": finding.body,
                "fingerprint": finding.fingerprint,
                "decision": finding.decision.as_str(),
                "posted": finding.posted,
                "fixedInSha": finding.fixed_in_sha,
                "thread": thread.map(|thread| json!({
                    "resolved": thread.is_resolved,
                    "outdated": thread.is_outdated,
                    "replies": thread.comments.iter().skip(1).map(|reply| json!({
                        "author": reply.author.as_ref().map(|author| author.login.clone()),
                        "body": reply.body.chars().take(MAX_REPLY_CHARS).collect::<String>(),
                    })).collect::<Vec<_>>(),
                })),
            })
        })
        .collect::<Vec<_>>())
}

/// A convention file past this size is left out rather than read whole.
const MAX_CONVENTION_BYTES: usize = 256 * 1024;

/// The repository's conventions as the base branch has them, flat under
/// `conventions/`. The reviewer reads these, not the checkout's: the PR's
/// copies are part of what is under review, so a PR cannot rewrite the rules
/// it is judged by.
fn write_conventions(workspace: &Path, folder: &Path, base_tip: &str) -> Result<(), StepError> {
    for file in super::session::CONVENTION_FILES {
        let Ok(output) = git::git_command(workspace).args(["cat-file", "blob", &format!("{base_tip}:{file}")]).output() else {
            continue;
        };
        if !output.status.success() || output.stdout.is_empty() || output.stdout.len() > MAX_CONVENTION_BYTES {
            continue;
        }
        std::fs::create_dir_all(folder).map_err(|error| StepError::Fatal(error.to_string()))?;
        std::fs::write(folder.join(super::session::convention_name(file)), &output.stdout)
            .map_err(|error| StepError::Fatal(format!("Could not write the conventions for the reviewer: {error}")))?;
    }
    Ok(())
}

/// The path a `diff --git a/<p> b/<p>` header names, when both sides name the
/// same path (everything but a rename or copy).
fn section_path(section: &[u8]) -> Option<&[u8]> {
    let header = section.split(|byte| *byte == b'\n').next()?;
    let names = header.strip_prefix(b"diff --git ")?;
    // `a/<p> b/<p>`, possibly quoted as `"a/<p>" "b/<p>"`: two equal halves
    // around one space, once the a/ and b/ prefixes are set aside.
    if names.len() % 2 == 0 {
        return None;
    }
    let half = names.len() / 2;
    let (left, right) = (&names[..half], &names[half + 1..]);
    fn strip<'a>(side: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
        side.strip_prefix(prefix).or_else(|| side.strip_prefix(b"\"").and_then(|rest| rest.strip_prefix(prefix)))
    }
    let (left, right) = (strip(left, b"a/")?, strip(right, b"b/")?);
    (left == right).then_some(left)
}

/// The patch, whole files only, within `max` bytes, and the files left out.
///
/// `git diff` prints files in the order `diff_files` lists them, but one file
/// can take several sections (a type change is a deletion and an addition of
/// the same path), so consecutive sections of one path are grouped first and
/// each group is one file.
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
    let mut groups: Vec<Vec<&[u8]>> = Vec::new();
    for section in sections {
        let joins = groups.last().and_then(|group| group.last()).is_some_and(|previous| {
            section_path(previous).is_some_and(|path| section_path(section) == Some(path))
        });
        match groups.last_mut() {
            Some(group) if joins => group.push(section),
            _ => groups.push(vec![section]),
        }
    }
    let mut patch = Vec::new();
    let mut omitted = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let size = groups.get(index).map(|group| group.iter().map(|section| section.len()).sum::<usize>());
        match (groups.get(index), size) {
            (Some(group), Some(size)) if patch.len() + size <= max => {
                for section in group {
                    patch.extend_from_slice(section);
                }
            }
            _ => omitted.push(file.path.clone()),
        }
    }
    Ok((patch, omitted))
}

/// Remove one review checkout and what git knows of it. Only ever touches a
/// directory directly inside `layout.reviews`. The caller holds the checkout
/// lock.
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

/// Remove a directory `depth` levels inside `root`, and nothing else.
fn remove_dir_inside(root: &Path, dir: &Path, depth: usize) {
    let inside = dir.ancestors().nth(depth).is_some_and(|ancestor| canonical_key(ancestor) == canonical_key(root));
    if inside && dir.exists() {
        if let Err(error) = std::fs::remove_dir_all(dir) {
            eprintln!("[reviews] could not remove {}: {error}", dir.display());
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

/// Remove every checkout of `job_id`, all attempts, its job folder and its
/// fetched refs. Idempotent.
pub fn cleanup(core: &BridgeCore, job_id: &str) {
    let layout = Layout::for_core(core);
    let tag = job_tag(job_id);
    {
        let _checkouts = core.reviews.checkouts();
        for checkout in subdirectories(&layout.reviews) {
            if checkout.file_name().is_some_and(|name| belongs_to(&name.to_string_lossy(), &tag)) {
                remove_checkout(core, &layout, &checkout);
            }
        }
    }
    remove_dir_inside(&layout.jobs, &layout.job_dir(job_id), 1);
    let workspace = {
        let db = super::lock_db(core);
        store::get_job(&db, job_id).ok().flatten().map(|job| job.workspace_id)
    }
    .and_then(|workspace_id| core.workspace_path(&workspace_id).ok())
    .map(PathBuf::from);
    if let Some(workspace) = workspace.filter(|workspace| workspace.is_dir()) {
        for name in git::refs_with_prefix(&workspace, &format!("{REF_ROOT}{tag}-")).unwrap_or_default() {
            if let Err(error) = git::delete_ref(&workspace, &name) {
                eprintln!("[reviews] could not delete {name}: {error}");
            }
        }
    }
}

/// What a reap removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reaped {
    pub checkouts: usize,
    pub job_dirs: usize,
    pub registrations: usize,
    pub refs: usize,
}

/// Remove every review checkout, job or attempt folder, git registration and
/// fetched ref that no live job's current attempt owns. Holds the checkout
/// lock throughout, so nothing is cut while it decides.
pub fn reap(core: &BridgeCore) -> Result<Reaped, BridgeError> {
    let _checkouts = core.reviews.checkouts();
    let layout = Layout::for_core(core);
    let (live, attempts, repos) = {
        let db = super::lock_db(core);
        let live: HashSet<String> = store::live_worktrees(&db)?.into_iter().collect();
        let attempts = store::live_attempts(&db)?;
        let mut statement = db.prepare(
            "SELECT path FROM workspaces WHERE COALESCE(path,'')<>'' UNION SELECT path FROM projects WHERE COALESCE(path,'')<>''",
        )?;
        let repos = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        (live, attempts, repos)
    };
    let mut reaped = Reaped::default();
    for checkout in subdirectories(&layout.reviews) {
        if !live.contains(&canonical_key(&checkout)) {
            remove_checkout(core, &layout, &checkout);
            reaped.checkouts += 1;
        }
    }
    for job_dir in subdirectories(&layout.jobs) {
        let name = job_dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        match attempts.iter().find(|(id, _)| *id == name) {
            None => {
                remove_dir_inside(&layout.jobs, &job_dir, 1);
                reaped.job_dirs += 1;
            }
            // A live job: only its current attempt's folder stays.
            Some((_, current)) => {
                for attempt_dir in subdirectories(&job_dir) {
                    if attempt_dir.file_name().is_some_and(|name| name.to_string_lossy() != current.max(&1).to_string()) {
                        remove_dir_inside(&layout.jobs, &attempt_dir, 2);
                        reaped.job_dirs += 1;
                    }
                }
            }
        }
    }
    let live_refs: HashSet<String> =
        attempts.iter().map(|(id, current)| format!("{REF_ROOT}{}-{}/", job_tag(id), current.max(&1))).collect();
    let reviews_key = canonical_key(&layout.reviews);
    for repo in repos.iter().map(PathBuf::from).filter(|repo| repo.is_dir()) {
        for name in git::refs_with_prefix(&repo, REF_ROOT).unwrap_or_default() {
            if !live_refs.iter().any(|prefix| name.starts_with(prefix)) && git::delete_ref(&repo, &name).is_ok() {
                reaped.refs += 1;
            }
        }
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

/// Review checkouts in use: those on disk or reserved by a job preparing or
/// reviewing, plus claimed jobs that have not reserved theirs yet.
fn in_use(core: &BridgeCore, layout: &Layout) -> usize {
    let (reserved, pending) = {
        let db = super::lock_db(core);
        (store::reserved_worktrees(&db).unwrap_or_default(), store::claimed_without_worktree(&db).unwrap_or(0))
    };
    let mut checkouts: HashSet<String> = subdirectories(&layout.reviews).iter().map(|path| canonical_key(path)).collect();
    checkouts.extend(reserved);
    checkouts.len() + pending
}

/// Whether one more job may cut a checkout without passing
/// `maxReviewWorktrees`. Sweeps first when the budget looks full.
pub fn has_capacity(core: &BridgeCore) -> bool {
    let budget = usize::from(
        super::settings::load(&super::lock_db(core))
            .map(|settings| settings.max_review_worktrees)
            .unwrap_or(bridge_protocol::messages::DEFAULT_MAX_REVIEW_WORKTREES),
    );
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
