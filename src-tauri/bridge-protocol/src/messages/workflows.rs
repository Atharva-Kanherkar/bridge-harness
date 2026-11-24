//! The workflows domain: repeatable jobs Bridge runs on the user's own harness.
//!
//! A workflow is a saved definition plus an approval. Every edit is a new,
//! immutable revision; an edit that widens what the workflow may touch clears
//! approval, and nothing runs unapproved. Connectors are granted per server
//! (a harness presents its exact tools only once a session starts), and exact
//! tool identities appear only in `alwaysAllow`, recorded when a person chose
//! "always allow for this workflow" on a real call.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{HarnessId, WorkBriefLimits, WorkBriefingModel};

/// What starts a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkflowTrigger {
    /// Only Run now.
    Manual,
    /// A 5-field cron expression read in an IANA timezone.
    Schedule { cron: String, timezone: String },
    /// Something happened in an app. Slack events come from the connector
    /// Inbox's ledger; GitHub events from `gh`. Neither costs a model turn.
    Event {
        event: WorkflowEventKind,
        /// `owner/name`. Needed by the GitHub pull-request and issue events,
        /// optional for review requests.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repo: Option<String>,
        /// Slack only: the channel's label must contain this, any case.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
        /// Slack only: the author's name must contain this, any case.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        /// Collect events and run once at each slot of this schedule, instead
        /// of once per event.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        batch: Option<WorkflowBatch>,
    },
}

/// The events a workflow can start on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEventKind {
    /// Someone @-mentioned the account owner in Slack.
    SlackMention,
    /// A Slack direct message to the account owner.
    SlackDirectMessage,
    /// A reply in a Slack thread the account owner is part of.
    SlackThreadReply,
    /// A pull request was opened in `repo`.
    GithubPullRequestOpened,
    /// An issue was opened in `repo`.
    GithubIssueOpened,
    /// The person's review was requested on a pull request (in `repo`, if set).
    GithubReviewRequested,
    /// A new email arrived in the Gmail inbox (`from` filters the sender).
    GmailMessageReceived,
    /// A Linear issue was assigned to the person (`channel` filters the team).
    LinearIssueAssigned,
    /// A page was added to a Notion database (`channel` names the database
    /// and is required; `from` filters who created the page).
    NotionPageCreated,
}

/// Run collected events at each slot of a schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowBatch {
    pub cron: String,
    pub timezone: String,
}

/// One event, as a run, a preview or a dry run carries it. `payload` is the
/// source's own record and is untrusted: other people wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowEventSample {
    /// Stable across re-reads: how an event is recognised again.
    pub key: String,
    pub kind: WorkflowEventKind,
    /// A short line for a person: `#eng · Alex: can you look at…`.
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    pub occurred_at: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// Whether a granted connector can be reached right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowConnectorState {
    Connected,
    /// Configured, but Claude reports it needs the person to sign in again.
    NeedsSignIn,
    /// Not in Claude's configuration any more.
    Missing,
    /// Bridge has not been able to ask yet.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowConnectorStatus {
    pub server: String,
    pub state: WorkflowConnectorState,
}

/// Whether a granted server may be written to, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowWriteAccess {
    /// No write tool on this server is reachable.
    Never,
    /// Every write asks a person first, unless its exact identity is in
    /// `alwaysAllow`.
    Ask,
}

/// Which built-in file, shell and web tools a run gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowFiles {
    /// None: built-in tool families are denied, as in a briefing run.
    None,
    /// A fresh task worktree of the workflow's project, under the Isolated
    /// write mode. Needs a project.
    Worktree,
}

/// One connector server a workflow may use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowGrant {
    /// The server's id in the harness's own configuration.
    pub server: String,
    /// Read-verb tools on this server run without asking.
    pub read: bool,
    pub write: WorkflowWriteAccess,
}

