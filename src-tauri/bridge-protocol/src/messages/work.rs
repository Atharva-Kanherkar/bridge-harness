//! The work domain: the ranked board of what needs doing.
//!
//! Two layers, kept visibly apart because they are trusted differently.
//!
//! * **Facts** ([`WorkFact`]) are deterministic projections of Bridge's own
//!   SQLite. No model authors them, no model reorders them, and they render
//!   with no network. Every one carries a [`WorkFactAction`] that is legal for
//!   the state it was projected from — there is deliberately no generic
//!   "clear", because a failed check does not become passing by being
//!   dismissed.
//! * **Suggested work** ([`WorkTask`]) comes from a briefing model reading
//!   connected tools. Its types are contracted here so storage and the board
//!   have a stable shape, but nothing in this slice populates them:
//!   [`WorkBoard::tasks`] is empty and [`WorkBoard::latest_run`] is `None`
//!   until the briefing runner lands. [`WorkSuggestions`] says which it is.
//!
//! Freshness is explicit. A fact whose underlying observation needs the
//! filesystem cannot be measured on the read path, so it is served from a
//! timestamped cache and reports [`WorkFactFreshness`]. A stale or failed
//! observation is never rendered as current.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::common::{Effort, HarnessId, JsSafeU64};

// ---------------------------------------------------------------------------
// Facts
// ---------------------------------------------------------------------------

/// Which deterministic condition a fact describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkFactKind {
    /// A required or optional completion check reported `failed` on an attempt
    /// that has not been verified, waived, or superseded.
    FailedCompletionCheck,
    /// A queued worker is parked because an ancestor session is waiting on a
    /// human.
    BlockedWorkerQueueItem,
    /// An approval request nobody has answered. Past the approval deadline it
    /// is expired; before it, it is merely waiting.
    ActionableApproval,
    /// A workspace has drifted far enough behind its base branch to matter.
    WorkspaceBehindBase,
}

/// How loudly a fact asks for attention — the board's primary sort key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkFactSeverity {
    /// Work is stopped until a human acts.
    Blocking,
    /// Work continues, but something is wrong.
    Attention,
    /// Worth knowing, not worth interrupting for.
    Info,
}

/// How current the observation behind a fact is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkFactFreshness {
    /// Projected from live SQLite state, or from an observation inside the
    /// staleness window.
    Live,
    /// The observation is real but old. Its numbers describe the past.
    Stale,
    /// The last attempt to observe this failed. Nothing is known.
    Unknown,
}

/// What a fact is about, as an identity inside Bridge. Facts never carry an
/// external link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkFactTarget {
    #[serde(rename_all = "camelCase")]
    Session { session_id: String },
    #[serde(rename_all = "camelCase")]
    Workspace {
        workspace_id: String,
        session_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    CompletionAttempt {
        session_id: String,
        attempt_id: String,
    },
    #[serde(rename_all = "camelCase")]
    WorkerQueueItem {
        queue_id: String,
        workspace_id: String,
    },
}

/// The one thing a human can do about a fact, derived from the state the fact
/// was projected from.
///
/// There is no `clear` variant on purpose. A fact stops existing when the state
/// under it changes; offering to dismiss one would let the board disagree with
/// the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkFactAction {
    /// Open the failed check on its attempt. Only a new run or an explicit
    /// waiver changes a verdict, so this reviews — it does not resolve.
    #[serde(rename_all = "camelCase")]
    ReviewCompletionCheck {
        session_id: String,
        attempt_id: String,
        check_id: String,
    },
    /// Answer the approval holding this work. `approval_sequence` is the
    /// `session_entries` sequence of the request when the fact is the approval
    /// itself, and `None` when the fact is downstream of one.
    #[serde(rename_all = "camelCase")]
    AnswerApproval {
        session_id: String,
        approval_sequence: Option<i64>,
    },
    /// Fast-forward the workspace onto its base ref — `worktrees/refresh_workspace_base`.
    /// Offered only for a live observation.
    #[serde(rename_all = "camelCase")]
    RefreshWorkspaceBase {
        session_id: String,
        workspace_id: String,
    },
    /// Measure the workspace against its base again. A stale or failed
    /// observation is not something to act on; it is something to re-observe.
    #[serde(rename_all = "camelCase")]
    RefreshBaseObservation {
        session_id: String,
        workspace_id: String,
    },
}

