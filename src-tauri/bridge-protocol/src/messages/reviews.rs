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
    /// Whether a review may run on two harnesses at once. Off (the default),
    /// a review runs on one.
    pub second_opinion: bool,
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
            error: None,
            posted_review_url: Some("https://github.com/o/r/pull/42#pullrequestreview-1".into()),
            retry_of: Some("j0".into()),
            created_at: "2026-11-23T10:00:00.000Z".into(),
            started_at: None,
            finished_at: None,
        };
        let value = serde_json::to_value(&job).unwrap();
        assert_eq!(value["workspaceId"], "w1");
        assert_eq!(value["headSha"], "abc");
        assert_eq!(value["postMode"], "auto");
        assert_eq!(value["trigger"], "push");
        assert_eq!(value["status"], "drafted");
        assert_eq!(value["findingCounts"]["critical"], 1);
        assert_eq!(value["retryOf"], "j0");
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
