//! Deterministic, read-only access to GitHub data through the user's `gh` CLI.
//!
//! The surface deliberately owns no credentials. `gh auth status` is the
//! availability boundary, repository identity comes from the workspace's Git
//! configuration, and every later GitHub operation is launched as an argv
//! array rather than through a shell.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Mutex,
    time::{Duration, Instant},
};
use thiserror::Error;

pub const AUTH_REMEDIATION: &str = "gh auth login";
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(15);

const PR_LIST_FIELDS: &str = "number,title,state,isDraft,author,headRefName,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,url";
const PR_DETAIL_FIELDS: &str = "number,title,body,state,isDraft,author,headRefName,baseRefName,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,url";
const PR_CHECK_FIELDS: &str = "name,state,bucket,link,workflow";
const REVIEW_THREADS_QUERY: &str = "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:100){nodes{id,isResolved,isOutdated,path,line,originalLine,comments(first:100){nodes{id,databaseId,author{login},body,createdAt,url,replyTo{id}}}}}}}}";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GithubAvailability {
    Available,
    NotInstalled,
    NotAuthenticated { remediation: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl GithubRepository {
    /// Repository selector accepted by `gh --repo`.
    pub fn selector(&self) -> String {
        if self.host.eq_ignore_ascii_case("github.com") {
            format!("{}/{}", self.owner, self.name)
        } else {
            format!("{}/{}/{}", self.host, self.owner, self.name)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewDecision {
    None,
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mergeability {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckStatus {
    Queued,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CheckConclusion {
    Success,
    Failure,
    Cancelled,
    Skipped,
    Neutral,
    TimedOut,
    ActionRequired,
    StartupFailure,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubActor {
    pub login: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRollup {
    pub total: u32,
    pub queued: u32,
    pub in_progress: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub cancelled: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestSummary {
    pub number: u64,
    pub title: String,
    pub state: PullRequestState,
    pub is_draft: bool,
    pub author: Option<GithubActor>,
    pub head_branch: String,
    pub review_decision: ReviewDecision,
    pub mergeability: Mergeability,
    pub merge_state_status: String,
    pub checks: CheckRollup,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub summary: PullRequestSummary,
    pub body: String,
    pub base_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCheck {
    pub name: String,
    pub status: CheckStatus,
    pub conclusion: Option<CheckConclusion>,
    pub log_url: String,
    pub workflow: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewComment {
    pub id: String,
    pub database_id: Option<u64>,
    pub author: Option<GithubActor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
    pub reply_to_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<u64>,
    pub original_line: Option<u64>,
    pub comments: Vec<ReviewComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Resource {
    PullRequests,
    PullRequest(u64),
    Checks(u64),
    ReviewThreads(u64),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    repository: String,
    resource: Resource,
}

#[derive(Debug, Clone)]
enum CachedResource {
    PullRequests(Vec<PullRequestSummary>),
    PullRequest(PullRequestDetail),
    Checks(Vec<PullRequestCheck>),
    ReviewThreads(Vec<ReviewThread>),
}

#[derive(Debug, Clone)]
struct CacheEntry {
    stored_at: Instant,
    resource: CachedResource,
}

#[derive(Debug, Error)]
pub enum GithubSurfaceError {
    #[error("GitHub CLI is unavailable: {status:?}")]
    Unavailable { status: GithubAvailability },
    #[error("GitHub command {operation} failed: {stderr}")]
    CommandFailed {
        operation: &'static str,
        stderr: String,
    },
    #[error("GitHub returned a malformed {resource} response: {detail}")]
    MalformedResponse {
        resource: &'static str,
        detail: String,
    },
    #[error("could not resolve a GitHub repository for {workspace}: {detail}")]
    RepositoryResolution { workspace: String, detail: String },
    #[error("I/O while invoking GitHub tooling: {0}")]
    Io(#[from] std::io::Error),
}

/// A discovered `gh` installation and its current authentication state.
///
/// The status is retained so normal reads do not rerun `gh auth status` on
/// every panel refresh. Call [`GithubSurface::refresh_availability`] when the
/// user completes `gh auth login` or asks to probe again.
pub struct GithubSurface {
    binary: Option<PathBuf>,
    availability: Mutex<GithubAvailability>,
    cache_ttl: Duration,
    cache: Mutex<HashMap<CacheKey, CacheEntry>>,
}

impl std::fmt::Debug for GithubSurface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GithubSurface")
            .field("binary", &self.binary)
            .field("availability", &self.availability())
            .field("cache_ttl", &self.cache_ttl)
            .finish()
    }
}

impl Default for GithubSurface {
    fn default() -> Self {
        Self::discover()
    }
}

impl GithubSurface {
    pub fn discover() -> Self {
        Self::from_binary(crate::binary::resolve("gh"), DEFAULT_CACHE_TTL)
    }

    pub fn discover_with_ttl(cache_ttl: Duration) -> Self {
        Self::from_binary(crate::binary::resolve("gh"), cache_ttl)
    }

    fn from_binary(binary: Option<PathBuf>, cache_ttl: Duration) -> Self {
        let availability = probe_availability(binary.as_deref());
        Self {
            binary,
            availability: Mutex::new(availability),
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub fn availability(&self) -> GithubAvailability {
        self.availability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn refresh_availability(&self) -> GithubAvailability {
        let refreshed = probe_availability(self.binary.as_deref());
        *self
            .availability
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = refreshed.clone();
        refreshed
    }

    fn require_binary(&self) -> Result<&Path, GithubSurfaceError> {
        let status = self.availability();
        if status != GithubAvailability::Available {
            return Err(GithubSurfaceError::Unavailable { status });
        }
        self.binary
            .as_deref()
            .ok_or(GithubSurfaceError::Unavailable {
                status: GithubAvailability::NotInstalled,
            })
    }

    /// Resolve the repository from the workspace itself; callers cannot supply
    /// an arbitrary `owner/repo` string.
    pub fn resolve_repository(
        &self,
        workspace: &Path,
    ) -> Result<GithubRepository, GithubSurfaceError> {
        ensure_git_repository(workspace)?;

        for remote in branch_remote_candidates(workspace) {
            if let Some(repository) = repository_for_remote(workspace, &remote) {
                return Ok(repository);
            }
        }

        if let Some(repository) = self.default_repository(workspace) {
            return Ok(repository);
        }

        if let Some(repository) = repository_for_remote(workspace, "origin") {
            return Ok(repository);
        }

        Err(GithubSurfaceError::RepositoryResolution {
            workspace: workspace.display().to_string(),
            detail: "no GitHub push/upstream remote, gh default, or origin remote was available"
                .into(),
        })
    }

    fn default_repository(&self, workspace: &Path) -> Option<GithubRepository> {
        let binary = self.require_binary().ok()?;
        let output = Command::new(binary)
            .current_dir(workspace)
            .args(["repo", "set-default", "--view"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_repository_selector(String::from_utf8_lossy(&output.stdout).trim())
    }

    pub fn list_prs(
        &self,
        workspace: &Path,
    ) -> Result<Vec<PullRequestSummary>, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::PullRequests,
        };
        if let Some(CachedResource::PullRequests(pull_requests)) = self.cached(&key) {
            return Ok(pull_requests);
        }
        let bytes = self.run_gh(
            workspace,
            "pr list",
            &[
                "pr".into(),
                "list".into(),
                "--repo".into(),
                repository.selector(),
                "--state".into(),
                "open".into(),
                "--limit".into(),
                "100".into(),
                "--json".into(),
                PR_LIST_FIELDS.into(),
            ],
            false,
        )?;
        let raw: Vec<RawPullRequestSummary> = parse_json("pull-request list", &bytes)?;
        let pull_requests = raw
            .into_iter()
            .map(PullRequestSummary::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        self.store(key, CachedResource::PullRequests(pull_requests.clone()));
        Ok(pull_requests)
    }

    pub fn pr_detail(
        &self,
        workspace: &Path,
        number: u64,
    ) -> Result<PullRequestDetail, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::PullRequest(number),
        };
        if let Some(CachedResource::PullRequest(pull_request)) = self.cached(&key) {
            return Ok(pull_request);
        }
        let bytes = self.run_gh(
            workspace,
            "pr view",
            &[
                "pr".into(),
                "view".into(),
                number.to_string(),
                "--repo".into(),
                repository.selector(),
                "--json".into(),
                PR_DETAIL_FIELDS.into(),
            ],
            false,
        )?;
        let pull_request = PullRequestDetail::try_from(parse_json::<RawPullRequestDetail>(
            "pull-request detail",
            &bytes,
        )?)?;
        self.store(key, CachedResource::PullRequest(pull_request.clone()));
        Ok(pull_request)
    }

    pub fn pr_checks(
        &self,
        workspace: &Path,
        number: u64,
    ) -> Result<Vec<PullRequestCheck>, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::Checks(number),
        };
        if let Some(CachedResource::Checks(checks)) = self.cached(&key) {
            return Ok(checks);
        }
        // `gh pr checks` intentionally exits nonzero when checks are pending or
        // failing. A nonempty JSON document is still a successful read.
        let bytes = self.run_gh(
            workspace,
            "pr checks",
            &[
                "pr".into(),
                "checks".into(),
                number.to_string(),
                "--repo".into(),
                repository.selector(),
                "--json".into(),
                PR_CHECK_FIELDS.into(),
            ],
            true,
        )?;
        let raw: Vec<RawPullRequestCheck> = parse_json("pull-request checks", &bytes)?;
        let checks = raw
            .into_iter()
            .map(PullRequestCheck::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        self.store(key, CachedResource::Checks(checks.clone()));
        Ok(checks)
    }

    pub fn pr_review_threads(
        &self,
        workspace: &Path,
        number: u64,
    ) -> Result<Vec<ReviewThread>, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::ReviewThreads(number),
        };
        if let Some(CachedResource::ReviewThreads(threads)) = self.cached(&key) {
            return Ok(threads);
        }
        let bytes = self.run_gh(
            workspace,
            "review threads",
            &[
                "api".into(),
                "graphql".into(),
                "--hostname".into(),
                repository.host.clone(),
                "-f".into(),
                format!("query={REVIEW_THREADS_QUERY}"),
                "-f".into(),
                format!("owner={}", repository.owner),
                "-f".into(),
                format!("name={}", repository.name),
                "-F".into(),
                format!("number={number}"),
            ],
            false,
        )?;
        let envelope: RawReviewThreadsEnvelope = parse_json("review threads", &bytes)?;
        let pull_request = envelope
            .data
            .repository
            .pull_request
            .ok_or_else(|| malformed("review threads", "pull request was null"))?;
        let threads = pull_request
            .review_threads
            .nodes
            .into_iter()
            .map(ReviewThread::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        self.store(key, CachedResource::ReviewThreads(threads.clone()));
        Ok(threads)
    }

    fn run_gh(
        &self,
        workspace: &Path,
        operation: &'static str,
        args: &[String],
        accept_nonzero_json: bool,
    ) -> Result<Vec<u8>, GithubSurfaceError> {
        let output = Command::new(self.require_binary()?)
            .current_dir(workspace)
            .args(args)
            .output()?;
        if output.status.success() || (accept_nonzero_json && !output.stdout.is_empty()) {
            return Ok(output.stdout);
        }
        Err(GithubSurfaceError::CommandFailed {
            operation,
            stderr: stderr_or_status(&output),
        })
    }

    fn cached(&self, key: &CacheKey) -> Option<CachedResource> {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = cache.get(key)?;
        if entry.stored_at.elapsed() < self.cache_ttl {
            return Some(entry.resource.clone());
        }
        cache.remove(key);
        None
    }

    fn store(&self, key: CacheKey, resource: CachedResource) {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                key,
                CacheEntry {
                    stored_at: Instant::now(),
                    resource,
                },
            );
    }

    #[cfg(test)]
    fn discover_on_path(path: &Path) -> Self {
        let binary = which::which_in("gh", Some(std::ffi::OsString::from(path)), ".").ok();
        Self::from_binary(binary, DEFAULT_CACHE_TTL)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawActor {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCheckRollup {
    #[serde(rename = "__typename")]
    _type_name: Option<String>,
    status: Option<String>,
    conclusion: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequestSummary {
    number: u64,
    title: String,
    state: String,
    is_draft: bool,
    author: Option<RawActor>,
    head_ref_name: String,
    review_decision: String,
    mergeable: String,
    merge_state_status: String,
    status_check_rollup: Vec<RawCheckRollup>,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequestDetail {
    number: u64,
    title: String,
    body: String,
    state: String,
    is_draft: bool,
    author: Option<RawActor>,
    head_ref_name: String,
    base_ref_name: String,
    review_decision: String,
    mergeable: String,
    merge_state_status: String,
    status_check_rollup: Vec<RawCheckRollup>,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequestCheck {
    name: String,
    state: String,
    bucket: String,
    link: String,
    workflow: String,
}

#[derive(Debug, Deserialize)]
struct RawReviewThreadsEnvelope {
    data: RawReviewThreadsData,
}

#[derive(Debug, Deserialize)]
struct RawReviewThreadsData {
    repository: RawReviewThreadsRepository,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReviewThreadsRepository {
    pull_request: Option<RawReviewThreadsPullRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReviewThreadsPullRequest {
    review_threads: RawReviewThreadConnection,
}

#[derive(Debug, Deserialize)]
struct RawReviewThreadConnection {
    nodes: Vec<RawReviewThread>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReviewThread {
    id: String,
    is_resolved: bool,
    is_outdated: bool,
    path: String,
    line: Option<u64>,
    original_line: Option<u64>,
    comments: RawReviewCommentConnection,
}

#[derive(Debug, Deserialize)]
struct RawReviewCommentConnection {
    nodes: Vec<RawReviewComment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReviewComment {
    id: String,
    database_id: Option<u64>,
    author: Option<RawActor>,
    body: String,
    created_at: String,
    url: String,
    reply_to: Option<RawReviewReply>,
}

#[derive(Debug, Deserialize)]
struct RawReviewReply {
    id: String,
}

impl TryFrom<RawPullRequestSummary> for PullRequestSummary {
    type Error = GithubSurfaceError;

    fn try_from(raw: RawPullRequestSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            number: raw.number,
            title: raw.title,
            state: parse_pull_request_state(&raw.state)?,
            is_draft: raw.is_draft,
            author: raw.author.map(|author| GithubActor {
                login: author.login,
            }),
            head_branch: raw.head_ref_name,
            review_decision: parse_review_decision(&raw.review_decision)?,
            mergeability: parse_mergeability(&raw.mergeable)?,
            merge_state_status: raw.merge_state_status,
            checks: normalize_rollup(raw.status_check_rollup)?,
            url: raw.url,
        })
    }
}

impl TryFrom<RawPullRequestDetail> for PullRequestDetail {
    type Error = GithubSurfaceError;

    fn try_from(raw: RawPullRequestDetail) -> Result<Self, Self::Error> {
        Ok(Self {
            summary: PullRequestSummary {
                number: raw.number,
                title: raw.title,
                state: parse_pull_request_state(&raw.state)?,
                is_draft: raw.is_draft,
                author: raw.author.map(|author| GithubActor {
                    login: author.login,
                }),
                head_branch: raw.head_ref_name,
                review_decision: parse_review_decision(&raw.review_decision)?,
                mergeability: parse_mergeability(&raw.mergeable)?,
                merge_state_status: raw.merge_state_status,
                checks: normalize_rollup(raw.status_check_rollup)?,
                url: raw.url,
            },
            body: raw.body,
            base_branch: raw.base_ref_name,
        })
    }
}

impl TryFrom<RawPullRequestCheck> for PullRequestCheck {
    type Error = GithubSurfaceError;

    fn try_from(raw: RawPullRequestCheck) -> Result<Self, Self::Error> {
        let (status, conclusion) = normalize_check_state(&raw.state, &raw.bucket)?;
        Ok(Self {
            name: raw.name,
            status,
            conclusion,
            log_url: raw.link,
            workflow: raw.workflow,
        })
    }
}

impl TryFrom<RawReviewThread> for ReviewThread {
    type Error = GithubSurfaceError;

    fn try_from(raw: RawReviewThread) -> Result<Self, Self::Error> {
        let comments = raw
            .comments
            .nodes
            .into_iter()
            .map(|comment| ReviewComment {
                id: comment.id,
                database_id: comment.database_id,
                author: comment.author.map(|author| GithubActor {
                    login: author.login,
                }),
                body: comment.body,
                created_at: comment.created_at,
                url: comment.url,
                reply_to_id: comment.reply_to.map(|reply| reply.id),
            })
            .collect();
        Ok(Self {
            id: raw.id,
            is_resolved: raw.is_resolved,
            is_outdated: raw.is_outdated,
            path: raw.path,
            line: raw.line,
            original_line: raw.original_line,
            comments,
        })
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(
    resource: &'static str,
    bytes: &[u8],
) -> Result<T, GithubSurfaceError> {
    serde_json::from_slice(bytes).map_err(|error| malformed(resource, error.to_string()))
}

fn malformed(resource: &'static str, detail: impl Into<String>) -> GithubSurfaceError {
    GithubSurfaceError::MalformedResponse {
        resource,
        detail: detail.into(),
    }
}

fn parse_pull_request_state(value: &str) -> Result<PullRequestState, GithubSurfaceError> {
    match value {
        "OPEN" => Ok(PullRequestState::Open),
        "CLOSED" => Ok(PullRequestState::Closed),
        "MERGED" => Ok(PullRequestState::Merged),
        other => Err(malformed(
            "pull request",
            format!("unknown state {other:?}"),
        )),
    }
}

fn parse_review_decision(value: &str) -> Result<ReviewDecision, GithubSurfaceError> {
    match value {
        "" => Ok(ReviewDecision::None),
        "APPROVED" => Ok(ReviewDecision::Approved),
        "CHANGES_REQUESTED" => Ok(ReviewDecision::ChangesRequested),
        "REVIEW_REQUIRED" => Ok(ReviewDecision::ReviewRequired),
        other => Err(malformed(
            "pull request",
            format!("unknown review decision {other:?}"),
        )),
    }
}

fn parse_mergeability(value: &str) -> Result<Mergeability, GithubSurfaceError> {
    match value {
        "MERGEABLE" => Ok(Mergeability::Mergeable),
        "CONFLICTING" => Ok(Mergeability::Conflicting),
        "UNKNOWN" => Ok(Mergeability::Unknown),
        other => Err(malformed(
            "pull request",
            format!("unknown mergeability {other:?}"),
        )),
    }
}

fn normalize_rollup(raw: Vec<RawCheckRollup>) -> Result<CheckRollup, GithubSurfaceError> {
    let mut rollup = CheckRollup::default();
    for check in raw {
        let (status, conclusion) = if let Some(status) = check.status.as_deref() {
            normalize_status_and_conclusion(status, check.conclusion.as_deref())?
        } else if let Some(state) = check.state.as_deref() {
            normalize_check_state(state, "")?
        } else {
            return Err(malformed(
                "pull-request check rollup",
                "check has neither status nor state",
            ));
        };
        rollup.total += 1;
        match (status, conclusion) {
            (CheckStatus::Queued, _) => rollup.queued += 1,
            (CheckStatus::InProgress, _) => rollup.in_progress += 1,
            (CheckStatus::Completed, Some(CheckConclusion::Success)) => rollup.passed += 1,
            (CheckStatus::Completed, Some(CheckConclusion::Skipped | CheckConclusion::Neutral)) => {
                rollup.skipped += 1
            }
            (CheckStatus::Completed, Some(CheckConclusion::Cancelled)) => rollup.cancelled += 1,
            (CheckStatus::Completed, Some(_)) => rollup.failed += 1,
            (CheckStatus::Completed, None) => {
                return Err(malformed(
                    "pull-request check rollup",
                    "completed check has no conclusion",
                ))
            }
        }
    }
    Ok(rollup)
}

fn normalize_status_and_conclusion(
    status: &str,
    conclusion: Option<&str>,
) -> Result<(CheckStatus, Option<CheckConclusion>), GithubSurfaceError> {
    match status {
        "QUEUED" | "PENDING" | "WAITING" | "EXPECTED" => Ok((CheckStatus::Queued, None)),
        "IN_PROGRESS" => Ok((CheckStatus::InProgress, None)),
        "COMPLETED" => {
            let conclusion = conclusion
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    malformed("pull-request check", "completed check has no conclusion")
                })?;
            Ok((CheckStatus::Completed, Some(parse_conclusion(conclusion)?)))
        }
        other => Err(malformed(
            "pull-request check",
            format!("unknown execution status {other:?}"),
        )),
    }
}

fn normalize_check_state(
    state: &str,
    bucket: &str,
) -> Result<(CheckStatus, Option<CheckConclusion>), GithubSurfaceError> {
    match state {
        "QUEUED" | "PENDING" | "WAITING" | "EXPECTED" => Ok((CheckStatus::Queued, None)),
        "IN_PROGRESS" => Ok((CheckStatus::InProgress, None)),
        "SUCCESS" | "PASS" => Ok((CheckStatus::Completed, Some(CheckConclusion::Success))),
        "FAILURE" | "ERROR" | "FAIL" => {
            Ok((CheckStatus::Completed, Some(CheckConclusion::Failure)))
        }
        "CANCEL" | "CANCELLED" | "CANCELED" => {
            Ok((CheckStatus::Completed, Some(CheckConclusion::Cancelled)))
        }
        "SKIPPED" | "SKIPPING" => Ok((CheckStatus::Completed, Some(CheckConclusion::Skipped))),
        "NEUTRAL" => Ok((CheckStatus::Completed, Some(CheckConclusion::Neutral))),
        "TIMED_OUT" => Ok((CheckStatus::Completed, Some(CheckConclusion::TimedOut))),
        "ACTION_REQUIRED" => Ok((
            CheckStatus::Completed,
            Some(CheckConclusion::ActionRequired),
        )),
        "STARTUP_FAILURE" => Ok((
            CheckStatus::Completed,
            Some(CheckConclusion::StartupFailure),
        )),
        "STALE" => Ok((CheckStatus::Completed, Some(CheckConclusion::Stale))),
        other => Err(malformed(
            "pull-request check",
            format!("unknown state {other:?} in bucket {bucket:?}"),
        )),
    }
}

fn parse_conclusion(value: &str) -> Result<CheckConclusion, GithubSurfaceError> {
    match value {
        "SUCCESS" => Ok(CheckConclusion::Success),
        "FAILURE" => Ok(CheckConclusion::Failure),
        "CANCEL" | "CANCELLED" | "CANCELED" => Ok(CheckConclusion::Cancelled),
        "SKIPPED" => Ok(CheckConclusion::Skipped),
        "NEUTRAL" => Ok(CheckConclusion::Neutral),
        "TIMED_OUT" => Ok(CheckConclusion::TimedOut),
        "ACTION_REQUIRED" => Ok(CheckConclusion::ActionRequired),
        "STARTUP_FAILURE" => Ok(CheckConclusion::StartupFailure),
        "STALE" => Ok(CheckConclusion::Stale),
        other => Err(malformed(
            "pull-request check",
            format!("unknown conclusion {other:?}"),
        )),
    }
}

fn probe_availability(binary: Option<&Path>) -> GithubAvailability {
    let Some(binary) = binary else {
        return GithubAvailability::NotInstalled;
    };
    match Command::new(binary).args(["auth", "status"]).output() {
        Ok(output) if output.status.success() => GithubAvailability::Available,
        Ok(_) => GithubAvailability::NotAuthenticated {
            remediation: AUTH_REMEDIATION.into(),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            GithubAvailability::NotInstalled
        }
        Err(_) => GithubAvailability::NotAuthenticated {
            remediation: AUTH_REMEDIATION.into(),
        },
    }
}

fn ensure_git_repository(workspace: &Path) -> Result<(), GithubSurfaceError> {
    let output = run_git(workspace, ["rev-parse", "--git-dir"])?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GithubSurfaceError::RepositoryResolution {
            workspace: workspace.display().to_string(),
            detail: stderr_or_status(&output),
        })
    }
}

/// Git's push destination precedence is branch.pushRemote, then
/// remote.pushDefault, then the branch's upstream remote. Keep that ordering
/// before consulting `gh`'s per-repository default.
fn branch_remote_candidates(workspace: &Path) -> Vec<String> {
    let Some(branch) = git_stdout(workspace, ["symbolic-ref", "--quiet", "--short", "HEAD"]) else {
        return Vec::new();
    };
    let keys = [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".to_owned(),
        format!("branch.{branch}.remote"),
    ];
    let mut remotes = Vec::new();
    for key in keys {
        if let Some(remote) = git_stdout(workspace, ["config", "--get", key.as_str()]) {
            if remote != "." && !remotes.contains(&remote) {
                remotes.push(remote);
            }
        }
    }
    remotes
}

fn repository_for_remote(workspace: &Path, remote: &str) -> Option<GithubRepository> {
    let url = git_stdout(workspace, ["remote", "get-url", "--push", remote])
        .or_else(|| git_stdout(workspace, ["remote", "get-url", remote]))?;
    parse_remote_url(&url)
}

fn run_git<I, S>(workspace: &Path, args: I) -> Result<Output, GithubSurfaceError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Ok(Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()?)
}

fn git_stdout<I, S>(workspace: &Path, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = run_git(workspace, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn stderr_or_status(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!("process exited with {}", output.status)
    } else {
        stderr
    }
}

fn parse_repository_selector(value: &str) -> Option<GithubRepository> {
    let parts = value
        .trim()
        .trim_end_matches(".git")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let (host, owner, name) = match parts.as_slice() {
        [owner, name] => ("github.com", *owner, *name),
        [host, owner, name] => (*host, *owner, *name),
        _ => return None,
    };
    valid_repository_parts(host, owner, name).then(|| GithubRepository {
        host: host.to_owned(),
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn parse_remote_url(value: &str) -> Option<GithubRepository> {
    let value = value.trim().trim_end_matches('/').trim_end_matches(".git");
    if let Some((_, rest)) = value.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        return parse_repository_selector(&format!("{host}/{path}"));
    }

    // scp-style Git URL: git@github.com:owner/repository.git
    if let Some((authority, path)) = value.split_once(':') {
        if authority.contains('@') && !path.starts_with('/') {
            let host = authority.rsplit('@').next()?;
            return parse_repository_selector(&format!("{host}/{path}"));
        }
    }
    None
}

fn valid_repository_parts(host: &str, owner: &str, name: &str) -> bool {
    !host.is_empty()
        && !owner.is_empty()
        && !name.is_empty()
        && ![host, owner, name]
            .iter()
            .any(|part| part.contains(char::is_whitespace) || *part == "." || *part == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};
    use tempfile::TempDir;

    fn fake_gh(authenticated: bool, default_repository: Option<&str>) -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        let binary = directory.path().join("gh");
        let auth_exit = if authenticated { 0 } else { 1 };
        let default = default_repository.unwrap_or("");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/github")
            .canonicalize()
            .unwrap();
        let script = format!(
            "#!/bin/sh\nroot=$(dirname \"$0\")\nprintf '%s\\n' \"$*\" >> \"$root/invocations.log\"\nif [ \"$1 $2\" = \"auth status\" ]; then exit {auth_exit}; fi\nif [ \"$1 $2 $3\" = \"repo set-default --view\" ]; then\n  if [ -n \"{default}\" ]; then printf '%s\\n' '{default}'; exit 0; fi\n  exit 1\nfi\nif [ \"$1 $2\" = \"pr list\" ]; then\n  fixture=prs.json\n  if [ -f \"$root/pr-list-fixture\" ]; then fixture=$(cat \"$root/pr-list-fixture\"); fi\n  cat '{fixtures}/'$fixture\n  exit 0\nfi\nif [ \"$1 $2\" = \"pr view\" ]; then cat '{fixtures}/pr-detail.json'; exit 0; fi\nif [ \"$1 $2\" = \"pr checks\" ]; then cat '{fixtures}/checks.json'; exit 1; fi\nif [ \"$1 $2\" = \"api graphql\" ]; then cat '{fixtures}/review-threads.json'; exit 0; fi\nexit 2\n",
            fixtures = fixtures.display()
        );
        fs::write(&binary, script).unwrap();
        let mut permissions = fs::metadata(&binary).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).unwrap();
        directory
    }

    fn repository_with_origin() -> TempDir {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/fixture/project.git",
            ],
        );
        repository
    }

    fn invocation_count(fake: &TempDir, prefix: &str) -> usize {
        fs::read_to_string(fake.path().join("invocations.log"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with(prefix))
            .count()
    }

    fn git(repository: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repository)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Bridge Test")
            .env("GIT_AUTHOR_EMAIL", "bridge@example.com")
            .env("GIT_COMMITTER_NAME", "Bridge Test")
            .env("GIT_COMMITTER_EMAIL", "bridge@example.com")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        git(directory.path(), &["init", "-b", "main"]);
        fs::write(directory.path().join("README.md"), "fixture\n").unwrap();
        git(directory.path(), &["add", "README.md"]);
        git(directory.path(), &["commit", "-m", "fixture"]);
        directory
    }

    fn expected(owner: &str, name: &str) -> GithubRepository {
        GithubRepository {
            host: "github.com".into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn discovery_reports_available_for_an_authenticated_gh() {
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(surface.availability(), GithubAvailability::Available);
    }

    #[test]
    fn discovery_reports_not_installed_without_gh() {
        let empty_path = tempfile::tempdir().unwrap();
        let surface = GithubSurface::discover_on_path(empty_path.path());
        assert_eq!(surface.availability(), GithubAvailability::NotInstalled);
        assert!(matches!(
            surface.require_binary(),
            Err(GithubSurfaceError::Unavailable {
                status: GithubAvailability::NotInstalled
            })
        ));
    }

    #[test]
    fn discovery_reports_signed_out_with_exact_remediation() {
        let fake = fake_gh(false, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.availability(),
            GithubAvailability::NotAuthenticated {
                remediation: "gh auth login".into()
            }
        );
    }

    #[test]
    fn repository_resolution_follows_push_default_origin_order() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/origin/project.git",
            ],
        );
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "upstream",
                "git@github.com:push/project.git",
            ],
        );
        git(
            repository.path(),
            &["config", "branch.main.pushRemote", "upstream"],
        );
        let fake = fake_gh(true, Some("default/project"));
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("push", "project")
        );

        git(
            repository.path(),
            &["config", "--unset", "branch.main.pushRemote"],
        );
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("default", "project")
        );

        let no_default = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(no_default.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("origin", "project")
        );
    }

    #[test]
    fn upstream_remote_is_used_when_no_push_remote_is_configured() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/upstream/project.git",
            ],
        );
        git(
            repository.path(),
            &["config", "branch.main.remote", "origin"],
        );
        let fake = fake_gh(true, Some("default/project"));
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("upstream", "project")
        );
    }

    #[test]
    fn linked_worktree_resolves_like_its_parent() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@github.com/shared/project.git",
            ],
        );
        let worktree_root = tempfile::tempdir().unwrap();
        let worktree = worktree_root.path().join("linked");
        git(
            repository.path(),
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                worktree.to_str().unwrap(),
            ],
        );
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            surface.resolve_repository(&worktree).unwrap()
        );
    }

    #[test]
    fn remote_url_formats_are_normalized_without_accepting_local_paths() {
        assert_eq!(
            parse_remote_url("https://github.com/owner/repo.git"),
            Some(expected("owner", "repo"))
        );
        assert_eq!(
            parse_remote_url("git@github.com:owner/repo.git"),
            Some(expected("owner", "repo"))
        );
        assert_eq!(
            parse_remote_url("ssh://git@github.example.com/owner/repo.git")
                .unwrap()
                .selector(),
            "github.example.com/owner/repo"
        );
        assert_eq!(parse_remote_url("../local/repo"), None);
    }

    #[test]
    fn unavailable_reads_return_a_typed_error_before_repository_work() {
        let empty_path = tempfile::tempdir().unwrap();
        let surface = GithubSurface::discover_on_path(empty_path.path());
        let error = surface.list_prs(Path::new("/definitely/not/a/repository"));
        assert!(matches!(
            error,
            Err(GithubSurfaceError::Unavailable {
                status: GithubAvailability::NotInstalled
            })
        ));
    }

    #[test]
    fn list_prs_parses_every_interesting_state() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let pull_requests = surface.list_prs(repository.path()).unwrap();
        assert_eq!(pull_requests.len(), 5);
        assert_eq!(pull_requests[0].state, PullRequestState::Open);
        assert_eq!(pull_requests[0].review_decision, ReviewDecision::Approved);
        assert_eq!(pull_requests[0].checks.passed, 1);
        assert!(pull_requests[1].is_draft);
        assert_eq!(pull_requests[1].checks.in_progress, 1);
        assert_eq!(
            pull_requests[2].review_decision,
            ReviewDecision::ChangesRequested
        );
        assert_eq!(pull_requests[3].checks.failed, 1);
        assert_eq!(pull_requests[3].checks.passed, 1);
        assert_eq!(pull_requests[4].mergeability, Mergeability::Conflicting);
        assert_eq!(pull_requests[4].author, None);
    }

    #[test]
    fn pr_detail_parses_body_review_and_mergeability() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let detail = surface.pr_detail(repository.path(), 103).unwrap();
        assert_eq!(detail.summary.number, 103);
        assert_eq!(detail.base_branch, "main");
        assert_eq!(
            detail.summary.review_decision,
            ReviewDecision::ChangesRequested
        );
        assert_eq!(detail.summary.mergeability, Mergeability::Conflicting);
        assert!(detail.body.contains("typed GitHub surface"));
    }

    #[test]
    fn pr_checks_normalize_states_and_log_urls() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let checks = surface.pr_checks(repository.path(), 103).unwrap();
        assert_eq!(checks.len(), 3);
        assert_eq!(checks[0].status, CheckStatus::Queued);
        assert_eq!(checks[0].conclusion, None);
        assert_eq!(checks[1].conclusion, Some(CheckConclusion::Success));
        assert_eq!(checks[2].conclusion, Some(CheckConclusion::Failure));
        assert_eq!(
            checks[2].log_url,
            "https://github.com/fixture/project/actions/runs/3"
        );
        assert_eq!(
            invocation_count(&fake, "pr checks"),
            1,
            "a nonzero checks rollup exit still yielded valid JSON"
        );
    }

    #[test]
    fn pr_review_threads_parse_comments_and_replies() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let threads = surface.pr_review_threads(repository.path(), 103).unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].path, "src/lib.rs");
        assert_eq!(threads[0].line, Some(42));
        assert!(!threads[0].is_resolved);
        assert_eq!(threads[0].comments.len(), 2);
        assert_eq!(
            threads[0].comments[1].reply_to_id.as_deref(),
            Some("PRRC_comment_1")
        );
        assert!(threads[1].is_resolved);
        assert!(threads[1].is_outdated);
    }

    #[test]
    fn malformed_or_partial_json_is_a_typed_error() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("pr-list-fixture"), "malformed.json").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        assert!(matches!(
            surface.list_prs(repository.path()),
            Err(GithubSurfaceError::MalformedResponse { .. })
        ));

        fs::write(fake.path().join("pr-list-fixture"), "partial.json").unwrap();
        assert!(matches!(
            surface.list_prs(repository.path()),
            Err(GithubSurfaceError::MalformedResponse { .. })
        ));
        assert_eq!(
            invocation_count(&fake, "pr list"),
            2,
            "failed reads must not enter the cache"
        );

        let unknown_state = br#"[{"number":1,"title":"Future","state":"FUTURE","isDraft":false,"author":null,"headRefName":"future","reviewDecision":"","mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN","statusCheckRollup":[],"url":"https://example.invalid"}]"#;
        let raw: Vec<RawPullRequestSummary> =
            parse_json("pull-request list", unknown_state).unwrap();
        assert!(matches!(
            PullRequestSummary::try_from(raw.into_iter().next().unwrap()),
            Err(GithubSurfaceError::MalformedResponse { .. })
        ));

        let null_thread =
            br#"{"data":{"repository":{"pullRequest":{"reviewThreads":{"nodes":[null]}}}}}"#;
        assert!(matches!(
            parse_json::<RawReviewThreadsEnvelope>("review threads", null_thread),
            Err(GithubSurfaceError::MalformedResponse { .. })
        ));
    }

    #[test]
    fn same_resource_is_spawned_once_inside_the_ttl() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let first = surface.list_prs(repository.path()).unwrap();
        let second = surface.list_prs(repository.path()).unwrap();
        assert_eq!(first, second);
        assert_eq!(invocation_count(&fake, "pr list"), 1);

        surface.pr_detail(repository.path(), 103).unwrap();
        assert_eq!(invocation_count(&fake, "pr view"), 1);
        assert_eq!(invocation_count(&fake, "pr list"), 1);

        let log = fs::read_to_string(fake.path().join("invocations.log")).unwrap();
        assert!(log.contains(&format!("--json {PR_LIST_FIELDS}")));
        assert!(log.contains(&format!("--json {PR_DETAIL_FIELDS}")));
    }
}
