//! The reviews domain: pull-request reviews as stored background jobs.
//!
//! A review is a row Bridge's own runner drives, not a worker of a chat. It is
//! started, watched through `reviews/job_updated`, cancelled or retried by id,
//! and read back with its passes (one per harness that ran) and its findings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a review job is. `posted`, `failed`, `cancelled` and `interrupted`
/// are terminal. `drafted` waits for a person to triage and post.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewJobStatus {
    Queued,
    Preparing,
    Reviewing,
    Drafted,
    Posting,
    Posted,
    Failed,
    Cancelled,
    /// Bridge stopped while the job was in flight. Retry starts it again.
    Interrupted,
}

impl ReviewJobStatus {
    pub const ALL: [ReviewJobStatus; 9] = [
        ReviewJobStatus::Queued,
        ReviewJobStatus::Preparing,
        ReviewJobStatus::Reviewing,
        ReviewJobStatus::Drafted,
        ReviewJobStatus::Posting,
        ReviewJobStatus::Posted,
        ReviewJobStatus::Failed,
        ReviewJobStatus::Cancelled,
        ReviewJobStatus::Interrupted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewJobStatus::Queued => "queued",
            ReviewJobStatus::Preparing => "preparing",
            ReviewJobStatus::Reviewing => "reviewing",
            ReviewJobStatus::Drafted => "drafted",
            ReviewJobStatus::Posting => "posting",
            ReviewJobStatus::Posted => "posted",
            ReviewJobStatus::Failed => "failed",
            ReviewJobStatus::Cancelled => "cancelled",
            ReviewJobStatus::Interrupted => "interrupted",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewJobStatus> {
        ReviewJobStatus::ALL.into_iter().find(|status| status.as_str() == text)
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            ReviewJobStatus::Posted
                | ReviewJobStatus::Failed
                | ReviewJobStatus::Cancelled
                | ReviewJobStatus::Interrupted
        )
    }
}

/// What started a review.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTrigger {
    #[default]
    Manual,
    /// A new head commit on a watched PR (opt-in auto-review).
    Push,
}

impl ReviewTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewTrigger::Manual => "manual",
            ReviewTrigger::Push => "push",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewTrigger> {
        match text {
            "manual" => Some(ReviewTrigger::Manual),
            "push" => Some(ReviewTrigger::Push),
            _ => None,
        }
    }
}

/// Whether findings wait for triage or post as soon as they are drafted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPostMode {
    /// Findings land in the pane; nothing reaches GitHub until a person posts.
    #[default]
    Draft,
    /// Bridge posts the review as soon as it is drafted.
    Auto,
}

impl ReviewPostMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewPostMode::Draft => "draft",
            ReviewPostMode::Auto => "auto",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewPostMode> {
        match text {
            "draft" => Some(ReviewPostMode::Draft),
            "auto" => Some(ReviewPostMode::Auto),
            _ => None,
        }
    }
}

/// Which pull requests Bridge reviews on its own when they open or get new
/// commits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAutoMode {
    /// Reviews start only when a person asks.
    #[default]
    Off,
    /// Pull requests opened by the signed-in GitHub account.
    Mine,
    /// Every open pull request of a watched repository.
    All,
}

impl ReviewAutoMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewAutoMode::Off => "off",
            ReviewAutoMode::Mine => "mine",
            ReviewAutoMode::All => "all",
        }
    }
}

/// Findings on a job, by severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFindingCounts {
    pub total: u32,
    pub critical: u32,
    pub major: u32,
    pub minor: u32,
    pub nit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewJob {
    pub id: String,
    pub workspace_id: String,
    pub number: u64,
    pub head_sha: String,
    pub status: ReviewJobStatus,
    pub trigger: ReviewTrigger,
    pub post_mode: ReviewPostMode,
    /// The harness of each pass: the one that actually ran when a fallback
    /// replaced the one asked for, else the one asked for. Two entries mean a
    /// second opinion.
    pub harnesses: Vec<String>,
    pub finding_counts: ReviewFindingCounts,
    /// A re-review: the head of the PR's last posted review. The reviewer
    /// looks at the commits since it, remembering earlier findings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incremental_from_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted_review_url: Option<String>,
    /// The job this one retried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_of: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// The repository, for views that list reviews across repositories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_name: Option<String>,
    /// The PR's title and author, once the job has prepared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_author: Option<String>,
    /// The chat the review was sent to (see `ReviewJobSettings::handoff`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_chat: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_at: Option<String>,
    /// Why the review was not sent, when handoff is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPassStatus {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl ReviewPassStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewPassStatus::Running => "running",
            ReviewPassStatus::Done => "done",
            ReviewPassStatus::Failed => "failed",
            ReviewPassStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewPassStatus> {
        match text {
            "running" => Some(ReviewPassStatus::Running),
            "done" => Some(ReviewPassStatus::Done),
            "failed" => Some(ReviewPassStatus::Failed),
            "cancelled" => Some(ReviewPassStatus::Cancelled),
            _ => None,
        }
    }
}

