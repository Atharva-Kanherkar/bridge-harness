//! The session-forest snapshot: `sessions/get_session_forest`'s result — the
//! per-session conversation tree, worker fleet, usage ledger, and completion
//! state. Mirrors the `bridge_core::model` and `bridge_core::completion`
//! DTOs; the drift gate in bridge-core's `protocol_mirror` keeps the wire
//! values in lockstep.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::common::{JsSafeI64, JsSafeU64};
use super::completion::CheckRun;
use super::state::{BridgeEvent, ResumeEligibility, RestorationMode};

/// Mirrors `bridge_core::model::SessionEntry` — one immutable node of the
/// conversation tree. `payload` is the stored document; its inner schema is
/// versioned by `semanticSchemaVersion`, not by this contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionEntry {
    pub id: String,
    pub session_id: String,
    pub parent_entry_id: Option<String>,
    pub sequence: JsSafeI64,
    pub semantic_schema_version: JsSafeI64,
    pub kind: String,
    pub payload: Value,
    pub provider_event_id: Option<String>,
    pub context_visibility: String,
    pub token_estimate: Option<JsSafeI64>,
    pub created_at: String,
}

/// Mirrors `bridge_core::model::SessionHead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionHead {
    pub session_id: String,
    pub active_entry_id: Option<String>,
    pub native_provider_session_id: Option<String>,
    pub restoration_mode: RestorationMode,
    pub resume_eligibility: ResumeEligibility,
    pub latest_checkpoint_entry_id: Option<String>,
    pub updated_at: String,
}

/// Mirrors `bridge_core::model::WorkerLease`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkerLease {
    pub session_id: String,
    pub workspace_id: String,
    pub role: String,
    pub capability_tier: String,
    pub task_family: String,
    pub owned_paths: Value,
    pub write_mode: String,
    pub lease_status: String,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Mirrors `bridge_core::model::WorkerRuntimeRecord`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRuntimeRecord {
    pub session_id: String,
    pub parent_session_id: String,
    pub lifecycle_state: String,
    pub task_family: String,
    pub compatibility_key: String,
    pub result_status: String,
    pub retry_count: JsSafeI64,
    pub warm_until: Option<String>,
    pub worktree_path: Option<String>,
    pub worktree_branch: Option<String>,
    pub last_result: Option<Value>,
    pub last_activity_at: Option<String>,
    /// When the worker entered `waiting`, and why (e.g. `approval_requested`).
    pub waiting_since: Option<String>,
    pub waiting_reason: Option<String>,
    /// One line of "what it is doing right now", derived from the worker's
    /// own event stream.
    pub progress_summary: Option<String>,
    /// Bridge's own verdict on a failure: `stalled`, `protocol_invalid`,
    /// `transient` or `permanent`. Sent as a classification so surfaces do not
    /// each re-derive one by pattern-matching the summary.
    #[serde(default)]
    pub failure_class: Option<String>,
    pub updated_at: String,
}

/// Mirrors `bridge_core::model::QueuedWorkerRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueuedWorkerRequest {
    pub id: String,
    pub parent_session_id: String,
    pub workspace_id: String,
    pub turn_id: String,
    pub request: Value,
    pub actual_model: String,
    pub queue_status: String,
    pub sequence: JsSafeI64,
    pub dispatched_session_id: Option<String>,
    pub attempt_count: JsSafeI64,
    pub expires_at: String,
    pub blocked_at: Option<String>,
    pub claimed_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Mirrors `bridge_core::model::UsageLedgerRow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageLedgerRow {
    pub id: JsSafeI64,
    pub workspace_id: String,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub input_tokens: Option<JsSafeI64>,
    pub output_tokens: Option<JsSafeI64>,
    pub cache_read_tokens: Option<JsSafeI64>,
    pub cache_write_tokens: Option<JsSafeI64>,
    pub uncached_input_tokens: Option<JsSafeI64>,
    pub context_percent: Option<JsSafeI64>,
    pub capability_units: JsSafeI64,
    pub runtime_ms: Option<JsSafeI64>,
    pub cost_microusd: Option<JsSafeI64>,
    pub cost_source: Option<String>,
    pub stable_prefix_id: Option<String>,
    pub stable_prefix_hash: Option<String>,
    pub prompt_schema_version: Option<JsSafeI64>,
    pub prefix_token_estimate: Option<JsSafeI64>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    pub task_family: Option<String>,
    pub restoration_mode: Option<String>,
    pub cross_harness_reuse: Option<String>,
    pub reasoning_tokens: Option<JsSafeI64>,
    pub serving_model: Option<String>,
    pub context_window_tokens: Option<JsSafeI64>,
    pub context_used_tokens: Option<JsSafeI64>,
    pub provider_record_id: Option<String>,
    pub cache_savings_microusd: Option<JsSafeI64>,
    pub source: String,
    pub created_at: String,
}

/// Mirrors `bridge_core::model::PolicyLimits`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicyLimits {
    pub max_workers_per_turn: JsSafeI64,
    pub max_strong_workers_per_turn: JsSafeI64,
    pub max_capability_units_per_turn: JsSafeI64,
}

/// Mirrors `bridge_core::model::RepositoryDivergence`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryDivergence {
    pub status: String,
    pub selected_state: Option<Value>,
    pub current_state: Value,
}

/// Mirrors `bridge_core::completion::CompletionVerdict`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletionVerdict {
    Verifying,
    ChangesRequested,
    Verified,
    Waived,
    Failed,
    Superseded,
}

/// Mirrors `bridge_core::completion::RepositoryStamp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryStamp {
    pub head: String,
    pub dirty_digest: String,
}

