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

/// One step of a workflow. Phase 1 accepts exactly one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowStep {
    pub harness: HarnessId,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    pub instructions: String,
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
}

/// Why a run started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerKind {
    Manual,
    Schedule,
    /// The latest of one or more slots missed while Bridge was not running.
    CatchUp,
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
    fn save_without_an_id_creates() {
        let params: SaveWorkflowParams = serde_json::from_value(json!({ "definition": definition() })).unwrap();
        assert_eq!(params.workflow_id, None);
    }
}