/// One deterministic thing that needs doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkFact {
    pub kind: WorkFactKind,
    /// Stable across reads for the same underlying condition, and the board's
    /// final tie-break so ordering never depends on SQLite row order.
    pub dedupe_key: String,
    pub severity: WorkFactSeverity,
    pub title: String,
    pub detail: Option<String>,
    pub target: WorkFactTarget,
    /// When this became actionable — the board's secondary sort key, oldest
    /// first.
    pub actionable_at: String,
    /// When Bridge last saw the state behind it.
    pub observed_at: String,
    pub freshness: WorkFactFreshness,
    pub action: WorkFactAction,
}

// ---------------------------------------------------------------------------
// Suggested work — contracted here, populated by a later slice
// ---------------------------------------------------------------------------

/// Where a suggested task stands locally. `pinned` is orthogonal and lives on
/// [`WorkTask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkTaskState {
    Active,
    Snoozed,
    Done,
    Dismissed,
    Stale,
}

/// A Bridge-derived place a task's evidence can be opened. Never a
/// model-authored URL: an external link exists here only after Bridge resolved
/// it and matched it against its connector's host allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WorkEvidenceTarget {
    /// A validated HTTPS permalink, with the host Bridge matched it against.
    #[serde(rename_all = "camelCase")]
    ExternalLink { url: String, host: String },
    /// A local Bridge session.
    #[serde(rename_all = "camelCase")]
    Session { session_id: String },
}

/// One model-suggested task, after Bridge validated its identity and evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkTask {
    /// Durable row id. Ephemeral tasks have no fingerprint, so actions use this id.
    pub id: String,
    /// A versioned SHA-256 over length-prefixed connector and resource identities.
    pub fingerprint: Option<String>,
    pub connector_instance_id: String,
    pub canonical_resource_id: Option<String>,
    pub source_kind: String,
    /// Untrusted external text. Bounded, and never an instruction.
    pub title: String,
    pub why: String,
    pub rank: i64,
    pub confidence_bps: i64,
    pub state: WorkTaskState,
    pub pinned: bool,
    pub snoozed_until: Option<String>,
    pub evidence_digest: Option<String>,
    pub evidence_target: Option<WorkEvidenceTarget>,
    pub evidence_observed_at: Option<String>,
    /// Consecutive successful source-scoped misses. Two makes a task stale; a
    /// connector failure never increments it.
    pub miss_count: i64,
    pub workspace_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// What started a briefing run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkBriefTrigger {
    Manual,
    Focus,
    Schedule,
}

/// How a briefing run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkBriefRunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
    /// Refused before starting — over budget, no eligible provider, cooldown.
    Skipped,
}

/// What a briefing run cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkRunUsage {
    pub input_tokens: JsSafeU64,
    pub output_tokens: JsSafeU64,
    pub cached_input_tokens: JsSafeU64,
    /// Only when the provider exposes usage Bridge can trust.
    pub cost_microusd: Option<i64>,
    pub tool_calls: i64,
    pub turns: i64,
}

/// One briefing run's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkBriefRun {
    pub id: String,
    pub trigger: WorkBriefTrigger,
    pub status: WorkBriefRunStatus,
    /// The briefing configuration this run used, for provenance.
    pub profile_reference: Option<String>,
    /// The hidden briefing session, for **Inspect run**.
    pub session_id: Option<String>,
    /// A stable code, not a message: `schema_invalid`, `budget_exceeded`,
    /// `provider_unsupported`, …
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub output_digest: Option<String>,
    pub usage: Option<WorkRunUsage>,
    pub started_at: String,
    pub completed_at: Option<String>,
}

/// How far a connector instance got in a briefing run. These are distinct
/// states on purpose: "available" is not "was read".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkSourceStatus {
    /// Installed, but no reviewed evidence resolver — never offered to a model.
    Ineligible,
    /// Reviewed and offered to the model.
    Eligible,
    /// The model called it.
    Consulted,
    /// The call returned successfully and can back evidence.
    Succeeded,
    Failed,
    AuthRequired,
}

