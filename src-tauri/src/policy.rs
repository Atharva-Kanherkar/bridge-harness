use crate::{
    delegation::{CapabilityTier, DelegationRequest, Effort, WorkerRole, WriteMode},
    model::{UsageLedgerRow, WorkerLease},
    session_forest::{EntryKind, SessionForest},
    store, BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecision {
    ExecuteInParent,
    ResumeWorker { session_id: String },
    SpawnWorker(WorkerSpec),
    Queue,
    Reject,
    RequireUserApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteReason {
    ParentExecutionPreferred,
    CompatibleWarmWorker,
    EligibleFreshSpawn,
    ConcurrencyLimit,
    WriterConflict,
    DepthLimit,
    WorkerBudgetExhausted,
    StrongWorkerLimit,
    RetryLimit,
    CapabilityBudgetExhausted,
    UserApprovalRequired,
    OwnedPathProvenanceRequired,
    InvalidOwnedPath,
    ChildWorktreeUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnedPathProvenance {
    pub trusted_paths: Vec<String>,
    pub source_entry_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSpec {
    pub request: DelegationRequest,
    pub requires_child_worktree: bool,
    pub capability_units: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyOutcome {
    pub decision: RouteDecision,
    pub reason: RouteReason,
    pub capability_units: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestorationKind {
    Hot,
    Native,
    CheckpointRestored,
    Fresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSnapshot {
    pub session_id: String,
    pub workspace_id: String,
    pub worktree_id: String,
    pub role: WorkerRole,
    pub harness: String,
    pub capability_tier: CapabilityTier,
    pub task_family: String,
    pub owned_paths: Vec<String>,
    pub write_mode: WriteMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestBudget {
    pub workers_used: usize,
    pub strong_workers_used: usize,
    pub capability_units_used: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyInput {
    pub workspace_id: String,
    pub worktree_id: String,
    pub parent_session_id: String,
    pub turn_id: String,
    pub parent_depth: i64,
    pub request: DelegationRequest,
    pub owned_path_provenance: OwnedPathProvenance,
    pub requested_harness: String,
    pub task_family: String,
    pub active_workers: Vec<WorkerSnapshot>,
    pub warm_workers: Vec<WorkerSnapshot>,
    pub budget: RequestBudget,
    pub retry_count: usize,
    pub parent_can_execute: bool,
    pub requires_user_approval: bool,
    pub child_worktrees_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyConfig {
    pub max_concurrent_workers: usize,
    pub max_writers_per_worktree: usize,
    pub max_workers_per_turn: usize,
    pub max_strong_workers_per_turn: usize,
    pub max_automatic_retries: usize,
    pub max_depth: i64,
    pub max_capability_units_per_turn: i64,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            max_concurrent_workers: 2,
            max_writers_per_worktree: 1,
            max_workers_per_turn: 3,
            max_strong_workers_per_turn: 1,
            max_automatic_retries: 1,
            max_depth: 1,
            max_capability_units_per_turn: 24,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn decide(&self, input: &PolicyInput) -> PolicyOutcome {
        if input.parent_depth >= self.config.max_depth {
            return outcome(RouteDecision::Reject, RouteReason::DepthLimit, 0);
        }
        if input.requires_user_approval {
            return outcome(
                RouteDecision::RequireUserApproval,
                RouteReason::UserApprovalRequired,
                0,
            );
        }
        if input.parent_can_execute {
            return outcome(
                RouteDecision::ExecuteInParent,
                RouteReason::ParentExecutionPreferred,
                0,
            );
        }
        if normalize_owned_paths(&input.request.owned_paths).is_err() {
            return outcome(RouteDecision::Reject, RouteReason::InvalidOwnedPath, 0);
        }
        if input.request.write_mode != WriteMode::ReadOnly
            && !owned_paths_are_provenanced(
                &input.request.owned_paths,
                &input.owned_path_provenance.trusted_paths,
            )
        {
            return outcome(
                RouteDecision::RequireUserApproval,
                RouteReason::OwnedPathProvenanceRequired,
                0,
            );
        }
        if input.retry_count > self.config.max_automatic_retries {
            return outcome(RouteDecision::Reject, RouteReason::RetryLimit, 0);
        }
        if input.budget.workers_used >= self.config.max_workers_per_turn {
            return outcome(RouteDecision::Reject, RouteReason::WorkerBudgetExhausted, 0);
        }
        if input.request.capability_tier == CapabilityTier::Strong
            && input.budget.strong_workers_used >= self.config.max_strong_workers_per_turn
        {
            return outcome(RouteDecision::Reject, RouteReason::StrongWorkerLimit, 0);
        }
        let warm = input
            .warm_workers
            .iter()
            .find(|worker| compatible_worker(worker, input));
        let restoration = if warm.is_some() {
            RestorationKind::Native
        } else {
            RestorationKind::Fresh
        };
        let units = capability_units(
            input.request.capability_tier,
            input.request.effort,
            restoration,
            input.retry_count > 0,
        );
        if input.budget.capability_units_used + units > self.config.max_capability_units_per_turn {
            return outcome(
                RouteDecision::Reject,
                RouteReason::CapabilityBudgetExhausted,
                units,
            );
        }

        if input.active_workers.len() >= self.config.max_concurrent_workers {
            return outcome(RouteDecision::Queue, RouteReason::ConcurrencyLimit, units);
        }
        if writer_conflict(input, self.config.max_writers_per_worktree) {
            return outcome(RouteDecision::Queue, RouteReason::WriterConflict, units);
        }
        let requires_child_worktree = input.request.write_mode == WriteMode::Isolated
            && input
                .active_workers
                .iter()
                .any(|worker| worker.write_mode != WriteMode::ReadOnly);
        if requires_child_worktree && !input.child_worktrees_available {
            return outcome(
                RouteDecision::Queue,
                RouteReason::ChildWorktreeUnavailable,
                units,
            );
        }
        if let Some(worker) = warm {
            return outcome(
                RouteDecision::ResumeWorker {
                    session_id: worker.session_id.clone(),
                },
                RouteReason::CompatibleWarmWorker,
                units,
            );
        }

        outcome(
            RouteDecision::SpawnWorker(WorkerSpec {
                request: input.request.clone(),
                requires_child_worktree,
                capability_units: units,
            }),
            RouteReason::EligibleFreshSpawn,
            units,
        )
    }
}

pub fn owned_paths_are_provenanced(claimed: &[String], trusted: &[String]) -> bool {
    let Ok(claimed) = normalize_owned_paths(claimed) else {
        return false;
    };
    let Ok(trusted) = normalize_owned_paths(trusted) else {
        return false;
    };
    !claimed.is_empty()
        && claimed.iter().all(|claim| {
            trusted
                .iter()
                .any(|scope| trusted_pattern_covers(scope, claim))
        })
}

fn trusted_pattern_covers(scope: &str, claim: &str) -> bool {
    if scope == claim {
        return true;
    }
    if !scope.ends_with("/**") {
        return false;
    }
    let scope_base = scope.trim_end_matches("/**");
    if scope_base.is_empty() || scope_base.contains(['*', '?', '[']) {
        return false;
    }
    let claim_wildcard = claim.find(['*', '?', '[']);
    let claim_literal = claim_wildcard.map_or(claim, |index| &claim[..index]);
    let descendant_prefix = format!("{scope_base}/");
    claim_literal.starts_with(&descendant_prefix)
}

fn outcome(decision: RouteDecision, reason: RouteReason, capability_units: i64) -> PolicyOutcome {
    PolicyOutcome {
        decision,
        reason,
        capability_units,
    }
}

fn compatible_worker(worker: &WorkerSnapshot, input: &PolicyInput) -> bool {
    worker.workspace_id == input.workspace_id
        && worker.role == input.request.role
        && worker.harness == input.requested_harness
        && worker.capability_tier == input.request.capability_tier
        && worker.task_family == input.task_family
        && normalize_owned_paths(&worker.owned_paths).ok()
            == normalize_owned_paths(&input.request.owned_paths).ok()
}

fn writer_conflict(input: &PolicyInput, max_writers_per_worktree: usize) -> bool {
    if input.request.write_mode == WriteMode::ReadOnly {
        return false;
    }
    let active_writers = input
        .active_workers
        .iter()
        .filter(|worker| worker.write_mode != WriteMode::ReadOnly)
        .collect::<Vec<_>>();
    if matches!(
        input.request.write_mode,
        WriteMode::Shared | WriteMode::Full
    ) && active_writers
        .iter()
        .filter(|worker| worker.worktree_id == input.worktree_id)
        .count()
        >= max_writers_per_worktree
    {
        return true;
    }
    active_writers.iter().any(|worker| {
        owned_path_sets_overlap(&input.request.owned_paths, &worker.owned_paths).unwrap_or(true)
    })
}

pub fn capability_units(
    tier: CapabilityTier,
    effort: Effort,
    restoration: RestorationKind,
    is_retry: bool,
) -> i64 {
    let base: f64 = match (tier, effort) {
        (CapabilityTier::Fast, Effort::Low) => 1.0,
        (CapabilityTier::Fast, Effort::Medium) => 2.0,
        (CapabilityTier::Fast, Effort::High) => 3.0,
        (CapabilityTier::Fast, Effort::Xhigh) => 3.0 * 2.0,
        (CapabilityTier::Standard, Effort::Low) => 2.0,
        (CapabilityTier::Standard, Effort::Medium) => 3.0,
        (CapabilityTier::Standard, Effort::High) => 5.0,
        (CapabilityTier::Standard, Effort::Xhigh) => 5.0 * 2.0,
        (CapabilityTier::Strong, Effort::Low) => 5.0,
        (CapabilityTier::Strong, Effort::Medium) => 6.0,
        (CapabilityTier::Strong, Effort::High) => 8.0,
        (CapabilityTier::Strong, Effort::Xhigh) => 8.0 * 2.0,
    };
    let restoration_multiplier: f64 = match restoration {
        RestorationKind::Hot => 1.0,
        RestorationKind::Native => 0.75,
        RestorationKind::CheckpointRestored => 1.0,
        RestorationKind::Fresh => 1.25,
    };
    let retry_multiplier: f64 = if is_retry { 1.5 } else { 1.0 };
    (base * restoration_multiplier * retry_multiplier).ceil() as i64
}

pub fn normalize_owned_pattern(pattern: &str) -> Result<String, String> {
    let replaced = pattern.trim().replace('\\', "/");
    if replaced.is_empty() || replaced.starts_with('/') || replaced.starts_with('~') {
        return Err("owned path must be non-empty and relative".into());
    }
    let mut parts = Vec::new();
    for part in replaced.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err("owned path must not traverse parents".into()),
            value => parts.push(value),
        }
    }
    if parts.is_empty() {
        return Err("owned path must identify a workspace path".into());
    }
    Ok(parts.join("/"))
}

pub fn normalize_owned_paths(paths: &[String]) -> Result<Vec<String>, String> {
    let mut normalized = paths
        .iter()
        .map(|path| normalize_owned_pattern(path))
        .collect::<Result<Vec<_>, _>>()?;
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

pub fn owned_path_sets_overlap(left: &[String], right: &[String]) -> Result<bool, String> {
    if left.is_empty() || right.is_empty() {
        return Ok(true);
    }
    let left = normalize_owned_paths(left)?;
    let right = normalize_owned_paths(right)?;
    Ok(left.iter().any(|left| {
        right
            .iter()
            .any(|right| normalized_patterns_overlap(left, right))
    }))
}

fn normalized_patterns_overlap(left: &str, right: &str) -> bool {
    let left_wildcard = left.find(['*', '?', '[']);
    let right_wildcard = right.find(['*', '?', '[']);
    let left_prefix = left_wildcard
        .map(|index| left[..index].trim_end_matches('/'))
        .unwrap_or(left);
    let right_prefix = right_wildcard
        .map(|index| right[..index].trim_end_matches('/'))
        .unwrap_or(right);
    if left_prefix.is_empty() || right_prefix.is_empty() {
        return true;
    }
    path_prefix(left_prefix, right_prefix) || path_prefix(right_prefix, left_prefix)
}

fn path_prefix(prefix: &str, candidate: &str) -> bool {
    prefix == candidate
        || candidate
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageReport {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub uncached_input_tokens: Option<i64>,
    pub context_percent: Option<i64>,
    pub runtime_ms: Option<i64>,
    pub cost_microusd: Option<i64>,
    pub cost_source: Option<String>,
}

impl UsageReport {
    pub fn from_normalized(data: &Value) -> Option<Self> {
        let usage = data.get("usage").unwrap_or(data);
        let report = Self {
            input_tokens: integer_alias(usage, &["input_tokens", "inputTokens"]),
            output_tokens: integer_alias(usage, &["output_tokens", "outputTokens"]),
            cache_read_tokens: integer_alias(
                usage,
                &[
                    "cache_read_tokens",
                    "cacheReadTokens",
                    "cached_input_tokens",
                    "cache_read_input_tokens",
                ],
            ),
            cache_write_tokens: integer_alias(
                usage,
                &["cache_write_tokens", "cacheWriteTokens", "cache_creation_input_tokens"],
            ),
            uncached_input_tokens: integer_alias(
                usage,
                &["uncached_input_tokens", "uncachedInputTokens"],
            ),
            context_percent: integer_alias(data, &["context_percent", "contextPercent"])
                .or_else(|| integer_alias(usage, &["context_percent", "contextPercent"])),
            runtime_ms: integer_alias(data, &["runtime_ms", "runtimeMs", "duration_ms"]),
            cost_microusd: decimal_alias(data, &["cost_usd", "costUsd", "total_cost_usd", "totalCostUsd"])
                .or_else(|| decimal_alias(usage, &["cost_usd", "costUsd", "total_cost_usd", "totalCostUsd"]))
                .map(|value| (value * 1_000_000.0).round() as i64),
            cost_source: decimal_alias(data, &["cost_usd", "costUsd", "total_cost_usd", "totalCostUsd"])
                .or_else(|| decimal_alias(usage, &["cost_usd", "costUsd", "total_cost_usd", "totalCostUsd"]))
                .map(|_| "provider_reported".into()),
        };
        (report != Self::default()).then_some(report)
    }

    pub fn ledger_row(
        &self,
        workspace_id: &str,
        session_id: &str,
        turn_id: Option<&str>,
        source: &str,
    ) -> UsageLedgerRow {
        UsageLedgerRow {
            id: 0,
            workspace_id: workspace_id.into(),
            session_id: Some(session_id.into()),
            turn_id: turn_id.map(str::to_owned),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens,
            uncached_input_tokens: self.uncached_input_tokens,
            context_percent: self.context_percent,
            capability_units: 0,
            runtime_ms: self.runtime_ms,
            cost_microusd: self.cost_microusd,
            cost_source: self.cost_source.clone(),
            stable_prefix_id: None,
            stable_prefix_hash: None,
            prompt_schema_version: None,
            prefix_token_estimate: None,
            harness: None,
            model: None,
            role: None,
            task_family: None,
            restoration_mode: None,
            cross_harness_reuse: None,
            source: source.into(),
            created_at: Utc::now().to_rfc3339(),
        }
    }
}

fn integer_alias(value: &Value, keys: &[&str]) -> Option<i64> {
    if let Some(found) = keys.iter().find_map(|key| value.get(*key)?.as_i64()) {
        return Some(found);
    }
    value.as_object().and_then(|object| {
        object.values().find_map(|child| {
            child
                .is_object()
                .then(|| integer_alias(child, keys))
                .flatten()
        })
    })
}

fn decimal_alias(value: &Value, keys: &[&str]) -> Option<f64> {
    if let Some(found) = keys.iter().find_map(|key| value.get(*key)?.as_f64()) {
        return Some(found);
    }
    value.as_object().and_then(|object| {
        object.values().find_map(|child| {
            child
                .is_object()
                .then(|| decimal_alias(child, keys))
                .flatten()
        })
    })
}

pub fn record_provider_usage(
    db: &Connection,
    workspace_id: &str,
    session_id: &str,
    turn_id: Option<&str>,
    source: &str,
    data: &Value,
) -> Result<bool, BridgeError> {
    let Some(report) = UsageReport::from_normalized(data) else {
        return Ok(false);
    };
    let mut row = report.ledger_row(workspace_id, session_id, turn_id, source);
    row.uncached_input_tokens = report.uncached_input_tokens.or_else(|| {
        report.input_tokens.map(|input| {
            if source == "provider.claude" {
                input.max(0)
            } else {
                input.saturating_sub(report.cache_read_tokens.unwrap_or(0))
                    .saturating_sub(report.cache_write_tokens.unwrap_or(0))
                    .max(0)
            }
        })
    });
    let prompt = match turn_id {
        Some(turn_id) => store::prompt_compilation_for_turn(db, session_id, turn_id)?,
        None => store::latest_prompt_compilation(db, session_id)?,
    };
    if let Some(prompt) = prompt {
        row.stable_prefix_id = Some(prompt.prefix_id);
        row.stable_prefix_hash = Some(prompt.prefix_hash);
        row.prompt_schema_version = Some(prompt.schema_version);
        row.prefix_token_estimate = Some(prompt.prefix_token_estimate);
        row.harness = Some(prompt.harness);
        row.model = prompt.model;
        row.role = Some(prompt.role);
        row.task_family = Some(prompt.task_family);
        row.restoration_mode = Some(prompt.restoration_mode);
        row.cross_harness_reuse = Some(prompt.cross_harness_reuse);
    }
    store::append_usage_ledger(db, &row)?;
    Ok(true)
}

pub fn load_request_budget(
    db: &Connection,
    workspace_id: &str,
    turn_id: &str,
) -> Result<RequestBudget, BridgeError> {
    Ok(db.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN source='policy.spawn.strong' THEN 1 ELSE 0 END),0),
                COALESCE(SUM(capability_units),0)
         FROM usage_ledger
         WHERE workspace_id=?1 AND turn_id=?2 AND source LIKE 'policy.spawn.%'",
        params![workspace_id, turn_id],
        |row| {
            Ok(RequestBudget {
                workers_used: row.get::<_, i64>(0)? as usize,
                strong_workers_used: row.get::<_, i64>(1)? as usize,
                capability_units_used: row.get(2)?,
            })
        },
    )?)
}

pub fn load_workers(
    db: &Connection,
    workspace_id: &str,
    lease_status: &str,
) -> Result<Vec<WorkerSnapshot>, BridgeError> {
    let leases = store::worker_leases(db, workspace_id)?;
    let mut workers = Vec::new();
    for lease in leases {
        let eligible = if lease_status == "warm" {
            matches!(lease.lease_status.as_str(), "warm" | "checkpointed" | "expired")
                && db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM worker_runtime WHERE session_id=?1 AND lifecycle_state IN ('warm','stopped') AND result_status='reported')",
                    params![lease.session_id],
                    |row| row.get::<_, bool>(0),
                )?
        } else {
            lease.lease_status == lease_status
        };
        if eligible {
            workers.push(worker_from_lease(db, lease)?);
        }
    }
    Ok(workers)
}

fn worker_from_lease(db: &Connection, lease: WorkerLease) -> Result<WorkerSnapshot, BridgeError> {
    let harness: String = db.query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![lease.session_id],
        |row| row.get(0),
    )?;
    Ok(WorkerSnapshot {
        session_id: lease.session_id,
        workspace_id: lease.workspace_id.clone(),
        worktree_id: lease.workspace_id,
        role: parse_role(&lease.role),
        harness,
        capability_tier: parse_tier(&lease.capability_tier),
        task_family: lease.task_family,
        owned_paths: serde_json::from_value(lease.owned_paths).unwrap_or_default(),
        write_mode: parse_write_mode(&lease.write_mode),
    })
}

pub fn record_decision(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    input: &PolicyInput,
    outcome: &PolicyOutcome,
) -> Result<(), BridgeError> {
    let request = &input.request;
    let kind = match &outcome.decision {
        RouteDecision::SpawnWorker(_) | RouteDecision::ResumeWorker { .. } => {
            EntryKind::DelegationApproved
        }
        RouteDecision::RequireUserApproval => EntryKind::ApprovalRequested,
        RouteDecision::Queue => EntryKind::DelegationRequested,
        RouteDecision::Reject | RouteDecision::ExecuteInParent => EntryKind::DelegationRejected,
    };
    let mut payload = serde_json::json!({
        "requestId": turn_id,
        "turnId": turn_id,
        "decision": outcome.decision,
        "reason": outcome.reason,
        "request": request,
        "budget": {
            "workersUsed": input.budget.workers_used,
            "strongWorkersUsed": input.budget.strong_workers_used,
            "capabilityUnitsUsed": input.budget.capability_units_used,
        },
        "capabilityUnits": outcome.capability_units,
        "ownedPathProvenance": input.owned_path_provenance,
        "replaySchemaVersion": 1,
        "replayInput": input,
    });
    if matches!(outcome.decision, RouteDecision::RequireUserApproval) {
        let scope_key = normalize_owned_paths(&request.owned_paths)
            .unwrap_or_else(|_| request.owned_paths.clone())
            .join("|");
        let approval_id = format!("delegation-path-scope:{turn_id}:{scope_key}");
        let branch = SessionForest::new(db)
            .active_branch(parent_session_id)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?;
        let approval_already_recorded = branch.iter().any(|entry| {
            entry.kind == "approval.requested" && entry.payload["approvalId"] == approval_id
        });
        if approval_already_recorded {
            return Ok(());
        }
        payload["approvalId"] = Value::String(approval_id);
        payload["approvalType"] = Value::String("delegation_path_scope".into());
        payload["status"] = Value::String("pending".into());
        payload["title"] = Value::String("Approve delegation write scope".into());
        payload["text"] = Value::String(format!(
            "Allow this worker to write only within: {}",
            request.owned_paths.join(", ")
        ));
        payload["requestedOwnedPaths"] = serde_json::json!(request.owned_paths);
    }
    SessionForest::new(db)
        .append(parent_session_id, kind, payload)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "policy",
        "policy.route_decision",
        parent_session_id,
        &serde_json::to_string(&serde_json::json!({
            "turnId": turn_id,
            "decision": outcome.decision,
            "reason": outcome.reason,
        }))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?,
    )
}

pub fn record_spawn_usage(
    db: &Connection,
    workspace_id: &str,
    session_id: &str,
    turn_id: &str,
    outcome: &PolicyOutcome,
    tier: CapabilityTier,
) -> Result<(), BridgeError> {
    let source = match tier {
        CapabilityTier::Fast => "policy.spawn.fast",
        CapabilityTier::Standard => "policy.spawn.standard",
        CapabilityTier::Strong => "policy.spawn.strong",
    };
    store::append_usage_ledger(
        db,
        &UsageLedgerRow {
            id: 0,
            workspace_id: workspace_id.into(),
            session_id: Some(session_id.into()),
            turn_id: Some(turn_id.into()),
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            uncached_input_tokens: None,
            context_percent: None,
            capability_units: outcome.capability_units,
            runtime_ms: None,
            cost_microusd: None,
            cost_source: None,
            stable_prefix_id: None,
            stable_prefix_hash: None,
            prompt_schema_version: None,
            prefix_token_estimate: None,
            harness: None,
            model: None,
            role: None,
            task_family: None,
            restoration_mode: None,
            cross_harness_reuse: None,
            source: source.into(),
            created_at: Utc::now().to_rfc3339(),
        },
    )?;
    Ok(())
}

pub fn role_name(role: WorkerRole) -> &'static str {
    match role {
        WorkerRole::Research => "research",
        WorkerRole::Implementation => "implementation",
        WorkerRole::Verification => "verification",
        WorkerRole::Planning => "planning",
        WorkerRole::Documentation => "documentation",
    }
}

pub fn tier_name(tier: CapabilityTier) -> &'static str {
    match tier {
        CapabilityTier::Fast => "fast",
        CapabilityTier::Standard => "standard",
        CapabilityTier::Strong => "strong",
    }
}

pub fn write_mode_name(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::ReadOnly => "readOnly",
        WriteMode::Shared => "shared",
        WriteMode::Isolated => "isolated",
        WriteMode::Full => "full",
    }
}

