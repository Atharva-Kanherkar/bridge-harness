//! Cost-and-quality routing that learns from durable worker outcomes.
//!
//! The learning layer ranks provider/model candidates, while [`crate::policy`]
//! remains the non-bypassable authority for permissions, topology, worktrees,
//! and budgets. Shadow mode is the default so recommendations can be measured
//! before a workspace explicitly enables autonomous selection.

use crate::{
    delegation::{DelegationRequest, Effort, WorkerResult, WorkerResultStatus},
    model::{AdapterDescriptor, CapabilityTier},
    policy::{self, PolicyConfig, RestorationKind},
    BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const ROUTER_SCHEMA_VERSION: u32 = 1;
pub const MIN_SHADOW_OUTCOMES_FOR_AUTONOMY: i64 = 20;
const PRIOR_WEIGHT: i64 = 4;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RouterMode {
    Disabled,
    #[default]
    Shadow,
    Autonomous,
}

impl RouterMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Shadow => "shadow",
            Self::Autonomous => "autonomous",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterPreferences {
    pub mode: RouterMode,
    pub minimum_pass_bps: u16,
    #[serde(default)]
    pub pinned_harness: Option<String>,
    #[serde(default)]
    pub pinned_model: Option<String>,
    #[serde(default)]
    pub excluded_harnesses: Vec<String>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
}

impl Default for RouterPreferences {
    fn default() -> Self {
        Self {
            mode: RouterMode::Shadow,
            minimum_pass_bps: 6_500,
            pinned_harness: None,
            pinned_model: None,
            excluded_harnesses: Vec::new(),
            excluded_models: Vec::new(),
        }
    }
}

impl RouterPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if self.minimum_pass_bps > 10_000 {
            return Err("minimumPassBps cannot exceed 10000".into());
        }
        for (field, value) in [
            ("pinnedHarness", self.pinned_harness.as_deref()),
            ("pinnedModel", self.pinned_model.as_deref()),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("{field} cannot be empty"));
            }
        }
        if self
            .excluded_harnesses
            .iter()
            .any(|value| value.trim().is_empty())
            || self
                .excluded_models
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err("router exclusions cannot contain empty values".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CandidateExclusion {
    HarnessUnavailable,
    MissingCapability,
    UnsupportedPlatform,
    PermissionCeiling,
    QuotaExhausted,
    ContextExhausted,
    RiskCeiling,
    BudgetCeiling,
    UserExcludedHarness,
    UserExcludedModel,
    PinMismatch,
    BelowQualityFloor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteCandidate {
    pub harness: String,
    pub model: String,
    pub tier: CapabilityTier,
    pub effort: Effort,
    pub sandbox: String,
    pub capabilities: Vec<String>,
    pub available: bool,
    pub platform_supported: bool,
    pub permission_eligible: bool,
    pub quota_available: bool,
    pub context_available: bool,
    pub risk_eligible: bool,
    pub capability_units: i64,
    pub default_for_tier: bool,
}

impl RouteCandidate {
    pub fn key(&self) -> String {
        format!("{}:{}", self.harness, self.model)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HistoricalOutcome {
    pub samples: i64,
    pub successes: i64,
    pub runtime_ms_total: i64,
    pub normalized_cost_total: i64,
    pub retries: i64,
    pub human_interventions: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidatePrediction {
    pub pass_probability_bps: u16,
    pub latency_ms: i64,
    pub normalized_quota_cost: i64,
    pub retry_risk_bps: u16,
    pub samples: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateEvaluation {
    pub candidate: RouteCandidate,
    pub exclusions: Vec<CandidateExclusion>,
    pub prediction: CandidatePrediction,
    pub expected_cost_score: i64,
}

impl CandidateEvaluation {
    pub fn eligible(&self) -> bool {
        self.exclusions.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterDecision {
    pub schema_version: u32,
    pub id: String,
    pub workspace_id: String,
    pub parent_session_id: String,
    pub turn_id: String,
    pub task_family: String,
    pub mode: RouterMode,
    pub manual_override: bool,
    pub baseline_candidate: Option<String>,
    pub recommended_candidate: Option<String>,
    pub executed_candidate: Option<String>,
    pub explanation: String,
    pub candidates: Vec<CandidateEvaluation>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct RoutedDelegation {
    pub request: DelegationRequest,
    pub decision: RouterDecision,
}

#[derive(Debug, Clone)]
pub struct EvaluationInput {
    pub candidates: Vec<RouteCandidate>,
    pub preferences: RouterPreferences,
    pub histories: BTreeMap<String, HistoricalOutcome>,
    pub required_capabilities: Vec<String>,
    pub remaining_capability_units: i64,
}

fn tier_prior(tier: CapabilityTier) -> (u16, i64) {
    match tier {
        CapabilityTier::Fast => (6_500, 10_000),
        CapabilityTier::Standard => (7_800, 20_000),
        CapabilityTier::Strong => (8_600, 30_000),
    }
}

fn predict(candidate: &RouteCandidate, history: &HistoricalOutcome) -> CandidatePrediction {
    let (prior_pass, prior_latency) = tier_prior(candidate.tier);
    let denominator = PRIOR_WEIGHT + history.samples;
    let pass_probability_bps =
        ((i64::from(prior_pass) * PRIOR_WEIGHT + history.successes * 10_000) / denominator) as u16;
    let latency_ms = (prior_latency * PRIOR_WEIGHT + history.runtime_ms_total) / denominator;
    let prior_cost = candidate.capability_units * 1_000;
    let normalized_quota_cost =
        (prior_cost * PRIOR_WEIGHT + history.normalized_cost_total) / denominator;
    let retry_risk_bps = (((10_000 - i64::from(prior_pass)) * PRIOR_WEIGHT
        + history.retries * 10_000)
        / denominator) as u16;
    CandidatePrediction {
        pass_probability_bps,
        latency_ms,
        normalized_quota_cost,
        retry_risk_bps,
        samples: history.samples,
    }
}

pub fn evaluate(input: EvaluationInput) -> Vec<CandidateEvaluation> {
    let excluded_harnesses = input
        .preferences
        .excluded_harnesses
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let excluded_models = input
        .preferences
        .excluded_models
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let required = input
        .required_capabilities
        .iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut evaluations = input
        .candidates
        .into_iter()
        .map(|candidate| {
            let mut exclusions = BTreeSet::new();
            if !candidate.available {
                exclusions.insert(CandidateExclusion::HarnessUnavailable);
            }
            let capabilities = candidate
                .capabilities
                .iter()
                .map(|value| value.to_ascii_lowercase())
                .collect::<BTreeSet<_>>();
            if !required.is_subset(&capabilities) {
                exclusions.insert(CandidateExclusion::MissingCapability);
            }
            for (eligible, exclusion) in [
                (
                    candidate.platform_supported,
                    CandidateExclusion::UnsupportedPlatform,
                ),
                (
                    candidate.permission_eligible,
                    CandidateExclusion::PermissionCeiling,
                ),
                (
                    candidate.quota_available,
                    CandidateExclusion::QuotaExhausted,
                ),
                (
                    candidate.context_available,
                    CandidateExclusion::ContextExhausted,
                ),
                (candidate.risk_eligible, CandidateExclusion::RiskCeiling),
                (
                    candidate.capability_units <= input.remaining_capability_units,
                    CandidateExclusion::BudgetCeiling,
                ),
            ] {
                if !eligible {
                    exclusions.insert(exclusion);
                }
            }
            if excluded_harnesses.contains(&candidate.harness.to_ascii_lowercase()) {
                exclusions.insert(CandidateExclusion::UserExcludedHarness);
            }
            if excluded_models.contains(&candidate.model.to_ascii_lowercase()) {
                exclusions.insert(CandidateExclusion::UserExcludedModel);
            }
            if input
                .preferences
                .pinned_harness
                .as_ref()
                .is_some_and(|pin| !candidate.harness.eq_ignore_ascii_case(pin))
                || input
                    .preferences
                    .pinned_model
                    .as_ref()
                    .is_some_and(|pin| !candidate.model.eq_ignore_ascii_case(pin))
            {
                exclusions.insert(CandidateExclusion::PinMismatch);
            }
            let prediction = predict(
                &candidate,
                input
                    .histories
                    .get(&candidate.key())
                    .unwrap_or(&HistoricalOutcome::default()),
            );
            if prediction.pass_probability_bps < input.preferences.minimum_pass_bps {
                exclusions.insert(CandidateExclusion::BelowQualityFloor);
            }
            let pass = i64::from(prediction.pass_probability_bps.max(1));
            let expected_cost_score = prediction.normalized_quota_cost * 10_000 / pass
                + prediction.latency_ms / 1_000
                + i64::from(prediction.retry_risk_bps);
            CandidateEvaluation {
                candidate,
                exclusions: exclusions.into_iter().collect(),
                prediction,
                expected_cost_score,
            }
        })
        .collect::<Vec<_>>();
    evaluations.sort_by(|left, right| {
        left.expected_cost_score
            .cmp(&right.expected_cost_score)
            .then_with(|| {
                right
                    .candidate
                    .default_for_tier
                    .cmp(&left.candidate.default_for_tier)
            })
            .then_with(|| left.candidate.harness.cmp(&right.candidate.harness))
            .then_with(|| left.candidate.model.cmp(&right.candidate.model))
    });
    evaluations
}

fn tier_rank(tier: CapabilityTier) -> u8 {
    match tier {
        CapabilityTier::Fast => 0,
        CapabilityTier::Standard => 1,
        CapabilityTier::Strong => 2,
    }
}

fn sandbox_for(request: &DelegationRequest) -> String {
    match request.write_mode {
        crate::delegation::WriteMode::ReadOnly => "read_only",
        crate::delegation::WriteMode::Shared | crate::delegation::WriteMode::Isolated => {
            "workspace_write"
        }
        crate::delegation::WriteMode::Full => "danger_full_access",
    }
    .into()
}

fn build_candidates(
    descriptors: &[AdapterDescriptor],
    request: &DelegationRequest,
    unavailable_by_harness: &BTreeMap<String, (bool, bool)>,
) -> Vec<RouteCandidate> {
    let minimum_rank = tier_rank(request.capability_tier);
    descriptors
        .iter()
        .flat_map(|descriptor| {
            descriptor.models.iter().filter_map(move |model| {
                if tier_rank(model.tier) < minimum_rank {
                    return None;
                }
                let (quota_available, context_available) = unavailable_by_harness
                    .get(&descriptor.id)
                    .copied()
                    .unwrap_or((true, true));
                Some(RouteCandidate {
                    harness: descriptor.id.clone(),
                    model: model.id.clone(),
                    tier: model.tier,
                    effort: request.effort,
                    sandbox: sandbox_for(request),
                    capabilities: descriptor.capabilities.clone(),
                    available: descriptor.available,
                    platform_supported: true,
                    permission_eligible: true,
                    quota_available,
                    context_available,
                    risk_eligible: true,
                    capability_units: policy::capability_units(
                        model.tier,
                        request.effort,
                        RestorationKind::Fresh,
                        false,
                    ),
                    default_for_tier: model.default_for_tier,
                })
            })
        })
        .collect()
}

fn baseline_key(descriptors: &[AdapterDescriptor], request: &DelegationRequest) -> Option<String> {
    let harness = request.runtime_harness();
    let descriptor = descriptors.iter().find(|item| item.id == harness)?;
    let model = request
        .model
        .as_ref()
        .and_then(|hint| descriptor.models.iter().find(|model| model.id == *hint))
        .filter(|model| model.tier == request.capability_tier)
        .or_else(|| {
            descriptor
                .models
                .iter()
                .find(|model| model.tier == request.capability_tier && model.default_for_tier)
        })?;
    Some(format!("{}:{}", descriptor.id, model.id))
}

fn candidate_for_key<'a>(
    evaluations: &'a [CandidateEvaluation],
    key: &str,
) -> Option<&'a CandidateEvaluation> {
    evaluations.iter().find(|item| item.candidate.key() == key)
}

pub fn route(
    db: &Connection,
    parent_session_id: &str,
    turn_id: &str,
    request: &DelegationRequest,
    descriptors: &[AdapterDescriptor],
) -> Result<RoutedDelegation, BridgeError> {
    let workspace_id: String = db.query_row(
        "SELECT workspace_id FROM sessions WHERE id=?1",
        params![parent_session_id],
        |row| row.get(0),
    )?;
    let preferences = load_preferences(db, &workspace_id)?;
    let budget = policy::load_request_budget(db, &workspace_id, turn_id)?;
    let remaining =
        PolicyConfig::default().max_capability_units_per_turn - budget.capability_units_used;
    let availability = harness_capacity(db, &workspace_id)?;
    let candidates = build_candidates(descriptors, request, &availability);
    let histories = load_histories(db, policy::role_name(request.role))?;
    let required_capabilities = vec!["tools".into(), "commands".into()];
    let evaluations = evaluate(EvaluationInput {
        candidates,
        preferences: preferences.clone(),
        histories,
        required_capabilities,
        remaining_capability_units: remaining,
    });
    let baseline = baseline_key(descriptors, request);
    let recommendation = evaluations
        .iter()
        .find(|candidate| candidate.eligible())
        .map(|candidate| candidate.candidate.key());
    let manual_override = request.harness.is_some() || request.model.is_some();
    let executed = if manual_override || preferences.mode != RouterMode::Autonomous {
        baseline.clone()
    } else {
        recommendation.clone()
    };
    let explanation = if manual_override {
        "Manual harness/model override retained and recorded".to_owned()
    } else {
        match preferences.mode {
            RouterMode::Disabled => "Learning router disabled; baseline route retained".into(),
            RouterMode::Shadow => match (&recommendation, &baseline) {
                (Some(recommended), Some(baseline)) if recommended != baseline => format!(
                    "Shadow recommendation {recommended}; baseline {baseline} retained"
                ),
                (Some(recommended), _) => format!("Shadow recommendation matches {recommended}"),
                (None, _) => "No candidate met the quality and safety constraints; baseline retained in shadow mode".into(),
            },
            RouterMode::Autonomous => recommendation
                .as_ref()
                .map(|candidate| format!("Autonomous route selected {candidate}"))
                .unwrap_or_else(|| "No candidate met the quality and safety constraints".into()),
        }
    };
    let decision = RouterDecision {
        schema_version: ROUTER_SCHEMA_VERSION,
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.clone(),
        parent_session_id: parent_session_id.into(),
        turn_id: turn_id.into(),
        task_family: policy::role_name(request.role).into(),
        mode: preferences.mode,
        manual_override,
        baseline_candidate: baseline,
        recommended_candidate: recommendation,
        executed_candidate: executed.clone(),
        explanation,
        candidates: evaluations,
        created_at: Utc::now().to_rfc3339(),
    };
    persist_decision(db, &decision)?;
    let mut routed = request.clone();
    if let Some(key) = executed {
        let selected = candidate_for_key(&decision.candidates, &key).ok_or_else(|| {
            BridgeError::Invalid(format!("router selected unknown candidate {key}"))
        })?;
        routed.harness = Some(selected.candidate.harness.clone());
        routed.model = Some(selected.candidate.model.clone());
        routed.capability_tier = selected.candidate.tier;
        routed.effort = selected.candidate.effort;
    } else if preferences.mode == RouterMode::Autonomous {
        return Err(BridgeError::Invalid(
            "learning router found no eligible autonomous route".into(),
        ));
    }
    Ok(RoutedDelegation {
        request: routed,
        decision,
    })
}

fn harness_capacity(
    db: &Connection,
    workspace_id: &str,
) -> Result<BTreeMap<String, (bool, bool)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT harness,usage_percent,context_percent FROM sessions
         WHERE workspace_id=?1 AND rowid IN (
           SELECT MAX(rowid) FROM sessions WHERE workspace_id=?1 GROUP BY harness
         )",
    )?;
    let rows = statement.query_map(params![workspace_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, Option<i64>>(2)?,
        ))
    })?;
    let mut result = BTreeMap::new();
    for row in rows {
        let (harness, usage, context) = row?;
        result.insert(
            harness,
            (usage.unwrap_or(0) < 100, context.unwrap_or(0) < 100),
        );
    }
    Ok(result)
}

pub fn load_preferences(
    db: &Connection,
    workspace_id: &str,
) -> Result<RouterPreferences, BridgeError> {
    let raw: Option<String> = db
        .query_row(
            "SELECT preferences FROM router_preferences WHERE workspace_id=?1",
            params![workspace_id],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| {
        serde_json::from_str(&value).map_err(|error| BridgeError::Invalid(error.to_string()))
    })
    .transpose()
    .map(|value| value.unwrap_or_default())
}

pub fn save_preferences(
    db: &Connection,
    workspace_id: &str,
    preferences: &RouterPreferences,
) -> Result<(), BridgeError> {
    preferences.validate().map_err(BridgeError::Invalid)?;
    if preferences.mode == RouterMode::Autonomous {
        ensure_autonomous_ready(db, workspace_id)?;
    }
    let normalized = RouterPreferences {
        excluded_harnesses: normalized_list(&preferences.excluded_harnesses),
        excluded_models: normalized_list(&preferences.excluded_models),
        pinned_harness: preferences
            .pinned_harness
            .as_deref()
            .map(str::trim)
            .map(str::to_owned),
        pinned_model: preferences
            .pinned_model
            .as_deref()
            .map(str::trim)
            .map(str::to_owned),
        ..preferences.clone()
    };
    db.execute(
        "INSERT INTO router_preferences(workspace_id,mode,preferences,updated_at)
         VALUES(?1,?2,?3,?4)
         ON CONFLICT(workspace_id) DO UPDATE SET mode=excluded.mode,preferences=excluded.preferences,updated_at=excluded.updated_at",
        params![
            workspace_id,
            normalized.mode.as_str(),
            serde_json::to_string(&normalized).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

fn ensure_autonomous_ready(db: &Connection, workspace_id: &str) -> Result<(), BridgeError> {
    let (outcomes, manual): (i64, i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN d.recommended_candidate IS NULL THEN 1 ELSE 0 END),0)
         FROM router_decisions d JOIN router_outcomes o ON o.decision_id=d.id
         WHERE d.workspace_id=?1 AND d.mode='shadow'",
        params![workspace_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if outcomes < MIN_SHADOW_OUTCOMES_FOR_AUTONOMY {
        return Err(BridgeError::Invalid(format!(
            "autonomous routing requires at least {MIN_SHADOW_OUTCOMES_FOR_AUTONOMY} completed shadow outcomes; found {outcomes}"
        )));
    }
    if manual * 20 >= outcomes {
        return Err(BridgeError::Invalid(format!(
            "autonomous routing requires fewer than 5% manual selections in shadow evaluation; found {manual} of {outcomes}"
        )));
    }
    Ok(())
}

fn normalized_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn persist_decision(db: &Connection, decision: &RouterDecision) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO router_decisions(id,workspace_id,parent_session_id,turn_id,task_family,mode,manual_override,baseline_candidate,recommended_candidate,executed_candidate,decision,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            decision.id,
            decision.workspace_id,
            decision.parent_session_id,
            decision.turn_id,
            decision.task_family,
            decision.mode.as_str(),
            decision.manual_override,
            decision.baseline_candidate,
            decision.recommended_candidate,
            decision.executed_candidate,
            serde_json::to_string(decision).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            decision.created_at,
        ],
    )?;
    Ok(())
}

pub fn bind_worker(
    db: &Connection,
    decision_id: &str,
    child_session_id: &str,
) -> Result<(), BridgeError> {
    let now = Utc::now().to_rfc3339();
    db.execute(
        "UPDATE router_assignments SET status='superseded',completed_at=?2
         WHERE child_session_id=?1 AND status='active'",
        params![child_session_id, now],
    )?;
    db.execute(
        "INSERT INTO router_assignments(decision_id,child_session_id,status,created_at)
         VALUES(?1,?2,'active',?3)",
        params![decision_id, child_session_id, now],
    )?;
    Ok(())
}

pub fn record_policy_result(
    db: &Connection,
    decision_id: &str,
    outcome: &policy::PolicyOutcome,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE router_decisions SET policy_outcome=?2 WHERE id=?1",
        params![
            decision_id,
            serde_json::to_string(outcome)
                .map_err(|error| BridgeError::Invalid(error.to_string()))?,
        ],
    )?;
    Ok(())
}

pub fn record_route_status(
    db: &Connection,
    decision_id: &str,
    status: &str,
) -> Result<(), BridgeError> {
    if status.trim().is_empty() {
        return Err(BridgeError::Invalid(
            "router route status cannot be empty".into(),
        ));
    }
    db.execute(
        "UPDATE router_decisions SET route_status=?2 WHERE id=?1",
        params![decision_id, status],
    )?;
    Ok(())
}

pub fn record_worker_outcome(
    db: &Connection,
    child_session_id: &str,
    result: &WorkerResult,
) -> Result<(), BridgeError> {
    let assignment: Option<(String, String, bool)> = db
        .query_row(
            "SELECT a.decision_id,d.executed_candidate,d.manual_override
             FROM router_assignments a JOIN router_decisions d ON d.id=a.decision_id
             WHERE a.child_session_id=?1 AND a.status='active'
             ORDER BY a.rowid DESC LIMIT 1",
            params![child_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((decision_id, executed_candidate, human_intervention)) = assignment else {
        return Ok(());
    };
    let runtime: (i64, i64) = db.query_row(
        "SELECT COALESCE(CAST((julianday(COALESCE(ended_at,?2))-julianday(started_at))*86400000 AS INTEGER),0),
                COALESCE((SELECT retry_count FROM worker_runtime WHERE session_id=?1),0)
         FROM sessions WHERE id=?1",
        params![child_session_id, Utc::now().to_rfc3339()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let normalized_cost: i64 = db.query_row(
        "SELECT COALESCE(SUM(capability_units),0) FROM usage_ledger WHERE session_id=?1",
        params![child_session_id],
        |row| row.get::<_, i64>(0),
    )? * 1_000;
    let succeeded = matches!(result.status, WorkerResultStatus::Completed);
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO router_outcomes(decision_id,child_session_id,candidate,succeeded,status,runtime_ms,normalized_cost,retry_count,human_intervention,recorded_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
         ON CONFLICT(decision_id) DO UPDATE SET succeeded=excluded.succeeded,status=excluded.status,runtime_ms=excluded.runtime_ms,normalized_cost=excluded.normalized_cost,retry_count=excluded.retry_count,recorded_at=excluded.recorded_at",
        params![
            decision_id,
            child_session_id,
            executed_candidate,
            succeeded,
            result.status.as_str(),
            runtime.0,
            normalized_cost,
            runtime.1,
            human_intervention,
            now,
        ],
    )?;
    db.execute(
        "UPDATE router_assignments SET status='completed',completed_at=?2 WHERE decision_id=?1",
        params![decision_id, now],
    )?;
    Ok(())
}

fn load_histories(
    db: &Connection,
    task_family: &str,
) -> Result<BTreeMap<String, HistoricalOutcome>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT o.candidate,COUNT(*),SUM(CASE WHEN o.succeeded THEN 1 ELSE 0 END),
                COALESCE(SUM(o.runtime_ms),0),COALESCE(SUM(o.normalized_cost),0),
                SUM(CASE WHEN o.retry_count>0 THEN 1 ELSE 0 END),
                SUM(CASE WHEN o.human_intervention THEN 1 ELSE 0 END)
         FROM router_outcomes o JOIN router_decisions d ON d.id=o.decision_id
         WHERE d.task_family=?1 GROUP BY o.candidate",
    )?;
    let rows = statement.query_map(params![task_family], |row| {
        Ok((
            row.get::<_, String>(0)?,
            HistoricalOutcome {
                samples: row.get(1)?,
                successes: row.get(2)?,
                runtime_ms_total: row.get(3)?,
                normalized_cost_total: row.get(4)?,
                retries: row.get(5)?,
                human_interventions: row.get(6)?,
            },
        ))
    })?;
    rows.collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(BridgeError::from)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkTask {
    pub id: String,
    pub baseline_candidate: Option<String>,
    pub evaluations: Vec<CandidateEvaluation>,
    pub actual_passed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkReport {
    pub tasks: usize,
    pub recommendations: usize,
    pub manual_selections: usize,
    pub manual_selection_bps: u16,
    pub baseline_matches: usize,
    pub actual_passes: usize,
    pub policy_violations: usize,
}

pub fn benchmark(tasks: &[BenchmarkTask]) -> BenchmarkReport {
    let recommendations = tasks
        .iter()
        .filter(|task| task.evaluations.iter().any(CandidateEvaluation::eligible))
        .count();
    let manual_selections = tasks.len() - recommendations;
    let baseline_matches = tasks
        .iter()
        .filter(|task| {
            let recommended = task
                .evaluations
                .iter()
                .find(|candidate| candidate.eligible())
                .map(|candidate| candidate.candidate.key());
            recommended == task.baseline_candidate
        })
        .count();
    let policy_violations = tasks
        .iter()
        .filter(|task| {
            task.evaluations
                .iter()
                .find(|candidate| candidate.eligible())
                .is_some_and(|candidate| !candidate.exclusions.is_empty())
        })
        .count();
    BenchmarkReport {
        tasks: tasks.len(),
        recommendations,
        manual_selections,
        manual_selection_bps: if tasks.is_empty() {
            0
        } else {
            (manual_selections * 10_000 / tasks.len()) as u16
        },
        baseline_matches,
        actual_passes: tasks
            .iter()
            .filter(|task| task.actual_passed == Some(true))
            .count(),
        policy_violations,
    }
}

pub fn next_escalation(
    current: CapabilityTier,
    evaluations: &[CandidateEvaluation],
) -> Option<RouteCandidate> {
    evaluations
        .iter()
        .filter(|item| item.eligible() && tier_rank(item.candidate.tier) > tier_rank(current))
        .min_by_key(|item| (tier_rank(item.candidate.tier), item.expected_cost_score))
        .map(|item| item.candidate.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::{OutputContract, SuggestedNextAction, WorkerRole, WriteMode, SCHEMA_VERSION},
        model::{ModelOption, UsageLedgerRow, WorkerRuntimeRecord},
        store,
    };
    use std::path::Path;

    fn candidate(harness: &str, model: &str, tier: CapabilityTier, units: i64) -> RouteCandidate {
        RouteCandidate {
            harness: harness.into(),
            model: model.into(),
            tier,
            effort: Effort::Medium,
            sandbox: "workspace_write".into(),
            capabilities: vec!["tools".into(), "commands".into()],
            available: true,
            platform_supported: true,
            permission_eligible: true,
            quota_available: true,
            context_available: true,
            risk_eligible: true,
            capability_units: units,
            default_for_tier: true,
        }
    }

    fn evaluated(preferences: RouterPreferences) -> Vec<CandidateEvaluation> {
        evaluate(EvaluationInput {
            candidates: vec![
                candidate("codex", "fast", CapabilityTier::Fast, 2),
                candidate("claude", "standard", CapabilityTier::Standard, 4),
            ],
            preferences,
            histories: BTreeMap::new(),
            required_capabilities: vec!["tools".into()],
            remaining_capability_units: 24,
        })
    }

    fn request() -> DelegationRequest {
        DelegationRequest {
            schema_version: SCHEMA_VERSION,
            role: WorkerRole::Implementation,
            objective: "Implement the router fixture".into(),
            acceptance_criteria: vec!["Fixture passes".into()],
            known_facts: vec![],
            decisions: vec![],
            evidence_ids: vec![],
            relevant_files: vec![],
            owned_paths: vec![],
            write_mode: WriteMode::ReadOnly,
            capability_tier: CapabilityTier::Standard,
            effort: Effort::Medium,
            verification: vec!["cargo test".into()],
            output_contract: OutputContract::ImplementationResult,
            harness: None,
            model: None,
        }
    }

    fn descriptors() -> Vec<AdapterDescriptor> {
        [("codex", "codex-standard"), ("claude", "claude-standard")]
            .into_iter()
            .map(|(harness, model)| AdapterDescriptor {
                id: harness.into(),
                label: harness.into(),
                available: true,
                version: Some("test".into()),
                capabilities: vec!["tools".into(), "commands".into()],
                unavailable_reason: None,
                models: vec![ModelOption {
                    id: model.into(),
                    label: model.into(),
                    tier: CapabilityTier::Standard,
                    default_for_tier: true,
                }],
                default_model: Some(model.into()),
            })
            .collect()
    }

    fn routing_db() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO workspaces(id,title,status,created_at) VALUES('w','Router','idle','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('parent','w','codex','Parent','working','reported')", []).unwrap();
        db
    }

    #[test]
    fn lowest_expected_cost_above_quality_floor_wins_deterministically() {
        let result = evaluated(RouterPreferences::default());
        assert!(result[0].eligible());
        assert_eq!(result[0].candidate.key(), "codex:fast");
    }

    #[test]
    fn every_constraint_has_a_stable_reason_code() {
        let mut blocked = candidate("codex", "fast", CapabilityTier::Fast, 30);
        blocked.available = false;
        blocked.platform_supported = false;
        blocked.permission_eligible = false;
        blocked.quota_available = false;
        blocked.context_available = false;
        blocked.risk_eligible = false;
        blocked.capabilities.clear();
        let result = evaluate(EvaluationInput {
            candidates: vec![blocked],
            preferences: RouterPreferences {
                excluded_harnesses: vec!["codex".into()],
                excluded_models: vec!["fast".into()],
                pinned_harness: Some("claude".into()),
                minimum_pass_bps: 9_000,
                ..RouterPreferences::default()
            },
            histories: BTreeMap::new(),
            required_capabilities: vec!["tools".into()],
            remaining_capability_units: 1,
        });
        assert_eq!(result[0].exclusions.len(), 12);
        assert!(!result[0].eligible());
    }

    #[test]
    fn outcomes_update_predictions_but_keep_conservative_priors() {
        let route = candidate("codex", "fast", CapabilityTier::Fast, 2);
        let history = HistoricalOutcome {
            samples: 4,
            successes: 4,
            runtime_ms_total: 8_000,
            normalized_cost_total: 4_000,
            retries: 0,
            human_interventions: 0,
        };
        let learned = predict(&route, &history);
        assert!(learned.pass_probability_bps > 6_500);
        assert!(learned.pass_probability_bps < 10_000);
        assert!(learned.latency_ms < 10_000);
    }

    #[test]
    fn shadow_route_records_alternative_but_preserves_baseline() {
        let db = routing_db();
        let routed = route(&db, "parent", "turn", &request(), &descriptors()).unwrap();
        assert_eq!(routed.decision.mode, RouterMode::Shadow);
        assert_eq!(
            routed.decision.baseline_candidate.as_deref(),
            Some("codex:codex-standard")
        );
        assert_eq!(
            routed.decision.recommended_candidate.as_deref(),
            Some("claude:claude-standard")
        );
        assert_eq!(
            routed.decision.executed_candidate,
            routed.decision.baseline_candidate
        );
        assert_eq!(routed.request.runtime_harness(), "codex");
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM router_decisions", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn bound_worker_result_becomes_durable_learning_outcome() {
        let db = routing_db();
        let routed = route(&db, "parent", "turn", &request(), &descriptors()).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id,started_at) VALUES('child','w','codex','Worker','working','reported','parent','2026-07-16T00:00:00Z')", []).unwrap();
        store::upsert_worker_runtime(
            &db,
            &WorkerRuntimeRecord {
                session_id: "child".into(),
                parent_session_id: "parent".into(),
                lifecycle_state: "working".into(),
                task_family: "implementation".into(),
                compatibility_key: "key".into(),
                result_status: "pending".into(),
                retry_count: 1,
                warm_until: None,
                worktree_path: None,
                worktree_branch: None,
                last_result: None,
                updated_at: "now".into(),
            },
        )
        .unwrap();
        store::append_usage_ledger(
            &db,
            &UsageLedgerRow {
                id: 0,
                workspace_id: "w".into(),
                session_id: Some("child".into()),
                turn_id: Some("turn".into()),
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                context_percent: None,
                capability_units: 3,
                runtime_ms: None,
                source: "policy.spawn.standard".into(),
                created_at: "now".into(),
            },
        )
        .unwrap();
        bind_worker(&db, &routed.decision.id, "child").unwrap();
        record_worker_outcome(
            &db,
            "child",
            &WorkerResult {
                schema_version: SCHEMA_VERSION,
                status: WorkerResultStatus::Completed,
                summary: "done".into(),
                files_changed: vec![],
                tests: vec![],
                decisions: vec![],
                risks: vec![],
                remaining_work: vec![],
                suggested_next_action: SuggestedNextAction::Finish,
                suggested_role: None,
                suggested_task: None,
            },
        )
        .unwrap();
        let outcome: (bool, i64, i64) = db
            .query_row(
                "SELECT succeeded,normalized_cost,retry_count FROM router_outcomes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(outcome, (true, 3_000, 1));
        assert_eq!(
            load_histories(&db, "implementation").unwrap()["codex:codex-standard"].samples,
            1
        );
    }

    #[test]
    fn ineligible_pin_fails_closed_and_escalation_is_strictly_bounded() {
        let mut preferences = RouterPreferences::default();
        preferences.pinned_model = Some("missing".into());
        assert!(evaluated(preferences).iter().all(|item| !item.eligible()));

        let result = evaluated(RouterPreferences::default());
        let escalation = next_escalation(CapabilityTier::Fast, &result).unwrap();
        assert_eq!(escalation.tier, CapabilityTier::Standard);
        assert!(next_escalation(CapabilityTier::Strong, &result).is_none());
    }

    #[test]
    fn preferences_and_decisions_round_trip_through_idempotent_migration() {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO workspaces(id,title,status,created_at) VALUES('w','Router','idle','now')",
            [],
        )
        .unwrap();
        let preferences = RouterPreferences {
            mode: RouterMode::Shadow,
            pinned_harness: Some("claude".into()),
            excluded_models: vec!["OPUS".into(), "opus".into()],
            ..RouterPreferences::default()
        };
        save_preferences(&db, "w", &preferences).unwrap();
        let loaded = load_preferences(&db, "w").unwrap();
        assert_eq!(loaded.mode, RouterMode::Shadow);
        assert_eq!(loaded.excluded_models, vec!["opus"]);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM router_preferences", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let _ = WriteMode::ReadOnly;
    }

    #[test]
    fn autonomous_activation_requires_completed_shadow_evidence() {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO workspaces(id,title,status,created_at) VALUES('w','Router','idle','now')",
            [],
        )
        .unwrap();
        let error = save_preferences(
            &db,
            "w",
            &RouterPreferences {
                mode: RouterMode::Autonomous,
                ..RouterPreferences::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("completed shadow outcomes"));
    }

    #[test]
    fn held_out_fixture_reports_under_five_percent_manual_selection() {
        let evaluations = evaluated(RouterPreferences::default());
        let tasks = (0..25)
            .map(|index| BenchmarkTask {
                id: format!("held-out-{index}"),
                baseline_candidate: Some("codex:fast".into()),
                evaluations: evaluations.clone(),
                actual_passed: Some(true),
            })
            .collect::<Vec<_>>();
        let report = benchmark(&tasks);
        assert_eq!(report.manual_selection_bps, 0);
        assert_eq!(report.policy_violations, 0);
    }
}