/// One connector instance's coverage in a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkSourceCoverage {
    pub connector_instance_id: String,
    pub connector_family: String,
    pub status: WorkSourceStatus,
    pub detail: Option<String>,
    pub observed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Hard bounds on a briefing run. Reaching one terminates the run; a preflight
/// estimate over one refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkBriefLimits {
    pub max_wall_seconds: i64,
    pub max_turns: i64,
    pub max_tool_calls: i64,
    pub max_output_tokens: Option<i64>,
    pub cost_ceiling_microusd: Option<i64>,
}

/// The briefing model, pinned explicitly. Not a `ProfilePurpose`: profile
/// validation wants exactly one profile per purpose, and briefing permits no
/// fallback at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkBriefingProfile {
    pub harness: HarnessId,
    pub model: String,
    pub effort: Option<Effort>,
}

// ---------------------------------------------------------------------------
// Local actions on a suggested task
// ---------------------------------------------------------------------------

/// What a human asked of a task.
///
/// Deliberately no `pin` variant: pinning is orthogonal to these five, so folding it in
/// would let a caller send `pin` where a state change is expected and get one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkTaskActionKind {
    Done,
    Snooze,
    Dismiss,
    Restore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskActionParams {
    pub task_id: String,
    pub action: WorkTaskActionKind,
    /// The future deadline for a snooze. Ignored by every other action.
    pub snoozed_until: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskPinParams {
    pub task_id: String,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskPrepareSessionParams {
    pub task_id: String,
    /// Which harness the prepared session will use when the user eventually sends.
    ///
    /// Chosen by the caller, the same way a new chat's harness is: availability is a
    /// frontend concern, and inventing a different default here would give a task-started
    /// session a provider the user never picks anywhere else.
    pub harness: HarnessId,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskOpenEvidenceParams {
    pub task_id: String,
}

/// A session prepared from a task, with nothing sent.
///
/// There is no turn id here, and that absence is the contract: preparing creates a draft
/// the user edits and sends. A field naming a dispatched turn would mean this call had
/// already spoken to a model on their behalf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkTaskDraft {
    pub session_id: String,
    pub title: String,
    /// The composer's starting contents, carrying untrusted task text as text.
    pub draft: String,
}

/// Work's configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkSettings {
    /// `None` means Work is facts-only. There is no implicit model.
    pub briefing: Option<WorkBriefingProfile>,
    pub enabled_connector_instances: Vec<String>,
    pub refresh_on_focus: bool,
    /// `None` disables cadence refresh; manual still works.
    pub refresh_interval_minutes: Option<i64>,
    pub cooldown_minutes: i64,
    pub limits: WorkBriefLimits,
}

/// Why suggested work looks the way it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkSuggestionsState {
    /// No briefing profile. Facts are the whole board, which is a normal way to
    /// run Work — not an error.
    NotConfigured,
    /// A profile exists, but its provider has not passed the briefing policy
    /// conformance suite. Judgments stay off for it; nothing falls back.
    ProviderUnsupported,
    Running,
    /// The last run failed or was refused; the previous board is preserved.
    Degraded,
    Ready,
}

/// The state of the suggested-work half, and a non-sensitive explanation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkSuggestions {
    pub state: WorkSuggestionsState,
    pub detail: Option<String>,
}

/// `work/get_work_board`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkBoard {
    pub generated_at: String,
    pub facts: Vec<WorkFact>,
    pub tasks: Vec<WorkTask>,
    pub latest_run: Option<WorkBriefRun>,
    pub sources: Vec<WorkSourceCoverage>,
    pub usage: Option<WorkRunUsage>,
    pub settings: WorkSettings,
    pub suggestions: WorkSuggestions,
}

// ---------------------------------------------------------------------------
// Settings round-trip and the briefing surface
// ---------------------------------------------------------------------------

/// `work/read_settings` and `work/write_settings` result.
///
/// `configured` is the row's existence, not its contents: a fresh install reads
/// defaults with `configured: false`, while a user who wrote settings with no
/// briefing profile reads `configured: true` — briefing explicitly off, which is
/// a different stored state from never having set it up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkSettingsSnapshot {
    pub configured: bool,
    pub settings: WorkSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteSettingsParams {
    pub settings: WorkSettings,
}

