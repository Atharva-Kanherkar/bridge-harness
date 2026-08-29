//! Read-only GitHub pull-request surface payloads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubStatusParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GithubAvailability {
    Available,
    NotInstalled,
    NotAuthenticated { remediation: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubStatusResult {
    pub availability: GithubAvailability,
    pub repository: Option<GithubRepository>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubPrsParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubPrParams {
    pub workspace_id: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubChecksParams {
    pub workspace_id: String,
    pub number: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReviewDecision {
    None,
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Mergeability {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubActor {
    pub login: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubLabel {
    pub name: String,
    pub color: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubComment {
    pub id: String,
    pub author: Option<GithubActor>,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestDetail {
    pub summary: PullRequestSummary,
    pub body: String,
    pub base_branch: String,
    pub comments: Vec<GithubComment>,
    pub labels: Vec<GithubLabel>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestFile {
    pub path: String,
    pub previous_path: Option<String>,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubPullRequestsResult {
    pub pull_requests: Vec<PullRequestSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubPullRequestResult {
    pub pull_request: PullRequestDetail,
    pub review_threads: Vec<ReviewThread>,
    pub files: Vec<PullRequestFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum IssueState {
    Open,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssueDetail {
    pub summary: IssueSummary,
    pub body: String,
    pub comments: Vec<GithubComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubIssuesParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssuesResult {
    pub issues: Vec<IssueSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubIssueParams {
    pub workspace_id: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssueResult {
    pub issue: IssueDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubRepositoryParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepositoryResult {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum GithubCheckStatus {
    Queued,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum GithubCheckConclusion {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestCheck {
    pub name: String,
    pub status: GithubCheckStatus,
    pub conclusion: Option<GithubCheckConclusion>,
    pub log_url: String,
    pub workflow: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubChecksResult {
    pub checks: Vec<PullRequestCheck>,
}

// --- mutating actions (slice 4) --------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubMergeConfigParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MergeStrategies {
    pub merge: bool,
    pub squash: bool,
    pub rebase: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubMergeConfigResult {
    pub strategies: MergeStrategies,
    pub default_strategy: MergeStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LabelTarget {
    PullRequest,
    Issue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LabelOperation {
    Add,
    Remove,
}

/// One mutating GitHub action. The `kind` tag selects the variant; every write
/// path is expressed here so a client cannot smuggle a free-form `gh` argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum GithubAction {
    Merge { number: u64, strategy: MergeStrategy },
    Review { number: u64, event: ReviewEvent, body: String },
    Reply {
        number: u64,
        // `rename_all` does not reach struct-variant fields; name it explicitly
        // so the wire stays camelCase like every other payload.
        #[serde(rename = "commentId")]
        comment_id: u64,
        body: String,
    },
    Rerun { number: u64 },
    Label {
        target: LabelTarget,
        number: u64,
        label: String,
        operation: LabelOperation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubActParams {
    pub workspace_id: String,
    pub action: GithubAction,
    /// The outcome of the native per-action confirmation. `false` means the
    /// user declined; the backend then executes nothing.
    pub confirmed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubActResult {
    /// Whether the action actually ran. `false` when the approval was denied.
    pub executed: bool,
    /// A short outcome statement, or the reason it did not run.
    pub message: String,
}

// --- checkout into a task worktree (slice 5) --------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GithubCheckoutParams {
    pub workspace_id: String,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GithubCheckoutResult {
    /// The workspace node the checkout registered (or found) — always distinct
    /// from the source workspace the PR was viewed from.
    pub workspace_id: String,
    pub path: String,
    pub branch: String,
    /// `true` when a previous checkout's worktree/node was reused.
    pub reused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn github_checkout_payloads_round_trip_camel_case_and_reject_unknown_fields() {
        let params = GithubCheckoutParams {
            workspace_id: "workspace-1".into(),
            number: 344,
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({"workspaceId": "workspace-1", "number": 344})
        );
        assert_eq!(round_trip(&params), params);
        assert!(serde_json::from_value::<GithubCheckoutParams>(
            json!({"workspaceId": "w", "number": 1, "extra": true})
        )
        .is_err());
        let result = GithubCheckoutResult {
            workspace_id: "workspace-2".into(),
            path: "/data/worktrees/github/pr-344-feat-x".into(),
            branch: "feat/x".into(),
            reused: true,
        };
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn github_read_payloads_use_camel_case_and_inert_data_shapes() {
        let params = GithubPrParams {
            workspace_id: "workspace-1".into(),
            number: 341,
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({"workspaceId": "workspace-1", "number": 341})
        );
        assert_eq!(round_trip(&params), params);
        assert!(serde_json::from_value::<GithubChecksParams>(
            json!({"workspaceId": "w", "number": 1, "extra": true})
        )
        .is_err());
        let status = GithubStatusResult {
            availability: GithubAvailability::NotAuthenticated {
                remediation: "gh auth login".into(),
            },
            repository: None,
        };
        assert_eq!(round_trip(&status), status);

        let issue = GithubIssueParams { workspace_id: "workspace-1".into(), number: 17 };
        assert_eq!(serde_json::to_value(&issue).unwrap(), json!({"workspaceId": "workspace-1", "number": 17}));
        assert_eq!(round_trip(&issue), issue);
        assert!(serde_json::from_value::<GithubRepositoryParams>(
            json!({"workspaceId": "w", "extra": true})
        ).is_err());
    }

    #[test]
    fn github_act_payloads_round_trip_camel_case_and_reject_unknown_fields() {
        let params = GithubActParams {
            workspace_id: "workspace-1".into(),
            action: GithubAction::Merge {
                number: 328,
                strategy: MergeStrategy::Squash,
            },
            confirmed: true,
        };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({
                "workspaceId": "workspace-1",
                "action": {"kind": "merge", "number": 328, "strategy": "squash"},
                "confirmed": true,
            })
        );
        assert_eq!(round_trip(&params), params);

        let reply = GithubActParams {
            workspace_id: "w".into(),
            action: GithubAction::Reply {
                number: 341,
                comment_id: 7,
                body: "thanks".into(),
            },
            confirmed: false,
        };
        assert_eq!(round_trip(&reply), reply);

        let label = GithubAction::Label {
            target: LabelTarget::PullRequest,
            number: 341,
            label: "bug".into(),
            operation: LabelOperation::Add,
        };
        assert_eq!(
            serde_json::to_value(&label).unwrap(),
            json!({"kind": "label", "target": "pullRequest", "number": 341, "label": "bug", "operation": "add"})
        );
        assert_eq!(round_trip(&label), label);

        // Unknown fields are rejected at both the params and the action level.
        assert!(serde_json::from_value::<GithubActParams>(
            json!({"workspaceId": "w", "action": {"kind": "rerun", "number": 1}, "confirmed": true, "extra": 1})
        )
        .is_err());
        assert!(serde_json::from_value::<GithubAction>(
            json!({"kind": "rerun", "number": 1, "extra": 1})
        )
        .is_err());

        let config = GithubMergeConfigResult {
            strategies: MergeStrategies {
                merge: true,
                squash: true,
                rebase: false,
            },
            default_strategy: MergeStrategy::Squash,
        };
        assert_eq!(round_trip(&config), config);

        let acted = GithubActResult {
            executed: false,
            message: "Declined: merge PR #328 (squash)".into(),
        };
        assert_eq!(round_trip(&acted), acted);
    }
}