/// One harness's run over a job. A fallback is a new pass in the same slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPass {
    pub id: String,
    pub job_id: String,
    /// Which requested reviewer this pass stands for (0, or 1 for a second
    /// opinion).
    pub slot: u32,
    /// The claim of the job this pass ran under, from 1. A pass of an earlier
    /// attempt was abandoned when its runner lost the job.
    pub attempt: u32,
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub status: ReviewPassStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Permission requests the hidden session denied.
    pub denied_requests: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_microusd: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    /// The person's taste rules this pass was given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taste_rule_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ReviewSide {
    #[serde(rename = "LEFT")]
    Left,
    #[serde(rename = "RIGHT")]
    Right,
}

impl ReviewSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewSide::Left => "LEFT",
            ReviewSide::Right => "RIGHT",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewSide> {
        match text {
            "LEFT" => Some(ReviewSide::Left),
            "RIGHT" => Some(ReviewSide::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Critical,
    Major,
    Minor,
    Nit,
}

impl ReviewSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewSeverity::Critical => "critical",
            ReviewSeverity::Major => "major",
            ReviewSeverity::Minor => "minor",
            ReviewSeverity::Nit => "nit",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewSeverity> {
        match text {
            "critical" => Some(ReviewSeverity::Critical),
            "major" => Some(ReviewSeverity::Major),
            "minor" => Some(ReviewSeverity::Minor),
            "nit" => Some(ReviewSeverity::Nit),
            _ => None,
        }
    }
}

/// What a person decided about a finding. Every decision is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingDecision {
    #[default]
    Pending,
    Kept,
    Dropped,
    Edited,
}