/// One model a briefing could run on, as Settings offers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkBriefingModel {
    pub id: String,
    pub label: String,
    /// The capability tier, `fast` being the cheapest capable one.
    pub tier: String,
    /// Whether this is the model a briefing defaults to on this harness.
    pub default_for_briefing: bool,
}

/// One harness as the briefing Settings surface sees it: certified or refused,
/// with the refusal reason stated rather than the harness hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkBriefingHarness {
    pub id: String,
    pub label: String,
    pub available: bool,
    /// Whether the adapter passed the briefing conformance gate.
    pub supported: bool,
    /// The gate's own reason when it refused. Never a substitute suggestion.
    pub reason: Option<String>,
    /// The cheapest capable model — the Fast-tier default — chosen here because
    /// the resolver deliberately refuses to invent a model at run time.
    pub default_model: Option<String>,
    pub models: Vec<WorkBriefingModel>,
}

/// `work/briefing_options`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkBriefingOptions {
    pub harnesses: Vec<WorkBriefingHarness>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunBriefingParams {
    pub trigger: WorkBriefTrigger,
}

/// What a trigger got: a run it started, a run somebody else already holds, or
/// a refusal with a stable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkBriefReceiptOutcome {
    Started,
    /// Another trigger's run is active; this one observes it rather than racing.
    Observed,
    Refused,
}