/// One step of a workflow. Steps run in order, each in its own session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowStep {
    pub harness: HarnessId,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub instructions: String,
    /// The workflow's granted servers this step may use. Unset means every
    /// grant, as in a single-step workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<Vec<String>>,
    /// Whether this step works in the run's worktree. Unset means the
    /// workflow's own `files`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<WorkflowFiles>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowNotify {
    /// Failures and approval requests always notify; successes only when set.
    #[serde(default)]
    pub on_success: bool,
}

/// Everything a person edits. Stored whole on every revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDefinition {
    pub name: String,
    /// `None` runs the workflow in a private folder Bridge manages.
    #[serde(default)]
    pub project_id: Option<String>,
    pub steps: Vec<WorkflowStep>,
    pub trigger: WorkflowTrigger,
    #[serde(default)]
    pub grants: Vec<WorkflowGrant>,
    /// Exact `mcp__<server>__<tool>` identities that never ask.
    #[serde(default)]
    pub always_allow: Vec<String>,
    pub files: WorkflowFiles,
    pub budget: WorkBriefLimits,
    #[serde(default)]
    pub expected_result: String,
    #[serde(default)]
    pub notify: WorkflowNotify,
}

/// A workflow at its current revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub enabled: bool,
    /// Why Bridge turned it off, when it did (`project_removed`).
    pub disabled_reason: Option<String>,
    pub revision_id: String,
    pub revision: i64,
    /// Whether the current revision is approved. Nothing runs unapproved.
    pub approved: bool,
    pub approved_at: Option<String>,
    pub definition: WorkflowDefinition,
    pub created_at: String,
    pub updated_at: String,
}

/// One row of the workflow list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSummary {
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub approved: bool,
    pub trigger: WorkflowTrigger,
    pub harness: HarnessId,
    pub model: String,
    pub files: WorkflowFiles,
    pub updated_at: String,
    /// The next scheduled fire time, when the workflow could run on one.
    pub next_run_at: Option<String>,
    pub last_run: Option<WorkflowRun>,
    /// Each granted connector, and whether it can be reached.
    #[serde(default)]
    pub connectors: Vec<WorkflowConnectorStatus>,
    /// The mean cost of its last finished runs with a known cost.
    #[serde(default)]
    pub average_run_cost_microusd: Option<i64>,
    /// That mean times its expected runs in 30 days.
    #[serde(default)]
    pub monthly_cost_estimate_microusd: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowList {
    pub workflows: Vec<WorkflowSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSnapshot {
    pub workflow: Workflow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveWorkflowResult {
    pub workflow: Workflow,
    /// True when this save widened authority and the workflow waits for
    /// `approve_workflow`.
    pub needs_approval: bool,
}

/// What a harness may do as a workflow runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowHarnessOption {
    pub id: String,
    pub label: String,
    pub available: bool,
    /// Connector grants, and runs with no built-in tools, need an adapter that
    /// can enforce exact tool identities.
    pub connectors_allowed: bool,
    /// The gate's own reason when it refused.
    pub connectors_reason: Option<String>,
    /// Project worktree jobs with no connector grants.
    pub worktree_allowed: bool,
    pub default_model: Option<String>,
    pub models: Vec<WorkBriefingModel>,
}

/// A connector server the harness's own configuration holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowServerOption {
    pub id: String,
    /// `slack`, `gmail`, `github`, `linear`, `notion`, or `unknown`.
    pub family: String,
    /// `None` when the harness did not report health for it.
    pub connected: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowOptions {
    pub harnesses: Vec<WorkflowHarnessOption>,
    pub servers: Vec<WorkflowServerOption>,
    /// Why `servers` may be incomplete: Bridge could not ask Claude just now.
    #[serde(default)]
    pub connectors_error: Option<String>,
}

/// Why a run started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerKind {
    Manual,
    Schedule,
    /// The latest of one or more slots missed while Bridge was not running.
    CatchUp,
    /// One event, or a batch of them.
    Event,
}

