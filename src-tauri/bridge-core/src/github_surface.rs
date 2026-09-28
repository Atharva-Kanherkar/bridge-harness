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

/// Hard ceiling on any single `gh` invocation. A hung `gh` (credential
/// prompt, stuck network read) otherwise blocks a daemon connection forever —
/// the desktop shell round-robins unrelated invokes onto that connection and
/// the whole app appears to hang.
pub const GH_COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// How many of the newest pull requests get the expensive computed fields.
/// GitHub prices `statusCheckRollup`/`mergeable`/`reviewDecision` per PR. Even
/// successful queries become visibly slow on check-heavy repositories: 25 PRs
/// took six seconds in the reported case, while five kept the read interactive.
/// The cheap list still returns 100 PRs, and opening one loads its full detail.
const PR_ENRICH_LIMIT: &str = "5";
const PR_ENRICH_TIMEOUT: Duration = Duration::from_secs(3);

/// CI completion notifications previously covered the newest 25 pull requests.
/// Keep that wider query off the interactive pane path so reducing UI latency
/// cannot silently reduce the poller's coverage.
const PR_POLL_LIMIT: &str = "25";

/// Cheap identity fields only — reliable at `--limit 100` on any repository.
const PR_LIST_BASE_FIELDS: &str = "number,title,state,isDraft,author,headRefName,url";
const PR_LIST_FIELDS: &str = "number,title,state,isDraft,author,headRefName,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,url";
const PR_DETAIL_FIELDS: &str = "number,title,body,state,isDraft,author,headRefName,baseRefName,reviewDecision,mergeable,mergeStateStatus,statusCheckRollup,url,comments,labels,commits,additions,deletions,changedFiles";
const PR_CHECK_FIELDS: &str = "name,state,bucket,link,workflow";
const PR_HEAD_FIELDS: &str = "headRefOid";
const ISSUE_LIST_FIELDS: &str = "number,title,state,author,labels,createdAt,updatedAt,url";
const ISSUE_DETAIL_FIELDS: &str = "number,title,body,state,author,labels,comments,createdAt,updatedAt,url";
const REPOSITORY_FIELDS: &str = "nameWithOwner,description,visibility,defaultBranchRef,primaryLanguage,url,issues,pullRequests";
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubLabel {
    pub name: String,
    pub color: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubComment {
    pub id: String,
    pub author: Option<GithubActor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
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
pub struct PullRequestCommit {
    pub oid: String,
    pub abbreviated_oid: String,
    pub message_headline: String,
    pub message_body: String,
    pub committed_at: String,
    pub authors: Vec<GithubActor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub summary: PullRequestSummary,
    pub body: String,
    pub base_branch: String,
    pub comments: Vec<GithubComment>,
    pub labels: Vec<GithubLabel>,
    pub commits: Vec<PullRequestCommit>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub state: IssueState,
    pub author: Option<GithubActor>,
    pub labels: Vec<GithubLabel>,
    pub created_at: String,
    pub updated_at: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub summary: IssueSummary,
    pub body: String,
    pub comments: Vec<GithubComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryOverview {
    pub name_with_owner: String,
    pub description: String,
    pub visibility: String,
    pub default_branch: String,
    pub primary_language: Option<String>,
    pub url: String,
    pub open_issues: u64,
    pub open_pull_requests: u64,
    pub labels: Vec<GithubLabel>,
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

/// The merge strategies a repository permits, plus the one Bridge preselects.
///
/// GitHub's REST API exposes which strategies are *allowed* but not a per-repo
/// *default* merge method, so [`MergeConfig::default_strategy`] is Bridge's
/// choice: squash when allowed, else merge, else rebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeStrategies {
    pub merge: bool,
    pub squash: bool,
    pub rebase: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeConfig {
    pub strategies: MergeStrategies,
    pub default_strategy: MergeStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

impl MergeStrategy {
    /// The `gh pr merge` flag that selects this strategy.
    fn flag(self) -> &'static str {
        match self {
            MergeStrategy::Merge => "--merge",
            MergeStrategy::Squash => "--squash",
            MergeStrategy::Rebase => "--rebase",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GithubTarget {
    PullRequest,
    Issue,
}

impl GithubTarget {
    /// The `gh` noun for this target — `gh pr …` or `gh issue …`.
    fn noun(self) -> &'static str {
        match self {
            GithubTarget::PullRequest => "pr",
            GithubTarget::Issue => "issue",
        }
    }

    /// How a confirmation statement names this target.
    pub fn label(self) -> &'static str {
        match self {
            GithubTarget::PullRequest => "PR",
            GithubTarget::Issue => "issue",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LabelOperation {
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateOperation {
    Close,
    Reopen,
}

impl StateOperation {
    /// The `gh pr`/`gh issue` subcommand that performs this transition.
    fn subcommand(self) -> &'static str {
        match self {
            StateOperation::Close => "close",
            StateOperation::Reopen => "reopen",
        }
    }
}

/// One mutating GitHub action. The tag mirrors the protocol payload so the api
/// layer can convert a wire action into this by a JSON round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum GithubAction {
    Merge {
        number: u64,
        strategy: MergeStrategy,
    },
    Review {
        number: u64,
        event: ReviewEvent,
        body: String,
    },
    Reply {
        number: u64,
        // Keep the wire name camelCase; `rename_all` skips struct-variant fields.
        #[serde(rename = "commentId")]
        comment_id: u64,
        body: String,
    },
    Rerun {
        number: u64,
    },
    Label {
        target: GithubTarget,
        number: u64,
        label: String,
        operation: LabelOperation,
    },
    Comment {
        target: GithubTarget,
        number: u64,
        body: String,
    },
    SetState {
        target: GithubTarget,
        number: u64,
        operation: StateOperation,
    },
    Ready {
        number: u64,
    },
}

impl GithubAction {
    /// The pull request this action targets.
    pub fn number(&self) -> u64 {
        match self {
            GithubAction::Merge { number, .. }
            | GithubAction::Review { number, .. }
            | GithubAction::Reply { number, .. }
            | GithubAction::Rerun { number }
            | GithubAction::Label { number, .. }
            | GithubAction::Comment { number, .. }
            | GithubAction::SetState { number, .. }
            | GithubAction::Ready { number } => *number,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Resource {
    PullRequests,
    PollingPullRequests,
    PullRequest(u64),
    PullRequestFiles(u64),
    Checks(u64),
    ReviewThreads(u64),
    Issues,
    Issue(u64),
    RepositoryOverview,
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
    PullRequestFiles(Vec<PullRequestFile>),
    Checks(Vec<PullRequestCheck>),
    ReviewThreads(Vec<ReviewThread>),
    Issues(Vec<IssueSummary>),
    Issue(IssueDetail),
    RepositoryOverview(RepositoryOverview),
}

#[derive(Debug, Clone)]
struct CacheEntry {
    stored_at: Instant,
    resource: CachedResource,
}

/// What [`GithubSurface::connect_repository`] did, beyond succeeding.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedRepository {
    pub repository: GithubRepository,
    pub initialized: bool,
    pub replaced_remote: bool,
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
    command_timeout: Duration,
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

    #[cfg(test)]
    pub(crate) fn unavailable_for_tests() -> Self {
        Self::from_binary(None, DEFAULT_CACHE_TTL)
    }

    fn from_binary(binary: Option<PathBuf>, cache_ttl: Duration) -> Self {
        let availability = probe_availability(binary.as_deref());
        Self {
            binary,
            availability: Mutex::new(availability),
            cache_ttl,
            cache: Mutex::new(HashMap::new()),
            command_timeout: GH_COMMAND_TIMEOUT,
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

    pub fn invalidate_repository(&self, workspace: &Path) {
        if let Ok(repository) = self.resolve_repository(workspace) {
            let selector = repository.selector();
            self.cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .retain(|key, _| key.repository != selector);
        }
    }

    /// Point a workspace at `remote_url` so the GitHub surface can resolve it.
    ///
    /// Two setups reach here, and both are ordinary rather than exceptional: a
    /// folder that was never a git repository, and a repository with no GitHub
    /// remote. The caller validates the URL before this runs — nothing here
    /// interpolates user text into a git flag position.
    ///
    /// The workspace must be the repository root itself, not merely inside
    /// one: a folder nested in someone else's checkout gets its own
    /// repository rather than repointing that checkout's `origin`.
    ///
    /// Returns the resolved repository, so a caller can only report success
    /// once the surface genuinely sees the repository it asked for.
    pub fn connect_repository(
        &self,
        workspace: &Path,
        remote_url: &str,
    ) -> Result<ConnectedRepository, GithubSurfaceError> {
        let initialized = !is_git_root(workspace);
        if initialized {
            let output = run_git(workspace, ["init", "-b", "main"])?;
            if !output.status.success() {
                return Err(GithubSurfaceError::RepositoryResolution {
                    workspace: workspace.display().to_string(),
                    detail: stderr_or_status(&output),
                });
            }
        }

        let replaced_remote = git_stdout(workspace, ["remote", "get-url", "origin"]).is_some();
        let args: [&str; 5] = if replaced_remote {
            ["remote", "set-url", "origin", "--", remote_url]
        } else {
            ["remote", "add", "origin", "--", remote_url]
        };
        let output = run_git(workspace, args)?;
        if !output.status.success() {
            return Err(GithubSurfaceError::RepositoryResolution {
                workspace: workspace.display().to_string(),
                detail: stderr_or_status(&output),
            });
        }

        // A stale negative is the failure mode this guards: the pane would keep
        // reporting "not connected" against a workspace that now resolves.
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let repository = self.resolve_repository(workspace)?;
        Ok(ConnectedRepository {
            repository,
            initialized,
            replaced_remote,
        })
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
        let mut command = Command::new(binary);
        command
            .current_dir(workspace)
            .args(["repo", "set-default", "--view"]);
        let output = run_with_timeout(command, self.command_timeout).ok().flatten()?;
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
            &pr_list_args(&repository, PR_LIST_BASE_FIELDS, "100"),
            false,
        )?;
        let raw: Vec<RawPullRequestSummary> = parse_json("pull-request list", &bytes)?;
        let mut pull_requests = raw
            .into_iter()
            .map(PullRequestSummary::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        self.enrich_prs(workspace, &repository, &mut pull_requests);
        self.store(key, CachedResource::PullRequests(pull_requests.clone()));
        Ok(pull_requests)
    }

    /// Load the wider rich slice used to arm background CI notifications.
    /// This stays separate from [`Self::list_prs`] so the interactive GitHub
    /// pane never waits for the notification poller's larger GraphQL query.
    pub fn list_prs_for_polling(
        &self,
        workspace: &Path,
    ) -> Result<Vec<PullRequestSummary>, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::PollingPullRequests,
        };
        if let Some(CachedResource::PullRequests(pull_requests)) = self.cached(&key) {
            return Ok(pull_requests);
        }
        let bytes = self.run_gh(
            workspace,
            "pr list",
            &pr_list_args(&repository, PR_LIST_FIELDS, PR_POLL_LIMIT),
            false,
        )?;
        let pull_requests = parse_json::<Vec<RawPullRequestSummary>>(
            "pull-request polling list",
            &bytes,
        )?
        .into_iter()
        .map(PullRequestSummary::try_from)
        .collect::<Result<Vec<_>, _>>()?;
        self.store(key, CachedResource::PullRequests(pull_requests.clone()));
        Ok(pull_requests)
    }

    /// Overlay the expensive computed fields (review decision, mergeability,
    /// merge-state, check rollup) onto the newest [`PR_ENRICH_LIMIT`] pull
    /// requests. GitHub prices these per PR in its GraphQL budget and answers
    /// 502/504 once enough pull requests carry checks, so the rich read is
    /// bounded and its failure degrades the list instead of erroring it.
    fn enrich_prs(
        &self,
        workspace: &Path,
        repository: &GithubRepository,
        pull_requests: &mut [PullRequestSummary],
    ) {
        if pull_requests.is_empty() {
            return;
        }
        let Ok(bytes) = self.run_gh_with_timeout(
            workspace,
            "pr list",
            &pr_list_args(repository, PR_LIST_FIELDS, PR_ENRICH_LIMIT),
            false,
            self.command_timeout.min(PR_ENRICH_TIMEOUT),
        ) else {
            return;
        };
        let Ok(raw) = parse_json::<Vec<RawPullRequestSummary>>("pull-request list", &bytes) else {
            return;
        };
        let mut rich = HashMap::new();
        for entry in raw {
            if let Ok(summary) = PullRequestSummary::try_from(entry) {
                rich.insert(summary.number, summary);
            }
        }
        for pull_request in pull_requests {
            if let Some(summary) = rich.remove(&pull_request.number) {
                *pull_request = summary;
            }
        }
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

    pub fn pr_files(
        &self,
        workspace: &Path,
        number: u64,
    ) -> Result<Vec<PullRequestFile>, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::PullRequestFiles(number),
        };
        if let Some(CachedResource::PullRequestFiles(files)) = self.cached(&key) {
            return Ok(files);
        }
        let mut args = vec!["api".into()];
        if !repository.host.eq_ignore_ascii_case("github.com") {
            args.push("--hostname".into());
            args.push(repository.host.clone());
        }
        args.extend([
            "--paginate".into(),
            "--slurp".into(),
            format!(
                "repos/{}/{}/pulls/{number}/files?per_page=100",
                repository.owner, repository.name
            ),
        ]);
        let bytes = self.run_gh(workspace, "pull-request files", &args, false)?;
        let pages: Vec<Vec<RawPullRequestFile>> = parse_json("pull-request files", &bytes)?;
        let files = pages
            .into_iter()
            .flatten()
            .map(PullRequestFile::from)
            .collect::<Vec<_>>();
        self.store(key, CachedResource::PullRequestFiles(files.clone()));
        Ok(files)
    }

    pub fn list_issues(&self, workspace: &Path) -> Result<Vec<IssueSummary>, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::Issues,
        };
        if let Some(CachedResource::Issues(issues)) = self.cached(&key) {
            return Ok(issues);
        }
        let bytes = self.run_gh(
            workspace,
            "issue list",
            &[
                "issue".into(),
                "list".into(),
                "--repo".into(),
                repository.selector(),
                "--state".into(),
                "open".into(),
                "--limit".into(),
                "100".into(),
                "--json".into(),
                ISSUE_LIST_FIELDS.into(),
            ],
            false,
        )?;
        let issues = parse_json::<Vec<RawIssueSummary>>("issue list", &bytes)?
            .into_iter()
            .map(IssueSummary::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        self.store(key, CachedResource::Issues(issues.clone()));
        Ok(issues)
    }

    pub fn issue_detail(
        &self,
        workspace: &Path,
        number: u64,
    ) -> Result<IssueDetail, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::Issue(number),
        };
        if let Some(CachedResource::Issue(issue)) = self.cached(&key) {
            return Ok(issue);
        }
        let bytes = self.run_gh(
            workspace,
            "issue view",
            &[
                "issue".into(),
                "view".into(),
                number.to_string(),
                "--repo".into(),
                repository.selector(),
                "--json".into(),
                ISSUE_DETAIL_FIELDS.into(),
            ],
            false,
        )?;
        let issue = IssueDetail::try_from(parse_json::<RawIssueDetail>("issue detail", &bytes)?)?;
        self.store(key, CachedResource::Issue(issue.clone()));
        Ok(issue)
    }

    pub fn repository_overview(
        &self,
        workspace: &Path,
    ) -> Result<RepositoryOverview, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let key = CacheKey {
            repository: repository.selector(),
            resource: Resource::RepositoryOverview,
        };
        if let Some(CachedResource::RepositoryOverview(overview)) = self.cached(&key) {
            return Ok(overview);
        }
        let bytes = self.run_gh(
            workspace,
            "repository overview",
            &[
                "repo".into(),
                "view".into(),
                repository.selector(),
                "--json".into(),
                REPOSITORY_FIELDS.into(),
            ],
            false,
        )?;
        let raw: RawRepositoryOverview = parse_json("repository overview", &bytes)?;
        let labels_bytes = self.run_gh(
            workspace,
            "label list",
            &[
                "label".into(),
                "list".into(),
                "--repo".into(),
                repository.selector(),
                "--limit".into(),
                "100".into(),
                "--json".into(),
                "name,color,description".into(),
            ],
            false,
        )?;
        let labels = parse_json::<Vec<RawLabel>>("label list", &labels_bytes)?
            .into_iter()
            .map(GithubLabel::from)
            .collect();
        let overview = RepositoryOverview::from_raw(raw, labels);
        self.store(key, CachedResource::RepositoryOverview(overview.clone()));
        Ok(overview)
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
        // failing. A nonempty JSON document is still a successful read, and a
        // branch with no checks at all is an empty result, not an error.
        let bytes = match self.run_gh(
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
        ) {
            Ok(bytes) => bytes,
            Err(GithubSurfaceError::CommandFailed { stderr, .. })
                if stderr.starts_with("no checks reported") =>
            {
                self.store(key, CachedResource::Checks(Vec::new()));
                return Ok(Vec::new());
            }
            Err(error) => return Err(error),
        };
        let raw: Vec<RawPullRequestCheck> = parse_json("pull-request checks", &bytes)?;
        let checks = raw
            .into_iter()
            .map(PullRequestCheck::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        self.store(key, CachedResource::Checks(checks.clone()));
        Ok(checks)
    }

    pub fn invalidate_checks(&self, workspace: &Path, number: u64) {
        if let Ok(repository) = self.resolve_repository(workspace) {
            self.cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&CacheKey {
                repository: repository.selector(), resource: Resource::Checks(number),
            });
        }
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

    /// The merge strategies the repository permits. A separate `gh api` read so
    /// the merge confirmation can offer only the allowed choices; the reader
    /// never widens what `gh pr merge` will actually accept.
    pub fn merge_config(&self, workspace: &Path) -> Result<MergeConfig, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let mut args: Vec<String> = vec!["api".into()];
        if !repository.host.eq_ignore_ascii_case("github.com") {
            args.push("--hostname".into());
            args.push(repository.host.clone());
        }
        args.push(format!("repos/{}/{}", repository.owner, repository.name));
        let bytes = self.run_gh(workspace, "repo settings", &args, false)?;
        let raw: RawRepositorySettings = parse_json("repository settings", &bytes)?;
        let strategies = MergeStrategies {
            merge: raw.allow_merge_commit,
            squash: raw.allow_squash_merge,
            rebase: raw.allow_rebase_merge,
        };
        // GitHub does not report a default merge method; squash is the
        // community-common default, so prefer it when the repo allows it.
        let default_strategy = if strategies.squash {
            MergeStrategy::Squash
        } else if strategies.merge {
            MergeStrategy::Merge
        } else if strategies.rebase {
            MergeStrategy::Rebase
        } else {
            return Err(malformed("repository settings", "no merge strategies are enabled"));
        };
        Ok(MergeConfig {
            strategies,
            default_strategy,
        })
    }

    pub fn comment_on_pull_request(
        &self,
        workspace: &Path,
        number: u64,
        body: &str,
    ) -> Result<String, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let selector = repository.selector();
        self.run_gh(
            workspace,
            "pr comment",
            &[
                "pr".into(),
                "comment".into(),
                number.to_string(),
                "--repo".into(),
                selector,
                "--body".into(),
                body.into(),
            ],
            false,
        )?;
        self.invalidate_action_resources(&repository, number);
        Ok(format!("Commented on PR #{number}."))
    }

    /// Execute one mutating action through `gh`. Every invocation is an argv
    /// array; a `gh` refusal (branch protection, required reviews, conflicts)
    /// propagates verbatim and is never retried.
    pub fn act(&self, workspace: &Path, action: &GithubAction) -> Result<String, GithubSurfaceError> {
        self.require_binary()?;
        let repository = self.resolve_repository(workspace)?;
        let selector = repository.selector();
        let result = match action {
            GithubAction::Merge { number, strategy } => {
                self.run_gh(
                    workspace,
                    "pr merge",
                    &[
                        "pr".into(),
                        "merge".into(),
                        number.to_string(),
                        "--repo".into(),
                        selector.clone(),
                        strategy.flag().into(),
                    ],
                    false,
                )?;
                Ok(format!("Merge requested for PR #{number} ({}).", strategy_label(*strategy)))
            }
            GithubAction::Review {
                number,
                event,
                body,
            } => {
                let mut args = vec![
                    "pr".into(),
                    "review".into(),
                    number.to_string(),
                    "--repo".into(),
                    selector.clone(),
                ];
                match event {
                    ReviewEvent::Approve => {
                        args.push("--approve".into());
                        if !body.is_empty() {
                            args.push("--body".into());
                            args.push(body.clone());
                        }
                    }
                    ReviewEvent::RequestChanges => {
                        args.push("--request-changes".into());
                        args.push("--body".into());
                        args.push(body.clone());
                    }
                    ReviewEvent::Comment => {
                        args.push("--comment".into());
                        args.push("--body".into());
                        args.push(body.clone());
                    }
                }
                self.run_gh(workspace, "pr review", &args, false)?;
                Ok(format!("Submitted {} on PR #{number}.", review_label(*event)))
            }
            GithubAction::Reply {
                number,
                comment_id,
                body,
            } => {
                let mut args = vec!["api".into()];
                if !repository.host.eq_ignore_ascii_case("github.com") {
                    args.push("--hostname".into());
                    args.push(repository.host.clone());
                }
                args.extend([
                    "--method".into(),
                    "POST".into(),
                    format!(
                        "repos/{}/{}/pulls/{number}/comments/{comment_id}/replies",
                        repository.owner, repository.name
                    ),
                    "-f".into(),
                    format!("body={body}"),
                ]);
                self.run_gh(
                    workspace,
                    "review reply",
                    &args,
                    false,
                )?;
                Ok(format!("Replied on PR #{number}."))
            }
            GithubAction::Rerun { number } => self.rerun_failed(workspace, &selector, *number),
            GithubAction::Label {
                target,
                number,
                label,
                operation,
            } => {
                let noun = target.noun();
                let flag = match operation {
                    LabelOperation::Add => "--add-label",
                    LabelOperation::Remove => "--remove-label",
                };
                self.run_gh(
                    workspace,
                    "edit labels",
                    &[
                        noun.to_string(),
                        "edit".into(),
                        number.to_string(),
                        "--repo".into(),
                        selector.clone(),
                        flag.into(),
                        label.clone(),
                    ],
                    false,
                )?;
                Ok(format!(
                    "{} label {label:?} {} #{number}.",
                    match operation {
                        LabelOperation::Add => "Added",
                        LabelOperation::Remove => "Removed",
                    },
                    match operation {
                        LabelOperation::Add => "to",
                        LabelOperation::Remove => "from",
                    }
                ))
            }
            GithubAction::Comment {
                target,
                number,
                body,
            } => {
                self.run_gh(
                    workspace,
                    "post comment",
                    &[
                        target.noun().to_string(),
                        "comment".into(),
                        number.to_string(),
                        "--repo".into(),
                        selector.clone(),
                        "--body".into(),
                        body.clone(),
                    ],
                    false,
                )?;
                Ok(format!("Commented on {} #{number}.", target.label()))
            }
            GithubAction::SetState {
                target,
                number,
                operation,
            } => {
                self.run_gh(
                    workspace,
                    "set state",
                    &[
                        target.noun().to_string(),
                        operation.subcommand().into(),
                        number.to_string(),
                        "--repo".into(),
                        selector.clone(),
                    ],
                    false,
                )?;
                Ok(format!(
                    "{} {} #{number}.",
                    match operation {
                        StateOperation::Close => "Closed",
                        StateOperation::Reopen => "Reopened",
                    },
                    target.label()
                ))
            }
            GithubAction::Ready { number } => {
                self.run_gh(
                    workspace,
                    "pr ready",
                    &[
                        "pr".into(),
                        "ready".into(),
                        number.to_string(),
                        "--repo".into(),
                        selector.clone(),
                    ],
                    false,
                )?;
                Ok(format!("Marked PR #{number} ready for review."))
            }
        };
        if result.is_ok() {
            self.invalidate_action_resources(&repository, action.number());
        }
        result
    }

    /// Re-run only the failed workflow runs on the pull request's head branch.
    fn rerun_failed(
        &self,
        workspace: &Path,
        selector: &str,
        number: u64,
    ) -> Result<String, GithubSurfaceError> {
        let head_sha = self.pr_head_sha(workspace, selector, number)?;
        let bytes = self.run_gh(
            workspace,
            "run list",
            &[
                "run".into(),
                "list".into(),
                "--repo".into(),
                selector.to_string(),
                "--commit".into(),
                head_sha,
                "--limit".into(),
                "20".into(),
                "--json".into(),
                "databaseId,status,conclusion".into(),
            ],
            false,
        )?;
        let runs: Vec<RawWorkflowRun> = parse_json("workflow runs", &bytes)?;
        let failed: Vec<u64> = runs
            .into_iter()
            .filter(|run| run.is_failed())
            .map(|run| run.database_id)
            .collect();
        if failed.is_empty() {
            return Ok("No failed runs to re-run.".into());
        }
        for id in &failed {
            self.run_gh(
                workspace,
                "run rerun",
                &[
                    "run".into(),
                    "rerun".into(),
                    id.to_string(),
                    "--repo".into(),
                    selector.to_string(),
                    "--failed".into(),
                ],
                false,
            )?;
            // A later rerun may still fail, but this one has already queued
            // work. Drop every stale PR view before returning that refusal.
            self.invalidate_action_resources_for_selector(selector, number);
        }
        // The re-run makes the checks queue again; drop the stale cached rollup
        // so the very next read (and the poller re-arm in the api layer) sees
        // the runs return to "running".
        self.invalidate_checks(workspace, number);
        let count = failed.len();
        Ok(format!(
            "Re-running {count} failed {} on PR #{number}.",
            if count == 1 { "run" } else { "runs" }
        ))
    }

    fn run_gh(
        &self,
        workspace: &Path,
        operation: &'static str,
        args: &[String],
        accept_nonzero_json: bool,
    ) -> Result<Vec<u8>, GithubSurfaceError> {
        self.run_gh_with_timeout(
            workspace,
            operation,
            args,
            accept_nonzero_json,
            self.command_timeout,
        )
    }

    fn run_gh_with_timeout(
        &self,
        workspace: &Path,
        operation: &'static str,
        args: &[String],
        accept_nonzero_json: bool,
        timeout: Duration,
    ) -> Result<Vec<u8>, GithubSurfaceError> {
        let mut command = Command::new(self.require_binary()?);
        command.current_dir(workspace).args(args);
        let Some(output) = run_with_timeout(command, timeout)? else {
            return Err(GithubSurfaceError::CommandFailed {
                operation,
                stderr: format!(
                    "timed out after {}s and was killed",
                    timeout.as_secs_f64()
                ),
            });
        };
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

    /// A completed write changes every cached view of its pull request. Clear
    /// them together so the UI's immediate refresh and a re-run's poller
    /// re-arm observe GitHub instead of a pre-write 15-second cache entry.
    fn invalidate_action_resources(&self, repository: &GithubRepository, number: u64) {
        self.invalidate_action_resources_for_selector(&repository.selector(), number);
    }

    fn invalidate_action_resources_for_selector(&self, selector: &str, number: u64) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for resource in [
            Resource::PullRequests,
            Resource::PollingPullRequests,
            Resource::PullRequest(number),
            Resource::PullRequestFiles(number),
            Resource::Checks(number),
            Resource::ReviewThreads(number),
            Resource::Issues,
            Resource::Issue(number),
            Resource::RepositoryOverview,
        ] {
            cache.remove(&CacheKey {
                repository: selector.into(),
                resource,
            });
        }
    }

    fn pr_head_sha(
        &self,
        workspace: &Path,
        selector: &str,
        number: u64,
    ) -> Result<String, GithubSurfaceError> {
        let bytes = self.run_gh(
            workspace,
            "pr head SHA",
            &[
                "pr".into(),
                "view".into(),
                number.to_string(),
                "--repo".into(),
                selector.into(),
                "--json".into(),
                PR_HEAD_FIELDS.into(),
            ],
            false,
        )?;
        let head: RawPullRequestHead = parse_json("pull-request head", &bytes)?;
        if head.head_ref_oid.is_empty() {
            return Err(malformed("pull-request head", "headRefOid was empty"));
        }
        Ok(head.head_ref_oid)
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
struct RawLabel {
    name: String,
    color: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComment {
    id: String,
    author: Option<RawActor>,
    body: String,
    created_at: String,
    url: String,
}

/// `gh pr view --json commits` names the author fields `login`/`name`, and a
/// commit whose email GitHub cannot map to an account carries an empty login.
/// Falling back to the git name keeps the row attributed instead of blank.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCommitAuthor {
    #[serde(default)]
    login: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequestCommit {
    oid: String,
    #[serde(default)]
    message_headline: String,
    #[serde(default)]
    message_body: String,
    #[serde(default)]
    committed_date: String,
    #[serde(default)]
    authors: Vec<RawCommitAuthor>,
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
    // The computed fields are absent from the cheap base list read; their
    // defaults are the honest "not fetched" readings.
    #[serde(default)]
    review_decision: String,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default)]
    merge_state_status: String,
    #[serde(default)]
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
    #[serde(default)]
    comments: Vec<RawComment>,
    #[serde(default)]
    labels: Vec<RawLabel>,
    #[serde(default)]
    commits: Vec<RawPullRequestCommit>,
    additions: u64,
    deletions: u64,
    changed_files: u64,
}

#[derive(Debug, Deserialize)]
struct RawPullRequestFile {
    filename: String,
    previous_filename: Option<String>,
    status: String,
    additions: u64,
    deletions: u64,
    patch: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssueSummary {
    number: u64,
    title: String,
    state: String,
    author: Option<RawActor>,
    #[serde(default)]
    labels: Vec<RawLabel>,
    created_at: String,
    updated_at: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawIssueDetail {
    number: u64,
    title: String,
    body: String,
    state: String,
    author: Option<RawActor>,
    #[serde(default)]
    labels: Vec<RawLabel>,
    #[serde(default)]
    comments: Vec<RawComment>,
    created_at: String,
    updated_at: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawNamedRef {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawCountConnection {
    total_count: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRepositoryOverview {
    name_with_owner: String,
    #[serde(default)]
    description: String,
    visibility: String,
    default_branch_ref: RawNamedRef,
    primary_language: Option<RawNamedRef>,
    url: String,
    issues: RawCountConnection,
    pull_requests: RawCountConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPullRequestHead {
    head_ref_oid: String,
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

// GitHub's REST API returns snake_case, unlike the `gh --json` reads elsewhere
// in this module, so these field names map directly with no rename.
#[derive(Debug, Default, Deserialize)]
struct RawRepositorySettings {
    #[serde(default)]
    allow_merge_commit: bool,
    #[serde(default)]
    allow_squash_merge: bool,
    #[serde(default)]
    allow_rebase_merge: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWorkflowRun {
    database_id: u64,
    #[serde(default)]
    conclusion: Option<String>,
}

impl RawWorkflowRun {
    /// A completed run counts as "failed" for re-run purposes when GitHub gave
    /// it a non-success terminal conclusion. Runs still in flight (empty
    /// conclusion) are left alone — there is nothing to re-run yet.
    fn is_failed(&self) -> bool {
        matches!(
            self.conclusion.as_deref(),
            Some("failure" | "timed_out" | "startup_failure")
        )
    }
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
            mergeability: raw
                .mergeable
                .as_deref()
                .map_or(Ok(Mergeability::Unknown), parse_mergeability)?,
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
            comments: raw.comments.into_iter().map(GithubComment::from).collect(),
            labels: raw.labels.into_iter().map(GithubLabel::from).collect(),
            commits: raw
                .commits
                .into_iter()
                .map(PullRequestCommit::from)
                .collect(),
            additions: raw.additions,
            deletions: raw.deletions,
            changed_files: raw.changed_files,
        })
    }
}

impl From<RawActor> for GithubActor {
    fn from(raw: RawActor) -> Self {
        Self { login: raw.login }
    }
}

impl From<RawPullRequestCommit> for PullRequestCommit {
    fn from(raw: RawPullRequestCommit) -> Self {
        // Seven is what GitHub's own UI shows, and `git log --abbrev-commit`
        // agrees on every repository small enough to matter here.
        let abbreviated_oid = raw.oid.chars().take(7).collect();
        Self {
            oid: raw.oid,
            abbreviated_oid,
            message_headline: raw.message_headline,
            message_body: raw.message_body,
            committed_at: raw.committed_date,
            authors: raw
                .authors
                .into_iter()
                .filter_map(|author| {
                    let login = if author.login.is_empty() { author.name } else { author.login };
                    (!login.is_empty()).then_some(GithubActor { login })
                })
                .collect(),
        }
    }
}

impl From<RawLabel> for GithubLabel {
    fn from(raw: RawLabel) -> Self {
        Self {
            name: raw.name,
            color: raw.color,
            description: raw.description,
        }
    }
}

impl From<RawComment> for GithubComment {
    fn from(raw: RawComment) -> Self {
        Self {
            id: raw.id,
            author: raw.author.map(GithubActor::from),
            body: raw.body,
            created_at: raw.created_at,
            url: raw.url,
        }
    }
}

impl From<RawPullRequestFile> for PullRequestFile {
    fn from(raw: RawPullRequestFile) -> Self {
        Self {
            path: raw.filename,
            previous_path: raw.previous_filename,
            status: raw.status,
            additions: raw.additions,
            deletions: raw.deletions,
            patch: raw.patch,
        }
    }
}

impl TryFrom<RawIssueSummary> for IssueSummary {
    type Error = GithubSurfaceError;

    fn try_from(raw: RawIssueSummary) -> Result<Self, Self::Error> {
        Ok(Self {
            number: raw.number,
            title: raw.title,
            state: parse_issue_state(&raw.state)?,
            author: raw.author.map(GithubActor::from),
            labels: raw.labels.into_iter().map(GithubLabel::from).collect(),
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            url: raw.url,
        })
    }
}

impl TryFrom<RawIssueDetail> for IssueDetail {
    type Error = GithubSurfaceError;

    fn try_from(raw: RawIssueDetail) -> Result<Self, Self::Error> {
        Ok(Self {
            summary: IssueSummary {
                number: raw.number,
                title: raw.title,
                state: parse_issue_state(&raw.state)?,
                author: raw.author.map(GithubActor::from),
                labels: raw.labels.into_iter().map(GithubLabel::from).collect(),
                created_at: raw.created_at,
                updated_at: raw.updated_at,
                url: raw.url,
            },
            body: raw.body,
            comments: raw.comments.into_iter().map(GithubComment::from).collect(),
        })
    }
}

impl RepositoryOverview {
    fn from_raw(raw: RawRepositoryOverview, labels: Vec<GithubLabel>) -> Self {
        Self {
            name_with_owner: raw.name_with_owner,
            description: raw.description,
            visibility: raw.visibility,
            default_branch: raw.default_branch_ref.name,
            primary_language: raw.primary_language.map(|language| language.name),
            url: raw.url,
            open_issues: raw.issues.total_count,
            open_pull_requests: raw.pull_requests.total_count,
            labels,
        }
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

/// The lowercase word for a merge strategy, used in confirmation statements.
pub fn strategy_label(strategy: MergeStrategy) -> &'static str {
    match strategy {
        MergeStrategy::Merge => "merge",
        MergeStrategy::Squash => "squash",
        MergeStrategy::Rebase => "rebase",
    }
}

/// The human phrase for a review event, used in confirmation statements.
pub fn review_label(event: ReviewEvent) -> &'static str {
    match event {
        ReviewEvent::Approve => "approval",
        ReviewEvent::RequestChanges => "requested changes",
        ReviewEvent::Comment => "a review comment",
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

fn parse_issue_state(value: &str) -> Result<IssueState, GithubSurfaceError> {
    match value {
        "OPEN" => Ok(IssueState::Open),
        "CLOSED" => Ok(IssueState::Closed),
        other => Err(malformed("issue", format!("unknown state {other:?}"))),
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
        "QUEUED" | "PENDING" | "WAITING" | "EXPECTED" | "REQUESTED" => {
            Ok((CheckStatus::Queued, None))
        }
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
        "QUEUED" | "PENDING" | "WAITING" | "EXPECTED" | "REQUESTED" => {
            Ok((CheckStatus::Queued, None))
        }
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

fn pr_list_args(repository: &GithubRepository, fields: &str, limit: &str) -> Vec<String> {
    vec![
        "pr".into(),
        "list".into(),
        "--repo".into(),
        repository.selector(),
        "--state".into(),
        "open".into(),
        "--limit".into(),
        limit.into(),
        "--json".into(),
        fields.into(),
    ]
}

/// Run a command to completion within `timeout`, capturing output. `Ok(None)`
/// means the deadline expired and the child was killed. Readers drain the
/// pipes on their own threads so a chatty child cannot deadlock the wait.
fn run_with_timeout(mut command: Command, timeout: Duration) -> std::io::Result<Option<Output>> {
    use std::io::Read;
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout_pipe = child.stdout.take().expect("stdout is piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr is piped");
    let stdout_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stdout_pipe.read_to_end(&mut buffer);
        buffer
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr_pipe.read_to_end(&mut buffer);
        buffer
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            // The readers are deliberately not joined: a grandchild that
            // inherited the pipe (credential helper, `sleep` in tests) keeps
            // it open past the kill, and the whole point here is to return at
            // the deadline. The threads end when the pipe finally closes.
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Some(Output {
        status,
        stdout,
        stderr,
    }))
}

fn probe_availability(binary: Option<&Path>) -> GithubAvailability {
    let Some(binary) = binary else {
        return GithubAvailability::NotInstalled;
    };
    let mut command = Command::new(binary);
    command.args(["auth", "status"]);
    match run_with_timeout(command, GH_COMMAND_TIMEOUT) {
        Ok(Some(output)) if output.status.success() => GithubAvailability::Available,
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

/// The remote a PR checkout fetches from: the branch's push/upstream remote
/// when one is configured, falling back to `origin`.
pub fn resolve_remote_name(workspace: &Path) -> String {
    branch_remote_candidates(workspace)
        .into_iter()
        .next()
        .unwrap_or_else(|| "origin".to_owned())
}

fn repository_for_remote(workspace: &Path, remote: &str) -> Option<GithubRepository> {
    let url = git_stdout(workspace, ["remote", "get-url", "--push", remote])
        .or_else(|| git_stdout(workspace, ["remote", "get-url", remote]))?;
    parse_remote_url(&url)
}

/// `true` when `workspace` is itself a repository root.
///
/// `rev-parse --git-dir` succeeds from any subdirectory by walking up, which
/// is right for reads and wrong for writes — `connect_workspace_folder` stores
/// a non-root selection as a plain folder, so treating an ancestor's
/// repository as this workspace's would mutate a different project. Mirrors
/// `git::validate_repo`.
fn is_git_root(workspace: &Path) -> bool {
    let Some(root) = git_stdout(workspace, ["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    match (std::fs::canonicalize(root), std::fs::canonicalize(workspace)) {
        (Ok(root), Ok(workspace)) => root == workspace,
        _ => false,
    }
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
        // An empty authority is a local URL such as file:///path/to/repo.
        if authority.is_empty() {
            return None;
        }
        let host = strip_port(authority.rsplit('@').next()?);
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

fn strip_port(host: &str) -> &str {
    match host.rsplit_once(':') {
        Some((bare, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            bare
        }
        _ => host,
    }
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
    use std::{fs, os::unix::fs::symlink, process::Command};
    use tempfile::TempDir;

    fn fake_gh(authenticated: bool, default_repository: Option<&str>) -> TempDir {
        let directory = tempfile::tempdir().unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testing/fixtures/github")
            .canonicalize()
            .unwrap();
        fs::write(directory.path().join("fixture-root"), fixtures.to_str().unwrap()).unwrap();
        fs::write(directory.path().join("auth-exit"), if authenticated { "0" } else { "1" }).unwrap();
        fs::write(directory.path().join("default-repository"), default_repository.unwrap_or("")).unwrap();
        // A newly written executable can race another test's fork on Linux:
        // an inherited writer briefly prevents exec even after fs::write
        // returned. Keep executable bytes immutable; only this test's data
        // and invocation log live in its temporary directory.
        symlink(fixtures.join("fake-gh.sh"), directory.path().join("gh")).unwrap();
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
    fn connect_initializes_a_plain_folder_and_resolves_it() {
        let folder = tempfile::tempdir().unwrap();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        // The reported failure: a folder that was never a git repository.
        assert!(surface.resolve_repository(folder.path()).is_err());

        let connected = surface
            .connect_repository(folder.path(), "https://github.com/bridge/harness.git")
            .unwrap();
        assert!(connected.initialized);
        assert!(!connected.replaced_remote);
        assert_eq!(connected.repository, expected("bridge", "harness"));
        assert_eq!(
            surface.resolve_repository(folder.path()).unwrap(),
            expected("bridge", "harness")
        );
    }

    #[test]
    fn connect_never_repoints_an_ancestor_repository() {
        let outer = repository();
        git(
            outer.path(),
            &["remote", "add", "origin", "https://github.com/outer/project.git"],
        );
        let nested = outer.path().join("vendor/thing");
        std::fs::create_dir_all(&nested).unwrap();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());

        // `rev-parse --git-dir` walks up and would hand back the outer repo.
        let connected = surface
            .connect_repository(&nested, "https://github.com/inner/thing.git")
            .unwrap();
        assert!(connected.initialized);
        assert_eq!(connected.repository, expected("inner", "thing"));
        // The outer project keeps the remote it had.
        assert_eq!(
            surface.resolve_repository(outer.path()).unwrap(),
            expected("outer", "project")
        );
    }

    #[test]
    fn connect_repoints_an_existing_origin_without_reinitializing() {
        let repository = repository();
        git(
            repository.path(),
            &["remote", "add", "origin", "https://github.com/old/name.git"],
        );
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());

        let connected = surface
            .connect_repository(repository.path(), "git@github.com:new/name.git")
            .unwrap();
        assert!(!connected.initialized);
        assert!(connected.replaced_remote);
        assert_eq!(connected.repository, expected("new", "name"));
        // A stale cached resolution here would keep the pane on the old repo.
        assert_eq!(
            surface.resolve_repository(repository.path()).unwrap(),
            expected("new", "name")
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
        assert_eq!(
            parse_remote_url("ssh://git@github.com:22/owner/repo.git"),
            Some(expected("owner", "repo"))
        );
        assert_eq!(parse_remote_url("../local/repo"), None);
        assert_eq!(parse_remote_url("file:///Users/fixture/repo"), None);
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
        assert_eq!(detail.comments[0].author.as_ref().unwrap().login, "maintainer");
        assert_eq!(detail.labels[0].name, "bug");
        assert_eq!((detail.additions, detail.deletions, detail.changed_files), (18, 4, 2));
        // Commits ride the same `pr view` read the pane already pays for.
        assert_eq!(detail.commits.len(), 2);
        assert_eq!(detail.commits[0].abbreviated_oid, "8f2a1c9");
        assert_eq!(detail.commits[0].message_headline, "fix(review): honour the requested changes");
        assert_eq!(detail.commits[0].authors[0].login, "review-author");
        // A commit GitHub cannot map to an account falls back to its git name
        // rather than rendering an unattributed row.
        assert_eq!(detail.commits[1].authors[0].login, "Local Committer");
    }

    #[test]
    fn a_failed_enrichment_degrades_to_the_base_list() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        // The rich read answers what GitHub answers on large repositories.
        fs::write(fake.path().join("pr-list-rich-fail"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        let pull_requests = surface.list_prs(repository.path()).unwrap();
        assert_eq!(pull_requests.len(), 5, "the list itself still loads");
        for pull_request in &pull_requests {
            assert_eq!(pull_request.review_decision, ReviewDecision::None);
            assert_eq!(pull_request.mergeability, Mergeability::Unknown);
            assert_eq!(pull_request.checks, CheckRollup::default());
        }
        assert_eq!(invocation_count(&fake, "pr list"), 2);
    }

    #[test]
    fn a_hung_gh_is_killed_and_reported_instead_of_blocking() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("hang"), "").unwrap();
        let mut surface = GithubSurface::discover_on_path(fake.path());
        surface.command_timeout = Duration::from_millis(200);
        let started = Instant::now();
        let error = surface.list_prs(repository.path()).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the call returns at the deadline, not when the child feels like it"
        );
        match error {
            GithubSurfaceError::CommandFailed { operation, stderr } => {
                assert_eq!(operation, "pr list");
                assert!(stderr.contains("timed out"), "got: {stderr}");
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
    }

    #[test]
    fn pr_files_parse_text_and_binary_changes() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let files = surface.pr_files(repository.path(), 103).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/githubSurface.ts");
        assert!(files[0].patch.as_deref().unwrap().contains("+new line"));
        assert_eq!(files[1].path, "assets/github.png");
        assert_eq!(files[1].patch, None, "binary files have no textual patch");
        assert!(invocations(&fake).contains(
            "api --paginate --slurp repos/fixture/project/pulls/103/files?per_page=100"
        ));
    }

    #[test]
    fn issues_and_repository_overview_are_typed() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());

        let issues = surface.list_issues(repository.path()).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].state, IssueState::Open);
        assert_eq!(issues[0].labels[0].name, "enhancement");

        let detail = surface.issue_detail(repository.path(), 17).unwrap();
        assert!(detail.body.contains("<script>"));
        assert_eq!(detail.comments[0].body, "Issue comment");

        let overview = surface.repository_overview(repository.path()).unwrap();
        assert_eq!(overview.name_with_owner, "fixture/project");
        assert_eq!(overview.default_branch, "main");
        assert_eq!(overview.primary_language.as_deref(), Some("TypeScript"));
        assert_eq!((overview.open_issues, overview.open_pull_requests), (3, 2));
        assert_eq!(overview.labels.len(), 2);
    }

    #[test]
    fn label_actions_use_exact_argv_and_invalidate_reads() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        surface.list_issues(repository.path()).unwrap();
        surface.issue_detail(repository.path(), 17).unwrap();

        surface.act(repository.path(), &GithubAction::Label {
            target: GithubTarget::Issue,
            number: 17,
            label: "bug".into(),
            operation: LabelOperation::Add,
        }).unwrap();
        surface.list_issues(repository.path()).unwrap();
        surface.issue_detail(repository.path(), 17).unwrap();
        assert!(invocations(&fake).contains(
            "issue edit 17 --repo fixture/project --add-label bug"
        ));
        assert_eq!(invocation_count(&fake, "issue list"), 2);
        assert_eq!(invocation_count(&fake, "issue view"), 2);

        surface.act(repository.path(), &GithubAction::Label {
            target: GithubTarget::PullRequest,
            number: 103,
            label: "bug".into(),
            operation: LabelOperation::Remove,
        }).unwrap();
        assert!(invocations(&fake).contains(
            "pr edit 103 --repo fixture/project --remove-label bug"
        ));
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
    fn pr_checks_on_a_branch_without_checks_are_an_empty_list() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("pr-checks-empty"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        assert_eq!(surface.pr_checks(repository.path(), 103).unwrap(), vec![]);
        assert_eq!(surface.pr_checks(repository.path(), 103).unwrap(), vec![]);
        assert_eq!(
            invocation_count(&fake, "pr checks"),
            1,
            "an empty checks result is cacheable"
        );
    }

    #[test]
    fn requested_checks_count_as_queued() {
        let raw = br#"[{"number":1,"title":"Requested","state":"OPEN","isDraft":false,"author":null,"headRefName":"requested","reviewDecision":"","mergeable":"UNKNOWN","mergeStateStatus":"UNKNOWN","statusCheckRollup":[{"__typename":"CheckRun","status":"REQUESTED","conclusion":""}],"url":"https://example.invalid"}]"#;
        let raw: Vec<RawPullRequestSummary> = parse_json("pull-request list", raw).unwrap();
        let summary = PullRequestSummary::try_from(raw.into_iter().next().unwrap()).unwrap();
        assert_eq!(summary.checks.queued, 1);
        assert_eq!(summary.checks.total, 1);
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
        // One fresh load is two spawns: the cheap base list plus the bounded
        // rich enrichment.
        assert_eq!(invocation_count(&fake, "pr list"), 2);

        surface.pr_detail(repository.path(), 103).unwrap();
        assert_eq!(invocation_count(&fake, "pr view"), 1);
        assert_eq!(invocation_count(&fake, "pr list"), 2);

        let log = fs::read_to_string(fake.path().join("invocations.log")).unwrap();
        assert!(log.contains(&format!("--limit 100 --json {PR_LIST_BASE_FIELDS}")));
        assert!(log.contains(&format!("--limit {PR_ENRICH_LIMIT} --json {PR_LIST_FIELDS}")));
        assert!(log.contains(&format!("--json {PR_DETAIL_FIELDS}")));
    }

    #[test]
    fn list_prs_enrichment_is_bounded_for_interactive_loading() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());

        surface.list_prs(repository.path()).unwrap();

        let log = invocations(&fake);
        assert!(
            log.contains(&format!("--limit 100 --json {PR_LIST_BASE_FIELDS}")),
            "the complete cheap list must remain available"
        );
        assert!(
            log.contains(&format!("--limit 5 --json {PR_LIST_FIELDS}")),
            "rich CI and review fields must stay within the interactive budget"
        );
    }

    #[test]
    fn slow_interactive_enrichment_returns_the_base_list_at_its_deadline() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("pr-list-rich-slow"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());

        let started = Instant::now();
        let pull_requests = surface.list_prs(repository.path()).unwrap();

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "slow enrichment must not make the pane look frozen"
        );
        assert_eq!(pull_requests.len(), 5, "the complete base fixture still loads");
        assert!(
            pull_requests
                .iter()
                .all(|pull_request| pull_request.checks == CheckRollup::default()),
            "timed-out enrichment degrades to honest not-fetched rollups"
        );
    }

    #[test]
    fn polling_prs_keep_the_previous_notification_coverage() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());

        surface.list_prs_for_polling(repository.path()).unwrap();

        let log = invocations(&fake);
        assert!(
            log.contains(&format!("--limit 25 --json {PR_LIST_FIELDS}")),
            "background CI polling must keep its previous 25-PR coverage"
        );
    }

    #[test]
    fn interactive_and_polling_pr_lists_have_independent_caches() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());

        surface.list_prs(repository.path()).unwrap();
        surface.list_prs_for_polling(repository.path()).unwrap();
        surface.list_prs(repository.path()).unwrap();
        surface.list_prs_for_polling(repository.path()).unwrap();

        let log = invocations(&fake);
        assert!(log.contains(&format!("--limit 5 --json {PR_LIST_FIELDS}")));
        assert!(log.contains(&format!("--limit 25 --json {PR_LIST_FIELDS}")));
        assert_eq!(
            invocation_count(&fake, "pr list"),
            3,
            "base, interactive-rich, and polling-rich reads cache independently"
        );
    }

    #[test]
    fn explicit_repository_refresh_bypasses_the_ttl_cache() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        surface.list_prs(repository.path()).unwrap();
        surface.list_prs(repository.path()).unwrap();
        assert_eq!(invocation_count(&fake, "pr list"), 2);
        surface.invalidate_repository(repository.path());
        surface.list_prs(repository.path()).unwrap();
        assert_eq!(invocation_count(&fake, "pr list"), 4);
    }

    fn invocations(fake: &TempDir) -> String {
        fs::read_to_string(fake.path().join("invocations.log")).unwrap_or_default()
    }

    #[test]
    fn merge_config_reports_allowed_strategies_and_default() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let config = surface.merge_config(repository.path()).unwrap();
        assert_eq!(
            config.strategies,
            MergeStrategies {
                merge: true,
                squash: true,
                rebase: false,
            }
        );
        // Squash is allowed, so it is the preselected default.
        assert_eq!(config.default_strategy, MergeStrategy::Squash);
        assert!(invocations(&fake).contains("api repos/fixture/project"));
    }

    #[test]
    fn merge_config_refuses_a_repository_without_any_merge_strategy() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("no-merge-strategies"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        assert!(matches!(
            surface.merge_config(repository.path()),
            Err(GithubSurfaceError::MalformedResponse { resource: "repository settings", .. })
        ));
    }

    #[test]
    fn merge_sends_the_selected_strategy_flag() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        for strategy in [MergeStrategy::Merge, MergeStrategy::Squash, MergeStrategy::Rebase] {
            let message = surface
                .act(
                    repository.path(),
                    &GithubAction::Merge { number: 103, strategy },
                )
                .unwrap();
            assert!(message.contains("Merge requested for PR #103"));
        }
        let log = invocations(&fake);
        assert!(log.contains("pr merge 103 --repo fixture/project --merge"));
        assert!(log.contains("pr merge 103 --repo fixture/project --squash"));
        assert!(log.contains("pr merge 103 --repo fixture/project --rebase"));
    }

    #[test]
    fn merge_propagates_a_branch_protection_refusal_verbatim() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        // Steer the shim to refuse the merge the way branch protection would.
        fs::write(fake.path().join("pr-merge-blocked"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        let error = surface
            .act(
                repository.path(),
                &GithubAction::Merge {
                    number: 103,
                    strategy: MergeStrategy::Squash,
                },
            )
            .unwrap_err();
        match error {
            GithubSurfaceError::CommandFailed { operation, stderr } => {
                assert_eq!(operation, "pr merge");
                assert_eq!(
                    stderr,
                    "GraphQL: Branch protections: at least 1 approving review is required (mergePullRequest)"
                );
            }
            other => panic!("expected a verbatim command failure, got {other:?}"),
        }
        // The refusal is never retried: exactly one merge attempt was spawned.
        assert_eq!(invocation_count(&fake, "pr merge"), 1);
    }

    #[test]
    fn a_conversation_comment_uses_pr_comment_argv_and_invalidates_the_pr() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        surface.pr_detail(repository.path(), 103).unwrap();
        surface
            .comment_on_pull_request(repository.path(), 103, "cursor review")
            .unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        assert!(invocations(&fake).contains(
            "pr comment 103 --repo fixture/project --body cursor review"
        ));
        assert_eq!(invocation_count(&fake, "pr view"), 2);
    }

    #[test]
    fn conversation_and_state_actions_build_the_gh_argv_for_each_target() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        for (action, expected) in [
            (
                GithubAction::Comment {
                    target: GithubTarget::PullRequest,
                    number: 103,
                    body: "looks good".into(),
                },
                "pr comment 103 --repo fixture/project --body looks good",
            ),
            (
                GithubAction::Comment {
                    target: GithubTarget::Issue,
                    number: 17,
                    body: "on it".into(),
                },
                "issue comment 17 --repo fixture/project --body on it",
            ),
            (
                GithubAction::SetState {
                    target: GithubTarget::PullRequest,
                    number: 103,
                    operation: StateOperation::Close,
                },
                "pr close 103 --repo fixture/project",
            ),
            (
                GithubAction::SetState {
                    target: GithubTarget::Issue,
                    number: 17,
                    operation: StateOperation::Reopen,
                },
                "issue reopen 17 --repo fixture/project",
            ),
            (GithubAction::Ready { number: 103 }, "pr ready 103 --repo fixture/project"),
        ] {
            surface.act(repository.path(), &action).unwrap();
            assert!(
                invocations(&fake).contains(expected),
                "expected {expected:?} in the invocation log"
            );
        }
    }

    #[test]
    fn a_state_change_invalidates_the_cached_views_of_its_target() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        surface.pr_detail(repository.path(), 103).unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        assert_eq!(invocation_count(&fake, "pr view"), 1, "the second read is cached");
        surface
            .act(
                repository.path(),
                &GithubAction::SetState {
                    target: GithubTarget::PullRequest,
                    number: 103,
                    operation: StateOperation::Close,
                },
            )
            .unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        assert_eq!(invocation_count(&fake, "pr view"), 2, "the write dropped the cached view");
    }

    #[test]
    fn review_builds_the_correct_event_argv() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        // Approve with no body omits --body entirely.
        surface
            .act(
                repository.path(),
                &GithubAction::Review {
                    number: 103,
                    event: ReviewEvent::Approve,
                    body: String::new(),
                },
            )
            .unwrap();
        surface
            .act(
                repository.path(),
                &GithubAction::Review {
                    number: 103,
                    event: ReviewEvent::RequestChanges,
                    body: "please fix".into(),
                },
            )
            .unwrap();
        surface
            .act(
                repository.path(),
                &GithubAction::Review {
                    number: 103,
                    event: ReviewEvent::Comment,
                    body: "a note".into(),
                },
            )
            .unwrap();
        let log = invocations(&fake);
        assert!(log.contains("pr review 103 --repo fixture/project --approve\n"));
        assert!(!log.contains("--approve --body"));
        assert!(log.contains("pr review 103 --repo fixture/project --request-changes --body please fix"));
        assert!(log.contains("pr review 103 --repo fixture/project --comment --body a note"));
    }

    #[test]
    fn reply_posts_to_the_review_comment_replies_endpoint() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        surface
            .act(
                repository.path(),
                &GithubAction::Reply {
                    number: 103,
                    comment_id: 55,
                    body: "thanks".into(),
                },
            )
            .unwrap();
        let log = invocations(&fake);
        assert!(log.contains(
            "api --method POST repos/fixture/project/pulls/103/comments/55/replies -f body=thanks"
        ));
    }

    #[test]
    fn reply_uses_workspace_hostname_for_github_enterprise() {
        let repository = repository();
        git(
            repository.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://github.example.test/fixture/project.git",
            ],
        );
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("allow-all"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        surface
            .act(
                repository.path(),
                &GithubAction::Reply {
                    number: 103,
                    comment_id: 55,
                    body: "thanks".into(),
                },
            )
            .unwrap();
        assert!(invocations(&fake).contains(
            "api --hostname github.example.test --method POST repos/fixture/project/pulls/103/comments/55/replies -f body=thanks"
        ));
    }

    #[test]
    fn completed_action_invalidates_all_cached_pull_request_resources() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        surface.list_prs(repository.path()).unwrap();
        surface.list_prs_for_polling(repository.path()).unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        surface.pr_checks(repository.path(), 103).unwrap();
        surface.pr_review_threads(repository.path(), 103).unwrap();

        surface
            .act(
                repository.path(),
                &GithubAction::Review {
                    number: 103,
                    event: ReviewEvent::Approve,
                    body: String::new(),
                },
            )
            .unwrap();

        surface.list_prs(repository.path()).unwrap();
        surface.list_prs_for_polling(repository.path()).unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        surface.pr_checks(repository.path(), 103).unwrap();
        surface.pr_review_threads(repository.path(), 103).unwrap();
        assert_eq!(invocation_count(&fake, "pr list"), 6);
        assert_eq!(invocation_count(&fake, "pr view"), 2);
        assert_eq!(invocation_count(&fake, "pr checks"), 2);
        assert_eq!(invocation_count(&fake, "api graphql"), 2);
    }

    #[test]
    fn rerun_acts_only_on_failed_runs() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        let surface = GithubSurface::discover_on_path(fake.path());
        let message = surface
            .act(repository.path(), &GithubAction::Rerun { number: 103 })
            .unwrap();
        assert!(message.contains("Re-running 1 failed run"));
        let log = invocations(&fake);
        // Only the failed run (42) is re-run — the passing (43) and in-progress
        // (44) runs are left alone.
        assert!(log.contains("run rerun 42 --repo fixture/project --failed"));
        assert!(!log.contains("run rerun 43"));
        assert!(!log.contains("run rerun 44"));
        assert_eq!(invocation_count(&fake, "run rerun"), 1);
        assert!(log.contains("pr view 103 --repo fixture/project --json headRefOid"));
        assert!(log.contains("run list --repo fixture/project --commit current-head-sha"));
    }

    #[test]
    fn partial_rerun_failure_still_invalidates_cached_pull_request_resources() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("partial-rerun-failure"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        surface.list_prs(repository.path()).unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        surface.pr_checks(repository.path(), 103).unwrap();
        surface.pr_review_threads(repository.path(), 103).unwrap();

        assert!(surface
            .act(repository.path(), &GithubAction::Rerun { number: 103 })
            .is_err());

        surface.list_prs(repository.path()).unwrap();
        surface.pr_detail(repository.path(), 103).unwrap();
        surface.pr_checks(repository.path(), 103).unwrap();
        surface.pr_review_threads(repository.path(), 103).unwrap();
        assert_eq!(invocation_count(&fake, "pr list"), 4);
        assert_eq!(invocation_count(&fake, "pr view"), 3);
        assert_eq!(invocation_count(&fake, "pr checks"), 2);
        assert_eq!(invocation_count(&fake, "api graphql"), 2);
    }

    #[test]
    fn rerun_with_no_failed_runs_spawns_no_rerun() {
        let repository = repository_with_origin();
        let fake = fake_gh(true, None);
        fs::write(fake.path().join("run-list-clean"), "").unwrap();
        let surface = GithubSurface::discover_on_path(fake.path());
        let message = surface
            .act(repository.path(), &GithubAction::Rerun { number: 103 })
            .unwrap();
        assert_eq!(message, "No failed runs to re-run.");
        assert_eq!(invocation_count(&fake, "run rerun"), 0);
    }
}