/// `work/run_briefing` and `work/cancel_briefing` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkBriefReceipt {
    pub outcome: WorkBriefReceiptOutcome,
    pub run_id: Option<String>,
    /// A stable code for a refusal — `not_configured`, `cooldown`,
    /// `provider_unsupported` — never payload or provider text.
    pub code: Option<String>,
    pub detail: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    fn limits() -> WorkBriefLimits {
        WorkBriefLimits {
            max_wall_seconds: 600,
            max_turns: 12,
            max_tool_calls: 24,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        }
    }

    fn settings() -> WorkSettings {
        WorkSettings {
            briefing: None,
            enabled_connector_instances: Vec::new(),
            refresh_on_focus: false,
            refresh_interval_minutes: None,
            cooldown_minutes: 15,
            limits: limits(),
        }
    }

    fn fact() -> WorkFact {
        WorkFact {
            kind: WorkFactKind::FailedCompletionCheck,
            dedupe_key: "completion-check:a-1:cargo-test".into(),
            severity: WorkFactSeverity::Blocking,
            title: "cargo test failed".into(),
            detail: Some("2 failing tests".into()),
            target: WorkFactTarget::CompletionAttempt {
                session_id: "s-1".into(),
                attempt_id: "a-1".into(),
            },
            actionable_at: "2026-08-19T10:00:00+00:00".into(),
            observed_at: "2026-08-19T10:00:00+00:00".into(),
            freshness: WorkFactFreshness::Live,
            action: WorkFactAction::ReviewCompletionCheck {
                session_id: "s-1".into(),
                attempt_id: "a-1".into(),
                check_id: "cargo-test".into(),
            },
        }
    }

    #[test]
    fn a_facts_only_board_round_trips_with_camel_case_wire_names() {
        let board = WorkBoard {
            generated_at: "2026-08-19T10:05:00+00:00".into(),
            facts: vec![fact()],
            tasks: Vec::new(),
            latest_run: None,
            sources: Vec::new(),
            usage: None,
            settings: settings(),
            suggestions: WorkSuggestions {
                state: WorkSuggestionsState::NotConfigured,
                detail: Some("no briefing model is configured".into()),
            },
        };
        let wire = serde_json::to_value(&board).unwrap();
        assert_eq!(wire["generatedAt"], json!("2026-08-19T10:05:00+00:00"));
        assert_eq!(wire["facts"][0]["dedupeKey"], json!("completion-check:a-1:cargo-test"));
        assert_eq!(wire["facts"][0]["actionableAt"], json!("2026-08-19T10:00:00+00:00"));
        assert_eq!(wire["facts"][0]["target"]["attemptId"], json!("a-1"));
        assert_eq!(wire["settings"]["enabledConnectorInstances"], json!([]));
        assert_eq!(wire["settings"]["limits"]["maxWallSeconds"], json!(600));
        assert_eq!(wire["suggestions"]["state"], json!("not_configured"));
        assert_eq!(round_trip(&board), board);
    }

    #[test]
    fn every_fact_action_is_a_tagged_variant_with_its_own_fields() {
        let actions = [
            (
                WorkFactAction::ReviewCompletionCheck {
                    session_id: "s".into(),
                    attempt_id: "a".into(),
                    check_id: "c".into(),
                },
                json!({"kind": "reviewCompletionCheck", "sessionId": "s", "attemptId": "a", "checkId": "c"}),
            ),
            (
                WorkFactAction::AnswerApproval {
                    session_id: "s".into(),
                    approval_sequence: Some(7),
                },
                json!({"kind": "answerApproval", "sessionId": "s", "approvalSequence": 7}),
            ),
            (
                WorkFactAction::RefreshWorkspaceBase {
                    session_id: "s".into(),
                    workspace_id: "w".into(),
                },
                json!({"kind": "refreshWorkspaceBase", "sessionId": "s", "workspaceId": "w"}),
            ),
            (
                WorkFactAction::RefreshBaseObservation {
                    session_id: "s".into(),
                    workspace_id: "w".into(),
                },
                json!({"kind": "refreshBaseObservation", "sessionId": "s", "workspaceId": "w"}),
            ),
        ];
        for (action, wire) in actions {
            assert_eq!(serde_json::to_value(&action).unwrap(), wire);
            assert_eq!(round_trip(&action), action);
        }
        assert!(
            serde_json::from_value::<WorkFactAction>(json!({"kind": "clear"})).is_err(),
            "there is no generic clear action, and an unknown kind is not one either"
        );
    }

    #[test]
    fn a_fact_target_is_never_an_external_link() {
        let target = WorkFactTarget::Workspace {
            workspace_id: "w".into(),
            session_id: None,
        };
        assert_eq!(
            serde_json::to_value(&target).unwrap(),
            json!({"kind": "workspace", "workspaceId": "w", "sessionId": null})
        );
        assert!(
            serde_json::from_value::<WorkFactTarget>(json!({"kind": "externalLink", "url": "https://example.test"}))
                .is_err()
        );
    }

    #[test]
    fn severity_freshness_and_state_wire_values_are_snake_case() {
        assert_eq!(serde_json::to_value(WorkFactSeverity::Blocking).unwrap(), json!("blocking"));
        assert_eq!(serde_json::to_value(WorkFactSeverity::Attention).unwrap(), json!("attention"));
        assert_eq!(serde_json::to_value(WorkFactSeverity::Info).unwrap(), json!("info"));
        assert_eq!(serde_json::to_value(WorkFactFreshness::Live).unwrap(), json!("live"));
        assert_eq!(serde_json::to_value(WorkFactFreshness::Stale).unwrap(), json!("stale"));
        assert_eq!(serde_json::to_value(WorkFactFreshness::Unknown).unwrap(), json!("unknown"));
        assert_eq!(
            serde_json::to_value(WorkFactKind::BlockedWorkerQueueItem).unwrap(),
            json!("blocked_worker_queue_item")
        );
        assert_eq!(serde_json::to_value(WorkTaskState::Snoozed).unwrap(), json!("snoozed"));
        assert_eq!(serde_json::to_value(WorkSourceStatus::AuthRequired).unwrap(), json!("auth_required"));
        assert_eq!(serde_json::to_value(WorkBriefRunStatus::Skipped).unwrap(), json!("skipped"));
        assert_eq!(serde_json::to_value(WorkBriefTrigger::Focus).unwrap(), json!("focus"));
        assert!(serde_json::from_value::<WorkFactSeverity>(json!("critical")).is_err());
        assert!(serde_json::from_value::<WorkTaskState>(json!("archived")).is_err());
    }

    #[test]
    fn severity_orders_blocking_before_attention_before_info() {
        let mut severities = [
            WorkFactSeverity::Info,
            WorkFactSeverity::Blocking,
            WorkFactSeverity::Attention,
        ];
        severities.sort();
        assert_eq!(
            severities,
            [
                WorkFactSeverity::Blocking,
                WorkFactSeverity::Attention,
                WorkFactSeverity::Info
            ]
        );
    }

    #[test]
    fn settings_and_limits_reject_unknown_fields() {
        let configured: WorkSettings = serde_json::from_value(json!({
            "briefing": {"harness": "claude", "model": "claude-opus-5", "effort": "high"},
            "enabledConnectorInstances": ["github:acme"],
            "refreshOnFocus": true,
            "refreshIntervalMinutes": 30,
            "cooldownMinutes": 15,
            "limits": {
                "maxWallSeconds": 600,
                "maxTurns": 12,
                "maxToolCalls": 24,
                "maxOutputTokens": null,
                "costCeilingMicrousd": null,
            },
        }))
        .unwrap();
        let briefing = configured.briefing.as_ref().expect("briefing profile");
        assert_eq!(briefing.harness.as_str(), "claude");
        assert_eq!(briefing.effort, Some(Effort::High));
        assert_eq!(round_trip(&configured), configured);

        assert!(
            serde_json::from_value::<WorkSettings>(json!({
                "briefing": null,
                "enabledConnectorInstances": [],
                "refreshOnFocus": false,
                "refreshIntervalMinutes": null,
                "cooldownMinutes": 15,
                "limits": {
                    "maxWallSeconds": 600, "maxTurns": 12, "maxToolCalls": 24,
                    "maxOutputTokens": null, "costCeilingMicrousd": null,
                },
                "writeConnectorTools": true,
            }))
            .is_err(),
            "an unknown settings field is rejected, not silently honoured"
        );
        assert!(
            serde_json::from_value::<WorkBriefLimits>(json!({
                "maxWallSeconds": 600, "maxTurns": 12, "maxToolCalls": 24,
                "maxOutputTokens": null, "costCeilingMicrousd": null,
                "maxConnectorWrites": 1,
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<WorkBriefingProfile>(json!({
                "harness": "claude", "model": "claude-opus-5", "apiKey": "sk-live",
            }))
            .is_err(),
            "the briefing profile is not a place to smuggle a credential"
        );
    }

    #[test]
    fn a_settings_snapshot_separates_configured_from_its_contents() {
        // The two states the write path must keep apart: defaults nobody wrote,
        // and briefing explicitly off.
        let fresh = WorkSettingsSnapshot { configured: false, settings: settings() };
        let switched_off = WorkSettingsSnapshot { configured: true, settings: settings() };
        assert_ne!(fresh, switched_off);
        let wire = serde_json::to_value(&switched_off).unwrap();
        assert_eq!(wire["configured"], json!(true));
        assert_eq!(wire["settings"]["briefing"], json!(null));
        assert_eq!(round_trip(&switched_off), switched_off);
    }

    #[test]
    fn briefing_options_carry_the_refusal_reason_and_the_cheapest_default() {
        let options = WorkBriefingOptions {
            harnesses: vec![
                WorkBriefingHarness {
                    id: "claude".into(),
                    label: "Claude".into(),
                    available: true,
                    supported: true,
                    reason: None,
                    default_model: Some("haiku".into()),
                    models: vec![WorkBriefingModel {
                        id: "haiku".into(),
                        label: "Claude Haiku".into(),
                        tier: "fast".into(),
                        default_for_briefing: true,
                    }],
                },
                WorkBriefingHarness {
                    id: "codex".into(),
                    label: "Codex".into(),
                    available: true,
                    supported: false,
                    reason: Some("no per-tool authority".into()),
                    default_model: None,
                    models: Vec::new(),
                },
            ],
        };
        let wire = serde_json::to_value(&options).unwrap();
        assert_eq!(wire["harnesses"][0]["defaultModel"], json!("haiku"));
        assert_eq!(wire["harnesses"][0]["models"][0]["defaultForBriefing"], json!(true));
        assert_eq!(wire["harnesses"][1]["supported"], json!(false));
        assert_eq!(wire["harnesses"][1]["reason"], json!("no per-tool authority"));
        assert_eq!(round_trip(&options), options);
    }

    #[test]
    fn a_run_receipt_round_trips_and_refuses_unknown_params() {
        let receipt = WorkBriefReceipt {
            outcome: WorkBriefReceiptOutcome::Refused,
            run_id: None,
            code: Some("cooldown".into()),
            detail: Some("the last run finished 4 minutes ago".into()),
        };
        let wire = serde_json::to_value(&receipt).unwrap();
        assert_eq!(wire["outcome"], json!("refused"));
        assert_eq!(wire["runId"], json!(null));
        assert_eq!(round_trip(&receipt), receipt);

        let params: RunBriefingParams =
            serde_json::from_value(json!({"trigger": "focus"})).unwrap();
        assert_eq!(params.trigger, WorkBriefTrigger::Focus);
        assert!(
            serde_json::from_value::<RunBriefingParams>(
                json!({"trigger": "focus", "force": true})
            )
            .is_err(),
            "an unknown params field is rejected, not silently honoured"
        );
        assert!(
            serde_json::from_value::<WriteSettingsParams>(
                json!({"settings": serde_json::to_value(settings()).unwrap(), "actor": "model"})
            )
            .is_err()
        );
    }

    #[test]
    fn a_task_carries_bridge_derived_identity_and_only_validated_evidence() {
        let task = WorkTask {
            id: "task-a".into(),
            fingerprint: Some("a".repeat(64)),
            connector_instance_id: "github:acme".into(),
            canonical_resource_id: Some("acme/bridge#204".into()),
            source_kind: "github_issue".into(),
            title: "Review the migration".into(),
            why: "open three days with a requested change".into(),
            rank: 1,
            confidence_bps: 8200,
            state: WorkTaskState::Active,
            pinned: false,
            snoozed_until: None,
            evidence_digest: Some("sha256:abc".into()),
            evidence_target: Some(WorkEvidenceTarget::ExternalLink {
                url: "https://github.com/acme/bridge/issues/204".into(),
                host: "github.com".into(),
            }),
            evidence_observed_at: Some("2026-08-19T09:00:00+00:00".into()),
            miss_count: 0,
            workspace_id: None,
            created_at: "2026-08-19T09:00:00+00:00".into(),
            updated_at: "2026-08-19T09:00:00+00:00".into(),
        };
        let wire = serde_json::to_value(&task).unwrap();
        assert_eq!(wire["canonicalResourceId"], json!("acme/bridge#204"));
        assert_eq!(wire["confidenceBps"], json!(8200));
        assert_eq!(wire["evidenceTarget"]["kind"], json!("externalLink"));
        assert_eq!(wire["evidenceTarget"]["host"], json!("github.com"));
        assert_eq!(round_trip(&task), task);
    }

    #[test]
    fn a_run_reports_its_usage_and_a_stable_failure_code() {
        let run = WorkBriefRun {
            id: "run-1".into(),
            trigger: WorkBriefTrigger::Manual,
            status: WorkBriefRunStatus::Failed,
            profile_reference: Some("work:settings".into()),
            session_id: Some("briefing-1".into()),
            failure_code: Some("schema_invalid".into()),
            failure_detail: Some("two repair attempts would be one too many".into()),
            output_digest: Some("sha256:def".into()),
            usage: Some(WorkRunUsage {
                input_tokens: 1_200u64.try_into().unwrap(),
                output_tokens: 340u64.try_into().unwrap(),
                cached_input_tokens: 0u64.try_into().unwrap(),
                cost_microusd: Some(4_100),
                tool_calls: 6,
                turns: 2,
            }),
            started_at: "2026-08-19T09:00:00+00:00".into(),
            completed_at: Some("2026-08-19T09:01:00+00:00".into()),
        };
        let wire = serde_json::to_value(&run).unwrap();
        assert_eq!(wire["failureCode"], json!("schema_invalid"));
        assert_eq!(wire["usage"]["inputTokens"], json!(1_200));
        assert_eq!(wire["usage"]["costMicrousd"], json!(4_100));
        assert_eq!(round_trip(&run), run);

        let coverage = WorkSourceCoverage {
            connector_instance_id: "slack:acme".into(),
            connector_family: "slack".into(),
            status: WorkSourceStatus::AuthRequired,
            detail: Some("reconnect Slack".into()),
            observed_at: None,
        };
        assert_eq!(
            serde_json::to_value(&coverage).unwrap()["connectorInstanceId"],
            json!("slack:acme")
        );
        assert_eq!(round_trip(&coverage), coverage);
    }
}