/// Where a run is. The first four are active: at most one per workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    Queued,
    WaitingQuota,
    Running,
    WaitingApproval,
    Succeeded,
    /// The session ended without the result block the workflow asks for.
    Unverified,
    Failed,
    Cancelled,
    /// Recorded, never run: another run of this workflow was active.
    Skipped,
}

impl WorkflowRunStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::WaitingQuota | Self::Running | Self::WaitingApproval)
    }
}

/// One firing of a workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRun {
    pub id: String,
    pub workflow_id: String,
    pub revision_id: String,
    pub trigger_kind: WorkflowTriggerKind,
    pub missed_slots: i64,
    pub dry_run: bool,
    pub status: WorkflowRunStatus,
    pub failure_code: Option<String>,
    pub summary: Option<String>,
    pub session_id: Option<String>,
    /// The provider reported no usage, so the quota reserve could not be checked.
    pub quota_unknown: bool,
    pub cost_microusd: Option<i64>,
    pub tokens: Option<i64>,
    pub unread: bool,
    pub scheduled_for: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
    /// The event that started it (the first, for a batch).
    #[serde(default)]
    pub event: Option<WorkflowEventSample>,
    /// How many events it carries: 1 for one event, more for a batch.
    #[serde(default)]
    pub event_count: i64,
    /// The steps that started, in order. Steps not listed have not started.
    #[serde(default)]
    pub steps: Vec<WorkflowRunStep>,
}