impl ReviewFindingDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewFindingDecision::Pending => "pending",
            ReviewFindingDecision::Kept => "kept",
            ReviewFindingDecision::Dropped => "dropped",
            ReviewFindingDecision::Edited => "edited",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewFindingDecision> {
        match text {
            "pending" => Some(ReviewFindingDecision::Pending),
            "kept" => Some(ReviewFindingDecision::Kept),
            "dropped" => Some(ReviewFindingDecision::Dropped),
            "edited" => Some(ReviewFindingDecision::Edited),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFinding {
    pub id: String,
    pub job_id: String,
    pub pass_id: String,
    pub path: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    pub side: ReviewSide,
    pub severity: ReviewSeverity,
    pub title: String,
    pub body: String,
    pub confidence: f64,
    pub fingerprint: String,
    /// How many reviewers reported it (2 when a second opinion agreed).
    pub agreed_by: u32,
    /// The passes a merged finding came from, `pass_id`'s first. Empty unless
    /// two reviewers agreed on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pass_ids: Vec<String>,
    /// True when its line falls outside the diff: it goes in the summary
    /// rather than inline.
    pub outside_diff: bool,
    pub decision: ReviewFindingDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_body: Option<String>,
    /// When a person last decided. Absent while `pending`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_comment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted_at: Option<String>,
    /// The head of a later re-review that found this fixed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_in_sha: Option<String>,
    /// What the finding is about, as the reviewer tagged it: `security`,
    /// `correctness`, `error-handling`, `tests`, `performance`, `naming`,
    /// `docs`, `style` or `other`. Absent when it did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Set when the taste layer hid the finding because the person kept
    /// dropping ones like it, in their words ("You dropped 4 like this, last
    /// on #212."). A hidden finding is still pending: it can be shown and
    /// decided like any other, and it is never posted without a decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden_reason: Option<String>,
    /// The person's taste rules the reviewer says shaped this finding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_ids: Vec<String>,
}

/// How review jobs run. One record for the account, separate from
/// `ReviewerSettings` (the reviewer's model, effort and instructions) so that
/// type keeps the shape older clients decode.
///
/// Open on purpose (no `deny_unknown_fields`): later minors add fields here,
/// and an older client must still decode a newer response. Every field has a
/// default, so a record stored before a field existed loads with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", default)]
pub struct ReviewJobSettings {
    /// The harnesses a review runs on when the caller names none. Empty
    /// means the first available of Claude, Codex and OpenCode.
    pub default_harnesses: Vec<String>,
    /// Tried in order when a pass's harness fails (sign-in, usage limit,
    /// crash). Empty means a failed harness fails that pass.
    pub fallback_harnesses: Vec<String>,
    /// Reviews running at once, 1 to 4.
    pub max_concurrent_reviews: u8,
    /// Draft first (the default) or post as soon as findings are drafted.
    pub post_mode: ReviewPostMode,
    /// Review checkouts that may exist at once, 1 to 32. A job waits in the
    /// queue while they are all in use.
    pub max_review_worktrees: u8,
    /// Whether the default reviewers (and so auto-review) may be a pair. Off
    /// (the default), they are one harness. A review asked for with two
    /// harnesses (the Review menu's Second opinion) runs on both either way.
    pub second_opinion: bool,
    /// Review pull requests on their own when they open or get new commits.
    /// Off by default.
    pub auto_review: ReviewAutoMode,
    /// With auto-review on, leave draft pull requests until they are ready.
    pub skip_draft_prs: bool,
    /// A later review of a PR looks at the commits since its last posted
    /// review and remembers what was said and decided. Off by default.
    pub incremental: bool,
    /// Send finished reviews to the chat that wrote the pull request, which
    /// then works on them. Off by default.
    pub handoff: ReviewHandoff,
    /// Hide a new finding that closely resembles ones the person dropped at
    /// least three times across two pull requests. Hidden findings stay one
    /// click away and are never posted without a decision; critical and
    /// security findings are never hidden. On by default.
    pub hide_repeat_drops: bool,
    /// Give the reviewer the person's active taste rules for the repository,
    /// and let it say which shaped a finding. On by default.
    pub follow_taste_rules: bool,
}

/// When a finished review goes to the chat that wrote the pull request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewHandoff {
    /// Never: the person reads reviews in the Reviews app or on GitHub.
    #[default]
    Off,
    /// As soon as a review drafts, before anyone has triaged it.
    Drafted,
    /// When a review posts: only what was posted, with the person's edits.
    Posted,
}

/// The default and the bounds of `ReviewJobSettings::max_concurrent_reviews`.
pub const DEFAULT_MAX_CONCURRENT_REVIEWS: u8 = 2;
pub const MAX_CONCURRENT_REVIEWS_RANGE: std::ops::RangeInclusive<u8> = 1..=4;
/// The default and the bounds of `ReviewJobSettings::max_review_worktrees`.
pub const DEFAULT_MAX_REVIEW_WORKTREES: u8 = 8;
pub const MAX_REVIEW_WORKTREES_RANGE: std::ops::RangeInclusive<u8> = 1..=32;

impl Default for ReviewJobSettings {
    fn default() -> Self {
        Self {
            default_harnesses: Vec::new(),
            fallback_harnesses: Vec::new(),
            max_concurrent_reviews: DEFAULT_MAX_CONCURRENT_REVIEWS,
            post_mode: ReviewPostMode::Draft,
            max_review_worktrees: DEFAULT_MAX_REVIEW_WORKTREES,
            second_opinion: false,
            auto_review: ReviewAutoMode::Off,
            skip_draft_prs: true,
            incremental: false,
            handoff: ReviewHandoff::Off,
            hide_repeat_drops: true,
            follow_taste_rules: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveReviewJobSettingsParams {
    pub settings: ReviewJobSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartReviewParams {
    pub workspace_id: String,
    pub number: u64,
    /// Defaults to the reviewer settings' default harnesses. Two entries ask
    /// for a second opinion.
    #[serde(default)]
    pub harnesses: Option<Vec<String>>,
    #[serde(default)]
    pub trigger: Option<ReviewTrigger>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListReviewsParams {
    pub workspace_id: String,
    #[serde(default)]
    pub number: Option<u64>,
}

/// What a reviewer did, one entry at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewActivityKind {
    /// What Bridge asked the reviewer.
    Prompt,
    /// Something the reviewer said.
    Message,
    /// A tool the reviewer used (a read, a search).
    Tool,
    /// A command the reviewer ran in its sandbox.
    Command,
    /// A permission request or question Bridge refused.
    Denied,
    /// A provider error that ended the turn.
    Error,
}

impl ReviewActivityKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ReviewActivityKind::Prompt => "prompt",
            ReviewActivityKind::Message => "message",
            ReviewActivityKind::Tool => "tool",
            ReviewActivityKind::Command => "command",
            ReviewActivityKind::Denied => "denied",
            ReviewActivityKind::Error => "error",
        }
    }

    pub fn parse(text: &str) -> Option<ReviewActivityKind> {
        match text {
            "prompt" => Some(ReviewActivityKind::Prompt),
            "message" => Some(ReviewActivityKind::Message),
            "tool" => Some(ReviewActivityKind::Tool),
            "command" => Some(ReviewActivityKind::Command),
            "denied" => Some(ReviewActivityKind::Denied),
            "error" => Some(ReviewActivityKind::Error),
            _ => None,
        }
    }
}

/// One thing a reviewer pass did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewActivityEntry {
    pub job_id: String,
    pub pass_id: String,
    /// From 1, per pass.
    pub seq: u32,
    pub kind: ReviewActivityKind,
    /// A tool's or command's name, or a short heading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The message, the prompt, or a tool's detail (a path, a pattern).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewActivityParams {
    pub job_id: String,
}

/// Every entry of a job's passes: pass by pass, each in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewActivityList {
    pub entries: Vec<ReviewActivityEntry>,
}