/// Mirrors `bridge_core::completion::CompletionSummary` — the acceptance
/// contract's live verdict, also the result of the completion mutations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompletionSummary {
    pub attempt_id: String,
    pub contract_id: String,
    pub verdict: CompletionVerdict,
    pub repository: RepositoryStamp,
    pub passed_required: JsSafeU64,
    pub total_required: JsSafeU64,
    pub checks: Vec<CheckRun>,
    pub markdown_committed: bool,
    pub waiver_reason: Option<String>,
}

/// Mirrors `bridge_core::model::SessionForestSnapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionForestSnapshot {
    pub session_id: String,
    pub entries: Vec<SessionEntry>,
    pub head: Option<SessionHead>,
    pub leaves: Vec<SessionEntry>,
    pub worker_leases: Vec<WorkerLease>,
    pub worker_runtimes: Vec<WorkerRuntimeRecord>,
    pub worker_queue: Vec<QueuedWorkerRequest>,
    pub usage: Vec<UsageLedgerRow>,
    pub reasons: Vec<BridgeEvent>,
    pub policy_limits: PolicyLimits,
    pub repository_divergence: RepositoryDivergence,
    pub completion: Option<CompletionSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use crate::messages::completion::{CheckStatus, EvalKind};
    use serde_json::json;

    fn safe_i64(value: i64) -> JsSafeI64 {
        JsSafeI64::new(value).unwrap()
    }

    fn safe_u64(value: u64) -> JsSafeU64 {
        JsSafeU64::new(value).unwrap()
    }

    fn entry(sequence: i64) -> SessionEntry {
        SessionEntry {
            id: format!("e-{sequence}"),
            session_id: "s-1".into(),
            parent_entry_id: (sequence > 1).then(|| format!("e-{}", sequence - 1)),
            sequence: safe_i64(sequence),
            semantic_schema_version: safe_i64(2),
            kind: "assistant.message".into(),
            payload: json!({"text": "hello"}),
            provider_event_id: None,
            context_visibility: "visible".into(),
            token_estimate: Some(safe_i64(12)),
            created_at: "now".into(),
        }
    }

    #[test]
    fn the_forest_snapshot_round_trips_with_camel_case_wire_names() {
        let snapshot = SessionForestSnapshot {
            session_id: "s-1".into(),
            entries: vec![entry(1), entry(2)],
            head: Some(SessionHead {
                session_id: "s-1".into(),
                active_entry_id: Some("e-2".into()),
                native_provider_session_id: Some("prov-1".into()),
                restoration_mode: RestorationMode::Native,
                resume_eligibility: ResumeEligibility::Native,
                latest_checkpoint_entry_id: None,
                updated_at: "now".into(),
            }),
            leaves: vec![entry(2)],
            worker_leases: vec![WorkerLease {
                session_id: "worker-1".into(),
                workspace_id: "w-1".into(),
                role: "implementation".into(),
                capability_tier: "standard".into(),
                task_family: "rust".into(),
                owned_paths: json!(["src/"]),
                write_mode: "exclusive".into(),
                lease_status: "active".into(),
                expires_at: Some("later".into()),
                created_at: "now".into(),
                updated_at: "now".into(),
            }],
            worker_runtimes: vec![WorkerRuntimeRecord {
                session_id: "worker-1".into(),
                parent_session_id: "s-1".into(),
                lifecycle_state: "working".into(),
                task_family: "rust".into(),
                compatibility_key: "codex:gpt-5".into(),
                result_status: "pending".into(),
                retry_count: safe_i64(0),
                warm_until: None,
                worktree_path: Some("/worktrees/w".into()),
                worktree_branch: Some("bridge/w".into()),
                last_result: Some(json!({"ok": true})),
                last_activity_at: Some("now".into()),
                waiting_since: Some("now".into()),
                waiting_reason: Some("approval_requested".into()),
                progress_summary: Some("Running: cargo test".into()),
                failure_class: None,
                updated_at: "now".into(),
            }],
            worker_queue: vec![QueuedWorkerRequest {
                id: "q-1".into(),
                parent_session_id: "s-1".into(),
                workspace_id: "w-1".into(),
                turn_id: "turn-1".into(),
                request: json!({"objective": "fix tests"}),
                actual_model: "gpt-5".into(),
                queue_status: "queued".into(),
                sequence: safe_i64(1),
                dispatched_session_id: None,
                attempt_count: safe_i64(0),
                expires_at: "later".into(),
                blocked_at: None,
                claimed_at: None,
                last_error: None,
                created_at: "now".into(),
                updated_at: "now".into(),
            }],
            usage: vec![UsageLedgerRow {
                id: safe_i64(1),
                workspace_id: "w-1".into(),
                session_id: Some("s-1".into()),
                turn_id: Some("turn-1".into()),
                input_tokens: Some(safe_i64(1000)),
                output_tokens: Some(safe_i64(200)),
                cache_read_tokens: Some(safe_i64(800)),
                cache_write_tokens: Some(safe_i64(0)),
                uncached_input_tokens: Some(safe_i64(200)),
                context_percent: Some(safe_i64(30)),
                capability_units: safe_i64(2),
                runtime_ms: Some(safe_i64(1200)),
                cost_microusd: Some(safe_i64(310)),
                cost_source: Some("reported".into()),
                stable_prefix_id: Some("prefix-1".into()),
                stable_prefix_hash: Some("hash".into()),
                prompt_schema_version: Some(safe_i64(1)),
                prefix_token_estimate: Some(safe_i64(700)),
                harness: Some("codex".into()),
                model: Some("gpt-5".into()),
                role: Some("orchestrator".into()),
                task_family: Some("rust".into()),
                restoration_mode: Some("fresh".into()),
                cross_harness_reuse: Some("same_harness".into()),
                reasoning_tokens: Some(safe_i64(40)),
                serving_model: Some("gpt-5-mini".into()),
                context_window_tokens: Some(safe_i64(272_000)),
                context_used_tokens: Some(safe_i64(81_600)),
                provider_record_id: Some("msg_1:req_1".into()),
                cache_savings_microusd: Some(safe_i64(900)),
                source: "provider".into(),
                created_at: "now".into(),
            }],
            reasons: vec![BridgeEvent {
                id: safe_i64(4),
                source: "policy".into(),
                kind: "delegation.approved".into(),
                entity_id: "s-1".into(),
                body: "Write scope approved".into(),
                created_at: "now".into(),
            }],
            policy_limits: PolicyLimits {
                max_workers_per_turn: safe_i64(4),
                max_strong_workers_per_turn: safe_i64(1),
                max_capability_units_per_turn: safe_i64(8),
            },
            repository_divergence: RepositoryDivergence {
                status: "clean".into(),
                selected_state: None,
                current_state: json!({"head": "abc123"}),
            },
            completion: Some(CompletionSummary {
                attempt_id: "a-1".into(),
                contract_id: "c-1".into(),
                verdict: CompletionVerdict::ChangesRequested,
                repository: RepositoryStamp {
                    head: "abc123".into(),
                    dirty_digest: "sha256:d".into(),
                },
                passed_required: safe_u64(1),
                total_required: safe_u64(3),
                checks: vec![CheckRun {
                    check_id: "cargo-test".into(),
                    kind: EvalKind::Deterministic,
                    required: true,
                    status: CheckStatus::Failed,
                    executor: "shell".into(),
                    command: Some("cargo test".into()),
                    verifier_family: None,
                    detail: Some("2 failed".into()),
                    output_digest: None,
                    artifact_refs: Vec::new(),
                }],
                markdown_committed: true,
                waiver_reason: None,
            }),
        };
        let wire = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(wire["entries"][0]["parentEntryId"], json!(null));
        assert_eq!(wire["entries"][1]["parentEntryId"], json!("e-1"));
        assert_eq!(wire["head"]["resumeEligibility"], json!("native"));
        assert_eq!(wire["workerLeases"][0]["ownedPaths"], json!(["src/"]));
        assert_eq!(wire["workerQueue"][0]["queueStatus"], json!("queued"));
        assert_eq!(wire["usage"][0]["cacheReadTokens"], json!(800));
        assert_eq!(wire["policyLimits"]["maxWorkersPerTurn"], json!(4));
        assert_eq!(wire["completion"]["verdict"], json!("changes_requested"));
        assert_eq!(wire["completion"]["passedRequired"], json!(1));
        assert_eq!(round_trip(&snapshot), snapshot);
    }

    #[test]
    fn completion_verdicts_are_snake_case_and_closed() {
        assert_eq!(
            serde_json::to_value(CompletionVerdict::ChangesRequested).unwrap(),
            json!("changes_requested")
        );
        assert!(serde_json::from_value::<CompletionVerdict>(json!("approved")).is_err());
    }
}