/// One started step of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunStep {
    /// 0 for the first step.
    pub index: i64,
    pub harness: HarnessId,
    pub model: String,
    /// `running` or `waiting_approval` while it works, then how it ended.
    pub status: WorkflowRunStatus,
    pub session_id: String,
    pub failure_code: Option<String>,
    pub summary: Option<String>,
    pub tokens: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSnapshot {
    pub run: WorkflowRun,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunList {
    pub runs: Vec<WorkflowRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunWorkflowResult {
    pub run: WorkflowRun,
    /// False when this request's id, or an active run, already had a run.
    pub created: bool,
}

/// Settings for every workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowSettings {
    /// No workflow starts while paused; active runs finish.
    pub paused: bool,
    /// A run waits while its provider is above `100 - this` percent of any
    /// usage window. 0 to 90.
    pub quota_reserve_percent: i64,
    /// At most this many runs work at once; the rest wait in line. 1 to 5.
    #[serde(default = "default_max_concurrent_runs")]
    pub max_concurrent_runs: i64,
}

pub const DEFAULT_MAX_CONCURRENT_RUNS: i64 = 2;

fn default_max_concurrent_runs() -> i64 {
    DEFAULT_MAX_CONCURRENT_RUNS
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSettingsSnapshot {
    pub settings: WorkflowSettings,
    /// False on a fresh install, where `settings` are the defaults.
    pub configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunWorkflowParams {
    pub workflow_id: String,
    /// A client-chosen id: the same id twice is one run.
    pub request_id: String,
    #[serde(default)]
    pub dry_run: bool,
    /// A past event to try a dry run on, from `preview_workflow_trigger`.
    /// Refused on a real run.
    #[serde(default)]
    pub event: Option<WorkflowEventSample>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewWorkflowTriggerParams {
    pub trigger: WorkflowTrigger,
}

/// What a trigger would have done lately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTriggerPreview {
    /// False when Bridge cannot read this source now; `reason` says why.
    pub supported: bool,
    pub reason: Option<String>,
    pub window_days: i64,
    /// Events in the window that would have started a run.
    pub matches: i64,
    /// The newest few, for a dry run.
    pub samples: Vec<WorkflowEventSample>,
    /// A caveat for the count, such as how far back Bridge has history.
    pub note: Option<String>,
}

/// A ready-made workflow a person can try, customise and approve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Connector families it reads or writes: `slack`, `gmail`, …
    pub needs: Vec<String>,
    /// The ones the person has not connected in Claude.
    pub missing: Vec<String>,
    /// It works on a project's code, so a project must be chosen.
    pub needs_project: bool,
    /// Every connector is connected and nothing is left to choose.
    pub ready: bool,
    /// A draft built with the person's own servers, model and timezone.
    pub definition: WorkflowDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTemplateList {
    pub templates: Vec<WorkflowTemplate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelWorkflowRunParams {
    pub run_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListWorkflowRunsParams {
    #[serde(default)]
    pub workflow_id: Option<String>,
    #[serde(default)]
    pub unread_only: bool,
    /// 1 to 200.
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteWorkflowSettingsParams {
    pub settings: WorkflowSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkWorkflowRunsReadParams {
    pub run_ids: Vec<String>,
}

/// A prefilled, unsaved workflow and what drafting could not do for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDraft {
    pub definition: WorkflowDefinition,
    /// Plain sentences for the person: an app to connect, writes held back,
    /// a schedule that did not parse.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDraftResult {
    pub draft: WorkflowDraft,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftWorkflowFromTextParams {
    pub text: String,
    #[serde(default)]
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DraftWorkflowFromSessionParams {
    pub session_id: String,
}

/// "Always allow for this workflow" on a pending card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AlwaysAllowToolParams {
    pub workflow_id: String,
    /// The run's session, which holds the pending card.
    pub session_id: String,
    /// The card's durable sequence.
    pub event_id: i64,
    /// Must be the tool the card asks about.
    pub tool_identity: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AlwaysAllowToolResult {
    pub workflow: Workflow,
    pub resolution: super::InteractionResolutionResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetWorkflowParams {
    pub workflow_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveWorkflowParams {
    /// Absent creates a workflow; present adds a revision to it.
    #[serde(default)]
    pub workflow_id: Option<String>,
    pub definition: WorkflowDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveWorkflowParams {
    pub workflow_id: String,
    /// The revision the person looked at. A stale id is refused.
    pub revision_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteWorkflowParams {
    pub workflow_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetWorkflowEnabledParams {
    pub workflow_id: String,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn definition() -> serde_json::Value {
        json!({
            "name": "Morning brief",
            "projectId": null,
            "steps": [{ "harness": "claude", "model": "haiku", "instructions": "Summarise my mentions." }],
            "trigger": { "kind": "schedule", "cron": "0 9 * * 1-5", "timezone": "Asia/Kolkata" },
            "grants": [{ "server": "claude_ai_Slack", "read": true, "write": "ask" }],
            "alwaysAllow": ["mcp__claude_ai_Slack__slack_send_message"],
            "files": "none",
            "budget": { "maxWallSeconds": 600, "maxTurns": 24, "maxToolCalls": 64, "maxOutputTokens": null, "costCeilingMicrousd": 500000 },
            "expectedResult": "Five lines or fewer.",
            "notify": { "onSuccess": false }
        })
    }

    #[test]
    fn workflow_definition_round_trips() {
        let value = definition();
        let parsed: WorkflowDefinition = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed.steps.len(), 1);
        assert_eq!(parsed.trigger, WorkflowTrigger::Schedule { cron: "0 9 * * 1-5".into(), timezone: "Asia/Kolkata".into() });
        assert_eq!(parsed.grants[0].write, WorkflowWriteAccess::Ask);
        assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
    }

    #[test]
    fn a_manual_trigger_is_just_its_kind() {
        let trigger: WorkflowTrigger = serde_json::from_value(json!({ "kind": "manual" })).unwrap();
        assert_eq!(trigger, WorkflowTrigger::Manual);
        assert_eq!(serde_json::to_value(WorkflowTrigger::Manual).unwrap(), json!({ "kind": "manual" }));
    }

    #[test]
    fn unknown_fields_are_refused() {
        let mut value = definition();
        value["surprise"] = json!(true);
        assert!(serde_json::from_value::<WorkflowDefinition>(value).is_err());
        assert!(serde_json::from_value::<SaveWorkflowParams>(json!({ "definition": definition(), "extra": 1 })).is_err());
        assert!(serde_json::from_value::<GetWorkflowParams>(json!({ "workflowId": "w", "x": 1 })).is_err());
        assert!(serde_json::from_value::<ApproveWorkflowParams>(json!({ "workflowId": "w", "revisionId": "r", "x": 1 })).is_err());
        assert!(serde_json::from_value::<DeleteWorkflowParams>(json!({ "workflowId": "w", "x": 1 })).is_err());
        assert!(serde_json::from_value::<SetWorkflowEnabledParams>(json!({ "workflowId": "w", "enabled": true, "x": 1 })).is_err());
        assert!(serde_json::from_value::<WorkflowTrigger>(json!({ "kind": "schedule", "cron": "* * * * *", "timezone": "UTC", "x": 1 })).is_err());
    }

    #[test]
    fn run_params_default_to_a_real_run_and_refuse_unknown_fields() {
        let params: RunWorkflowParams = serde_json::from_value(json!({ "workflowId": "w", "requestId": "r" })).unwrap();
        assert!(!params.dry_run);
        assert!(serde_json::from_value::<RunWorkflowParams>(json!({ "workflowId": "w", "requestId": "r", "x": 1 })).is_err());
        assert!(serde_json::from_value::<CancelWorkflowRunParams>(json!({ "runId": "r", "x": 1 })).is_err());
        assert!(serde_json::from_value::<ListWorkflowRunsParams>(json!({ "limit": 5, "x": 1 })).is_err());
        assert!(serde_json::from_value::<WriteWorkflowSettingsParams>(json!({ "settings": { "paused": false, "quotaReservePercent": 25, "x": 1 } })).is_err());
        assert_eq!(serde_json::to_value(WorkflowRunStatus::WaitingApproval).unwrap(), json!("waiting_approval"));
        assert_eq!(serde_json::to_value(WorkflowTriggerKind::CatchUp).unwrap(), json!("catch_up"));
    }

    #[test]
    fn a_step_names_its_own_servers_and_files_only_when_set() {
        let mut value = definition();
        value["steps"][0]["servers"] = json!(["claude_ai_Slack"]);
        value["steps"].as_array_mut().unwrap().push(json!({
            "harness": "codex", "model": "gpt-5.5", "instructions": "Fix it.", "servers": [], "files": "worktree"
        }));
        let parsed: WorkflowDefinition = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(parsed.steps[0].servers.as_deref(), Some(&["claude_ai_Slack".to_owned()][..]));
        assert_eq!(parsed.steps[0].files, None);
        assert_eq!(parsed.steps[1].files, Some(WorkflowFiles::Worktree));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
        // Unset is absent, so a Phase 1 step reads back as it was written.
        let plain: WorkflowDefinition = serde_json::from_value(definition()).unwrap();
        assert_eq!(serde_json::to_value(&plain).unwrap(), definition());
        let mut unknown = definition();
        unknown["steps"][0]["grants"] = json!([]);
        assert!(serde_json::from_value::<WorkflowDefinition>(unknown).is_err(), "a step still refuses unknown fields");
    }

    #[test]
    fn a_run_from_before_steps_reads_with_none() {
        let run = json!({
            "id": "r", "workflowId": "w", "revisionId": "v", "triggerKind": "manual", "missedSlots": 0, "dryRun": false,
            "status": "succeeded", "failureCode": null, "summary": null, "sessionId": "s", "quotaUnknown": false,
            "costMicrousd": null, "tokens": null, "unread": false, "scheduledFor": null, "startedAt": null,
            "finishedAt": null, "createdAt": "now"
        });
        let parsed: WorkflowRun = serde_json::from_value(run).unwrap();
        assert!(parsed.steps.is_empty());
    }

    #[test]
    fn save_without_an_id_creates() {
        let params: SaveWorkflowParams = serde_json::from_value(json!({ "definition": definition() })).unwrap();
        assert_eq!(params.workflow_id, None);
    }
}