/// Everything the Reviews dashboard shows, for a period and optionally one
/// repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewOverviewParams {
    /// The last this many days, 1 to 365. Absent: all time.
    #[serde(default)]
    pub days: Option<u32>,
    /// `owner/name`, any case. Every workspace of that repository counts.
    #[serde(default)]
    pub repository: Option<String>,
}

/// The Reviews dashboard: jobs, stats, one row per reviewer, recent findings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOverview {
    /// Jobs created in the period, newest first, capped, plus every live job.
    pub jobs: Vec<ReviewJob>,
    pub stats: ReviewOverviewStats,
    /// One row per harness that ran a pass in the period.
    pub harnesses: Vec<ReviewHarnessStats>,
    /// Findings of the period's jobs, newest first, capped.
    pub findings: Vec<ReviewOverviewFinding>,
    /// Every `owner/name` with a review, any period, for the filter.
    pub repositories: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOverviewStats {
    /// Jobs created in the period.
    pub reviews: u32,
    /// Queued, preparing, reviewing or posting.
    pub running: u32,
    /// Drafted: waiting for a person.
    pub waiting: u32,
    pub posted: u32,
    /// Failed or interrupted.
    pub failed: u32,
    pub cancelled: u32,
    pub findings: ReviewFindingCounts,
    pub kept: u32,
    pub edited: u32,
    pub dropped: u32,
    pub pending: u32,
    /// Findings a later review found fixed.
    pub fixed_later: u32,
    /// Findings two reviewers agreed on.
    pub agreed: u32,
    /// Median time from a job's start to its last pass finishing, over jobs
    /// that drafted or posted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_seconds: Option<u32>,
    pub spend_microusd: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewHarnessStats {
    pub harness: String,
    pub passes: u32,
    pub failed: u32,
    /// Findings this harness raised (a merged finding counts for both).
    pub findings: u32,
    /// Kept or edited.
    pub kept: u32,
    pub dropped: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_seconds: Option<u32>,
    pub spend_microusd: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOverviewFinding {
    pub finding: ReviewFinding,
    pub workspace_id: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub number: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_title: Option<String>,
    /// The harnesses that found it.
    pub harnesses: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetReviewParams {
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelReviewParams {
    pub job_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryReviewParams {
    pub job_id: String,
}

/// What a person decided about one finding of a drafted job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewFindingDecisionInput {
    pub finding_id: String,
    /// `kept`, `dropped` or `edited`.
    pub decision: ReviewFindingDecision,
    /// The body to post instead of the reviewer's. Required with `edited`,
    /// refused with anything else.
    #[serde(default)]
    pub edited_body: Option<String>,
}

/// Decide findings of a drafted job. All or nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DecideReviewParams {
    pub job_id: String,
    pub decisions: Vec<ReviewFindingDecisionInput>,
}

/// Every finding of the job, after the decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewFindingList {
    pub findings: Vec<ReviewFinding>,
}

/// Post a drafted job's kept and edited findings as one `COMMENT` review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PostReviewParams {
    pub job_id: String,
}

/// One job: the answer to start, cancel and retry, and the payload of
/// `reviews/job_updated`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewJobResult {
    pub job: ReviewJob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewJobList {
    /// Newest first, at most 50.
    pub jobs: Vec<ReviewJob>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewJobDetail {
    pub job: ReviewJob,
    pub passes: Vec<ReviewPass>,
    pub findings: Vec<ReviewFinding>,
    /// What the reviewers said overall, reviewers named. Absent until the
    /// review drafted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn review_job_wire_shape_is_camel_case_and_round_trips() {
        let job = ReviewJob {
            id: "j1".into(),
            workspace_id: "w1".into(),
            number: 42,
            head_sha: "abc".into(),
            status: ReviewJobStatus::Drafted,
            trigger: ReviewTrigger::Push,
            post_mode: ReviewPostMode::Auto,
            harnesses: vec!["claude".into(), "codex".into()],
            finding_counts: ReviewFindingCounts { total: 3, critical: 1, major: 1, minor: 0, nit: 1 },
            incremental_from_sha: Some("prev".into()),
            error: None,
            posted_review_url: Some("https://github.com/o/r/pull/42#pullrequestreview-1".into()),
            retry_of: Some("j0".into()),
            created_at: "2026-11-23T10:00:00.000Z".into(),
            started_at: None,
            finished_at: None,
            repo_owner: None,
            repo_name: None,
            pr_title: None,
            pr_author: None,
            handoff_session_id: None,
            handoff_chat: None,
            handoff_at: None,
            handoff_note: None,
        };
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["workspaceId"], "w1");
        assert_eq!(value["headSha"], "abc");
        assert_eq!(value["postMode"], "auto");
        assert_eq!(value["trigger"], "push");
        assert_eq!(value["status"], "drafted");
        assert_eq!(value["findingCounts"]["critical"], 1);
        assert_eq!(value["retryOf"], "j0");
        assert_eq!(value["incrementalFromSha"], "prev");
        assert!(value.get("error").is_none(), "absent options are omitted");
        assert_eq!(serde_json::from_value::<ReviewJob>(value).unwrap(), job);
        assert_eq!(serde_json::to_value(ReviewSide::Left).unwrap(), json!("LEFT"));
        assert_eq!(serde_json::to_value(ReviewSeverity::Nit).unwrap(), json!("nit"));
    }

    #[test]
    fn params_refuse_unknown_fields_and_default_their_options() {
        let start: StartReviewParams = serde_json::from_value(json!({ "workspaceId": "w", "number": 7 })).unwrap();
        assert_eq!(start.harnesses, None);
        assert_eq!(start.trigger, None);
        assert!(serde_json::from_value::<StartReviewParams>(json!({ "workspaceId": "w", "number": 7, "sessionId": "s" })).is_err());
        assert!(serde_json::from_value::<GetReviewParams>(json!({ "jobId": "j", "x": 1 })).is_err());
        let list: ListReviewsParams = serde_json::from_value(json!({ "workspaceId": "w" })).unwrap();
        assert_eq!(list.number, None);
        let decide: DecideReviewParams = serde_json::from_value(json!({
            "jobId": "j",
            "decisions": [{ "findingId": "f", "decision": "edited", "editedBody": "b" }, { "findingId": "g", "decision": "dropped" }]
        }))
        .unwrap();
        assert_eq!(decide.decisions[0].decision, ReviewFindingDecision::Edited);
        assert_eq!(decide.decisions[1].edited_body, None);
        assert!(serde_json::from_value::<DecideReviewParams>(
            json!({ "jobId": "j", "decisions": [{ "findingId": "f", "decision": "kept", "why": 1 }] })
        )
        .is_err());
        assert!(serde_json::from_value::<PostReviewParams>(json!({ "jobId": "j", "event": "APPROVE" })).is_err());
    }

    #[test]
    fn review_job_settings_default_and_stay_open_to_newer_fields() {
        let defaults: ReviewJobSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(defaults, ReviewJobSettings::default());
        assert_eq!(defaults.max_concurrent_reviews, 2);
        assert_eq!(defaults.post_mode, ReviewPostMode::Draft);
        let newer: ReviewJobSettings =
            serde_json::from_value(json!({ "postMode": "auto", "autoReview": "mine" })).unwrap();
        assert_eq!(newer.post_mode, ReviewPostMode::Auto, "a field from a later minor is ignored, not refused");
        assert!(serde_json::from_value::<SaveReviewJobSettingsParams>(json!({ "settings": {}, "x": 1 })).is_err());
    }

    #[test]
    fn status_text_round_trips_and_terminal_set_is_pinned() {
        for status in ReviewJobStatus::ALL {
            assert_eq!(ReviewJobStatus::parse(status.as_str()), Some(status));
            assert_eq!(serde_json::to_value(status).unwrap(), json!(status.as_str()));
        }
        let terminal: Vec<_> = ReviewJobStatus::ALL.into_iter().filter(|status| status.is_terminal()).collect();
        assert_eq!(
            terminal,
            [ReviewJobStatus::Posted, ReviewJobStatus::Failed, ReviewJobStatus::Cancelled, ReviewJobStatus::Interrupted]
        );
    }
}