fn parse_role(value: &str) -> WorkerRole {
    match value {
        "research" => WorkerRole::Research,
        "verification" => WorkerRole::Verification,
        "planning" => WorkerRole::Planning,
        "documentation" => WorkerRole::Documentation,
        _ => WorkerRole::Implementation,
    }
}

fn parse_tier(value: &str) -> CapabilityTier {
    match value {
        "fast" => CapabilityTier::Fast,
        "strong" => CapabilityTier::Strong,
        _ => CapabilityTier::Standard,
    }
}

fn parse_write_mode(value: &str) -> WriteMode {
    match value {
        "readOnly" => WriteMode::ReadOnly,
        "isolated" => WriteMode::Isolated,
        "full" => WriteMode::Full,
        _ => WriteMode::Shared,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::{OutputContract, WorkerRole};
    use serde_json::json;

    fn request(
        role: WorkerRole,
        tier: CapabilityTier,
        effort: Effort,
        write_mode: WriteMode,
        owned_paths: &[&str],
    ) -> DelegationRequest {
        DelegationRequest {
            schema_version: 1,
            role,
            objective: "Complete focused work".into(),
            acceptance_criteria: vec!["Verification passes".into()],
            known_facts: Vec::new(),
            decisions: Vec::new(),
            evidence_ids: Vec::new(),
            relevant_files: Vec::new(),
            owned_paths: owned_paths.iter().map(|path| (*path).into()).collect(),
            write_mode,
            capability_tier: tier,
            effort,
            verification: vec!["cargo test".into()],
            output_contract: OutputContract::ImplementationResult,
            harness: Some("codex".into()),
            model: None,
        }
    }

    fn worker(
        id: &str,
        role: WorkerRole,
        tier: CapabilityTier,
        write_mode: WriteMode,
        paths: &[&str],
    ) -> WorkerSnapshot {
        WorkerSnapshot {
            session_id: id.into(),
            workspace_id: "w".into(),
            worktree_id: "w".into(),
            role,
            harness: "codex".into(),
            capability_tier: tier,
            task_family: "implementation".into(),
            owned_paths: paths.iter().map(|path| (*path).into()).collect(),
            write_mode,
        }
    }

    fn input() -> PolicyInput {
        PolicyInput {
            workspace_id: "w".into(),
            worktree_id: "w".into(),
            parent_session_id: "parent".into(),
            turn_id: "turn-1".into(),
            parent_depth: 0,
            request: request(
                WorkerRole::Implementation,
                CapabilityTier::Standard,
                Effort::Medium,
                WriteMode::Isolated,
                &["src/auth/**"],
            ),
            owned_path_provenance: OwnedPathProvenance {
                trusted_paths: vec!["src/auth/**".into()],
                source_entry_ids: vec!["user-entry".into()],
            },
            requested_harness: "codex".into(),
            task_family: "implementation".into(),
            active_workers: Vec::new(),
            warm_workers: Vec::new(),
            budget: RequestBudget::default(),
            retry_count: 0,
            parent_can_execute: false,
            requires_user_approval: false,
            child_worktrees_available: true,
        }
    }

    #[test]
    fn write_claim_without_provenance_requires_user_approval() {
        let mut case = input();
        case.owned_path_provenance = OwnedPathProvenance::default();
        let outcome = PolicyEngine::default().decide(&case);
        assert_eq!(outcome.reason, RouteReason::OwnedPathProvenanceRequired);
        assert!(matches!(
            outcome.decision,
            RouteDecision::RequireUserApproval
        ));
    }

    #[test]
    fn write_claim_must_not_be_broader_than_provenance() {
        let mut case = input();
        case.request.owned_paths = vec!["src/**".into()];
        case.owned_path_provenance.trusted_paths = vec!["src/auth/session.rs".into()];
        assert_eq!(
            PolicyEngine::default().decide(&case).reason,
            RouteReason::OwnedPathProvenanceRequired
        );
        assert!(owned_paths_are_provenanced(
            &["src/auth/session.rs".into()],
            &["src/auth/**".into()]
        ));
        assert!(!owned_paths_are_provenanced(
            &["src/auth/nested/session.rs".into()],
            &["src/auth/*".into()]
        ));
        for escaping_claim in ["src/auth*", "src/auth?", "src/auth[0-9]", "src/auth*/**"] {
            assert!(
                !owned_paths_are_provenanced(&[escaping_claim.into()], &["src/auth/**".into()]),
                "component-escaping claim was authorized: {escaping_claim}"
            );
        }
        assert!(owned_paths_are_provenanced(
            &["src/auth/*.rs".into()],
            &["src/auth/**".into()]
        ));
        for escaping_scope in ["src/auth*/**", "src/[ab]/**", "src/auth?/**"] {
            assert!(
                !owned_paths_are_provenanced(&["src/secrets/**".into()], &[escaping_scope.into()]),
                "wildcard trusted scope broadened authority: {escaping_scope}"
            );
        }
    }

    #[test]
    fn read_only_request_does_not_require_write_provenance() {
        let mut case = input();
        case.request.write_mode = WriteMode::ReadOnly;
        case.request.owned_paths.clear();
        case.owned_path_provenance = OwnedPathProvenance::default();
        assert!(matches!(
            PolicyEngine::default().decide(&case).decision,
            RouteDecision::SpawnWorker(_)
        ));
    }

    #[test]
    fn decision_matrix_covers_every_route_and_limit() {
        let engine = PolicyEngine::default();

        let mut case = input();
        case.parent_can_execute = true;
        assert_eq!(
            engine.decide(&case).reason,
            RouteReason::ParentExecutionPreferred
        );

        let mut case = input();
        case.requires_user_approval = true;
        assert!(matches!(
            engine.decide(&case).decision,
            RouteDecision::RequireUserApproval
        ));

        let spawn = engine.decide(&input());
        assert_eq!(spawn.reason, RouteReason::EligibleFreshSpawn);
        assert!(matches!(spawn.decision, RouteDecision::SpawnWorker(_)));

        let mut case = input();
        case.parent_depth = 1;
        assert_eq!(engine.decide(&case).reason, RouteReason::DepthLimit);

        let mut case = input();
        case.retry_count = 2;
        assert_eq!(engine.decide(&case).reason, RouteReason::RetryLimit);

        let mut case = input();
        case.budget.workers_used = 3;
        assert_eq!(
            engine.decide(&case).reason,
            RouteReason::WorkerBudgetExhausted
        );

        let mut case = input();
        case.request.capability_tier = CapabilityTier::Strong;
        case.budget.strong_workers_used = 1;
        assert_eq!(engine.decide(&case).reason, RouteReason::StrongWorkerLimit);

        let mut case = input();
        case.active_workers = vec![
            worker(
                "one",
                WorkerRole::Research,
                CapabilityTier::Fast,
                WriteMode::ReadOnly,
                &["docs/**"],
            ),
            worker(
                "two",
                WorkerRole::Verification,
                CapabilityTier::Fast,
                WriteMode::ReadOnly,
                &["tests/**"],
            ),
        ];
        assert_eq!(engine.decide(&case).reason, RouteReason::ConcurrencyLimit);

        let mut case = input();
        case.active_workers = vec![worker(
            "writer",
            WorkerRole::Implementation,
            CapabilityTier::Standard,
            WriteMode::Isolated,
            &["src/**"],
        )];
        assert_eq!(engine.decide(&case).reason, RouteReason::WriterConflict);

        let mut case = input();
        case.request.owned_paths = vec!["../outside".into()];
        assert_eq!(engine.decide(&case).reason, RouteReason::InvalidOwnedPath);

        let mut case = input();
        case.warm_workers = vec![worker(
            "warm",
            WorkerRole::Implementation,
            CapabilityTier::Standard,
            WriteMode::Isolated,
            &["src/auth/**"],
        )];
        assert!(matches!(
            engine.decide(&case).decision,
            RouteDecision::ResumeWorker { ref session_id } if session_id == "warm"
        ));

        let mut case = input();
        case.request.capability_tier = CapabilityTier::Strong;
        let tight = PolicyEngine::new(PolicyConfig {
            max_capability_units_per_turn: 5,
            ..PolicyConfig::default()
        });
        assert_eq!(
            tight.decide(&case).reason,
            RouteReason::CapabilityBudgetExhausted
        );
    }

    #[test]
    fn read_only_parallelism_and_disjoint_isolated_writers_are_allowed() {
        let engine = PolicyEngine::default();
        let mut reads = input();
        reads.request.write_mode = WriteMode::ReadOnly;
        reads.request.owned_paths.clear();
        reads.active_workers = vec![worker(
            "reader",
            WorkerRole::Research,
            CapabilityTier::Fast,
            WriteMode::ReadOnly,
            &[],
        )];
        assert!(matches!(
            engine.decide(&reads).decision,
            RouteDecision::SpawnWorker(_)
        ));

        let mut writers = input();
        writers.active_workers = vec![worker(
            "writer",
            WorkerRole::Implementation,
            CapabilityTier::Standard,
            WriteMode::Isolated,
            &["src/ui/**"],
        )];
        let outcome = engine.decide(&writers);
        let RouteDecision::SpawnWorker(spec) = outcome.decision else {
            panic!("disjoint writer did not spawn: {outcome:?}");
        };
        assert!(spec.requires_child_worktree);

        writers.child_worktrees_available = false;
        assert_eq!(
            engine.decide(&writers).reason,
            RouteReason::ChildWorktreeUnavailable
        );
    }

    #[test]
    fn same_inputs_produce_identical_decision_and_reason() {
        let engine = PolicyEngine::default();
        let input = input();
        assert_eq!(engine.decide(&input), engine.decide(&input));
    }

    #[test]
    fn capability_unit_matrix_and_multipliers_are_stable() {
        assert_eq!(
            capability_units(
                CapabilityTier::Fast,
                Effort::Low,
                RestorationKind::Hot,
                false
            ),
            1
        );
        assert_eq!(
            capability_units(
                CapabilityTier::Standard,
                Effort::Medium,
                RestorationKind::Hot,
                false
            ),
            3
        );
        assert_eq!(
            capability_units(
                CapabilityTier::Strong,
                Effort::High,
                RestorationKind::Hot,
                false
            ),
            8
        );
        assert_eq!(
            capability_units(
                CapabilityTier::Strong,
                Effort::Xhigh,
                RestorationKind::Fresh,
                true
            ),
            30
        );
        assert_eq!(
            capability_units(
                CapabilityTier::Standard,
                Effort::Medium,
                RestorationKind::Native,
                false
            ),
            3
        );
        assert_eq!(
            capability_units(
                CapabilityTier::Fast,
                Effort::Low,
                RestorationKind::Fresh,
                false
            ),
            2
        );
    }

    #[test]
    fn path_overlap_handles_glob_file_nested_and_disjoint_sets() {
        let paths = |items: &[&str]| items.iter().map(|item| (*item).into()).collect::<Vec<_>>();
        assert!(
            owned_path_sets_overlap(&paths(&["src/**"]), &paths(&["src/auth/store.rs"])).unwrap()
        );
        assert!(
            owned_path_sets_overlap(&paths(&["src/auth/*.rs"]), &paths(&["src/auth/**"])).unwrap()
        );
        assert!(!owned_path_sets_overlap(&paths(&["src/**"]), &paths(&["tests/**"])).unwrap());
        assert!(
            owned_path_sets_overlap(&paths(&["src\\auth\\**"]), &paths(&["src/auth/a.rs"]))
                .unwrap()
        );
        assert!(owned_path_sets_overlap(&[], &paths(&["src/**"])).unwrap());
        assert!(owned_path_sets_overlap(&paths(&["../secret"]), &paths(&["src/**"])).is_err());
        assert!(owned_path_sets_overlap(&paths(&["~/secret"]), &paths(&["src/**"])).is_err());
    }

    #[test]
    fn warm_worker_compatibility_uses_complete_key() {
        let engine = PolicyEngine::default();
        let compatible = worker(
            "warm",
            WorkerRole::Implementation,
            CapabilityTier::Standard,
            WriteMode::Isolated,
            &["src/auth/**"],
        );
        let fields = ["workspace", "role", "harness", "tier", "family", "paths"];
        for field in fields {
            let mut candidate = compatible.clone();
            match field {
                "workspace" => candidate.workspace_id = "other".into(),
                "role" => candidate.role = WorkerRole::Research,
                "harness" => candidate.harness = "claude".into(),
                "tier" => candidate.capability_tier = CapabilityTier::Strong,
                "family" => candidate.task_family = "other".into(),
                "paths" => candidate.owned_paths = vec!["src/ui/**".into()],
                _ => unreachable!(),
            }
            let mut input = input();
            input.warm_workers = vec![candidate];
            assert!(matches!(
                engine.decide(&input).decision,
                RouteDecision::SpawnWorker(_)
            ));
        }
    }

    #[test]
    fn usage_report_normalizes_codex_and_claude_shapes() {
        let codex = UsageReport::from_normalized(&json!({
            "inputTokens": 10,
            "outputTokens": 4,
            "cacheReadTokens": 3,
            "contextPercent": 25,
            "runtimeMs": 90
        }))
        .unwrap();
        assert_eq!(codex.input_tokens, Some(10));
        assert_eq!(codex.cache_read_tokens, Some(3));
        assert_eq!(codex.runtime_ms, Some(90));

        let nested_codex = UsageReport::from_normalized(&json!({
            "rate_limits": {
                "usage": {
                    "inputTokens": 12,
                    "outputTokens": 6,
                    "cacheReadTokens": 4
                }
            }
        }))
        .unwrap();
        assert_eq!(nested_codex.input_tokens, Some(12));
        assert_eq!(nested_codex.output_tokens, Some(6));
        assert_eq!(nested_codex.cache_read_tokens, Some(4));

        let claude_events = crate::agent::normalize_claude_message(&json!({
            "type": "result",
            "subtype": "success",
            "result": "done",
            "usage": {
                "input_tokens": 11,
                "output_tokens": 5,
                "cache_read_input_tokens": 2,
                "cache_creation_input_tokens": 1
            },
            "total_cost_usd": 0.012345
        }));
        let claude_data = &claude_events.iter().find(|event| event.kind == "usage.updated").unwrap().data;
        let claude = UsageReport::from_normalized(claude_data)
        .unwrap();
        assert_eq!(claude.output_tokens, Some(5));
        assert_eq!(claude.cache_read_tokens, Some(2));
        assert_eq!(claude.cache_write_tokens, Some(1));
        assert_eq!(claude.cost_microusd, Some(12_345));

        let unknown_cost = UsageReport::from_normalized(&json!({
            "usage": {"input_tokens": 1}
        }))
        .unwrap();
        assert_eq!(unknown_cost.cost_microusd, None);
    }

    #[test]
    fn provider_usage_is_recorded_with_parent_turn_id() {
        let db = database();
        let first_compilation = crate::model::PromptCompilationRecord {
            id: 0,
            session_id: "parent".into(),
            turn_id: None,
            prefix_id: "bridge-prompt-v1-deadbeef".into(),
            prefix_hash: "deadbeef".into(),
            schema_version: 1,
            prefix_bytes: 400,
            prefix_token_estimate: 100,
            harness: "claude".into(),
            model: Some("sonnet".into()),
            role: "orchestrator".into(),
            task_family: "orchestration".into(),
            restoration_mode: "fresh".into(),
            cross_harness_reuse: "not_applicable".into(),
            created_at: "now".into(),
        };
        store::record_prompt_compilation(&db, &first_compilation).unwrap();
        assert!(store::bind_latest_prompt_compilation_to_turn(&db, "parent", "turn-usage").unwrap());
        assert!(record_provider_usage(
            &db,
            "w",
            "parent",
            Some("turn-usage"),
            "provider.claude",
            &json!({
                "usage": {"input_tokens": 7, "output_tokens": 3, "cache_read_tokens": 2, "cache_write_tokens": 1},
                "duration_ms": 42
            }),
        )
        .unwrap());
        let rows = store::usage_ledger(&db, "w", Some("parent")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turn_id.as_deref(), Some("turn-usage"));
        assert_eq!(rows[0].input_tokens, Some(7));
        assert_eq!(rows[0].uncached_input_tokens, Some(7));
        assert_eq!(rows[0].runtime_ms, Some(42));
        assert_eq!(rows[0].source, "provider.claude");
        assert_eq!(rows[0].stable_prefix_id.as_deref(), Some("bridge-prompt-v1-deadbeef"));
        assert_eq!(rows[0].prefix_token_estimate, Some(100));
        assert_eq!(rows[0].harness.as_deref(), Some("claude"));
        assert_eq!(rows[0].restoration_mode.as_deref(), Some("fresh"));

        let mut next_compilation = first_compilation;
        next_compilation.prefix_id = "bridge-prompt-v1-next".into();
        next_compilation.prefix_hash = "next".into();
        next_compilation.model = Some("opus".into());
        next_compilation.restoration_mode = "hot".into();
        store::record_prompt_compilation(&db, &next_compilation).unwrap();
        assert!(record_provider_usage(
            &db,
            "w",
            "parent",
            Some("turn-usage"),
            "provider.claude",
            &json!({"usage":{"input_tokens":2,"cache_read_input_tokens":1}}),
        ).unwrap());
        let rows = store::usage_ledger(&db, "w", Some("parent")).unwrap();
        assert_eq!(rows[1].stable_prefix_id.as_deref(), Some("bridge-prompt-v1-deadbeef"));
        assert_eq!(rows[1].model.as_deref(), Some("sonnet"));
        assert_eq!(rows[1].restoration_mode.as_deref(), Some("fresh"));

        assert!(record_provider_usage(
            &db,
            "w",
            "parent",
            Some("turn-codex"),
            "provider.codex",
            &json!({"usage":{"input_tokens":10,"cache_read_tokens":4,"cache_write_tokens":1}}),
        ).unwrap());
        let rows = store::usage_ledger(&db, "w", Some("parent")).unwrap();
        assert_eq!(rows[2].uncached_input_tokens, Some(5));
    }

    fn database() -> Connection {
        let db = store::open(std::path::Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/policy-demo','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Kyoto','Task','bridge/task','/tmp/policy-workspace','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','idle','reported')", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('parent','fresh','now')", []).unwrap();
        db
    }

    #[test]
    fn new_turn_resets_request_budget() {
        let db = database();
        let outcome = PolicyEngine::default().decide(&input());
        for _ in 0..3 {
            record_spawn_usage(
                &db,
                "w",
                "parent",
                "turn-1",
                &outcome,
                CapabilityTier::Standard,
            )
            .unwrap();
        }
        assert_eq!(
            load_request_budget(&db, "w", "turn-1")
                .unwrap()
                .workers_used,
            3
        );
        assert_eq!(
            load_request_budget(&db, "w", "turn-2").unwrap(),
            RequestBudget::default()
        );
    }

    #[test]
    fn policy_decision_is_persisted_in_active_forest() {
        let db = database();
        let input = input();
        let outcome = PolicyEngine::default().decide(&input);
        record_decision(
            &db,
            "parent",
            "turn-1",
            &input,
            &outcome,
        )
        .unwrap();
        let branch = SessionForest::new(&db).active_branch("parent").unwrap();
        assert_eq!(branch.len(), 1);
        assert_eq!(branch[0].kind, "delegation.approved");
        assert_eq!(branch[0].payload["turnId"], "turn-1");
        assert_eq!(branch[0].payload["reason"], "eligible_fresh_spawn");
        assert_eq!(branch[0].payload["replaySchemaVersion"], 1);
        assert_eq!(branch[0].payload["replayInput"]["turnId"], "turn-1");
        assert_eq!(
            branch[0].payload["ownedPathProvenance"]["trustedPaths"][0],
            "src/auth/**"
        );
        assert_eq!(
            db.query_row(
                "SELECT kind FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "policy.route_decision"
        );
    }
}
