//! Cost-and-quality routing that learns from durable worker outcomes.
//!
//! The learning layer ranks provider/model candidates, while [`crate::policy`]
//! remains the non-bypassable authority for permissions, topology, worktrees,
//! and budgets. Shadow mode is the default so recommendations can be measured
//! before a workspace explicitly enables autonomous selection.

use crate::{
    delegation::{
        DelegationRequest, Effort, TestStatus, WorkerResult, WorkerResultStatus, WorkerRole,
    },
    model::{AdapterDescriptor, CapabilityTier},
    policy::{self, PolicyConfig, RestorationKind},
    BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const ROUTER_SCHEMA_VERSION: u32 = 2;
pub const MIN_SHADOW_OUTCOMES_FOR_AUTONOMY: i64 = 20;
pub const LEGACY_GLOBAL_SCOPE: &str = "legacy:global";
const PRIOR_WEIGHT: i64 = 4;

/// Stable learning-policy key for a workspace. Direct chats have no workspace
/// and must not share a NULL/`legacy:global` bucket.
pub fn workspace_learning_scope(workspace_id: &str) -> Result<String, BridgeError> {
    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        return Err(BridgeError::Invalid(
            "learning scope requires a workspace id".into(),
        ));
    }
    Ok(format!("workspace:{workspace_id}"))
}

pub fn workspace_id_from_scope(scope: &str) -> Result<&str, BridgeError> {
    scope
        .strip_prefix("workspace:")
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "learning scope {scope} is not a workspace scope"
            ))
        })
}

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
    SameAsImplementer,
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
    #[serde(default)]
    pub trace_id: Option<String>,
    pub task_family: String,
    #[serde(default)]
    pub task_fingerprint: String,
    #[serde(default)]
    pub repository_revision: Option<String>,
    #[serde(default)]
    pub profile_version: Option<i64>,
    #[serde(default)]
    pub profile_purpose: Option<String>,
    #[serde(default = "default_policy_version")]
    pub policy_version: i64,
    #[serde(default)]
    pub catalog_snapshot: serde_json::Value,
    pub mode: RouterMode,
    pub manual_override: bool,
    pub baseline_candidate: Option<String>,
    pub recommended_candidate: Option<String>,
    pub executed_candidate: Option<String>,
    pub explanation: String,
    pub candidates: Vec<CandidateEvaluation>,
    #[serde(default)]
    pub actual_provider: Option<String>,
    #[serde(default)]
    pub actual_model: Option<String>,
    #[serde(default)]
    pub actual_effort: Option<Effort>,
    pub created_at: String,
}

fn default_policy_version() -> i64 {
    1
}

fn task_fingerprint(request: &DelegationRequest) -> String {
    let body = serde_json::json!({
        "role": request.role,
        "acceptanceCriteria": request.acceptance_criteria,
        "relevantFiles": request.relevant_files,
        "writeMode": request.write_mode,
        "capabilityTier": request.capability_tier,
        "verification": request.verification,
        "outputContract": request.output_contract,
    });
    format!("{:x}", Sha256::digest(body.to_string().as_bytes()))
}

fn repository_revision(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    let state = crate::store::repository_state_for_session(db, session_id)?;
    Ok(state
        .get("head")
        .and_then(serde_json::Value::as_str)
        .map(|head| {
            format!(
                "{}:{}",
                head,
                state
                    .get("dirtyHash")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown")
            )
        }))
}

fn active_policy(
    db: &Connection,
    workspace_id: &str,
    fingerprint: &str,
) -> Result<(i64, BTreeMap<String, String>), BridgeError> {
    let scope = workspace_learning_scope(workspace_id)?;
    let Some((mut version, status, predecessor, mut weights)) = db
        .query_row(
            "SELECT version,status,predecessor,weights FROM routing_policies
             WHERE learning_scope=?1 AND status IN ('active','canary')
             ORDER BY CASE status WHEN 'canary' THEN 0 ELSE 1 END,version DESC LIMIT 1",
            params![scope],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok((0, BTreeMap::new()));
    };
    if status == "canary" && canary_bucket(fingerprint) >= 20 {
        if let Some(predecessor) = predecessor {
            (version, weights) = db.query_row(
                "SELECT version,weights FROM routing_policies WHERE version=?1 AND learning_scope=?2",
                params![predecessor, scope],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
        }
    }
    let value: serde_json::Value = serde_json::from_str(&weights).unwrap_or_default();
    let preferred = value
        .get("preferredCandidates")
        .and_then(serde_json::Value::as_object)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.into())))
                .collect()
        })
        .unwrap_or_default();
    Ok((version, preferred))
}

fn canary_bucket(fingerprint: &str) -> u8 {
    let digest = Sha256::digest(fingerprint.as_bytes());
    (u16::from_be_bytes([digest[0], digest[1]]) % 100) as u8
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
    pub budget_preference: Option<String>,
    pub latency_preference: Option<String>,
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
            let cost_score = prediction.normalized_quota_cost * 10_000 / pass;
            let cost_score = match input.budget_preference.as_deref() {
                Some("economy") => cost_score.saturating_mul(2),
                Some("quality") => cost_score / 2,
                _ => cost_score,
            };
            let latency_score = match input.latency_preference.as_deref() {
                Some("fast") => prediction.latency_ms / 100,
                Some("patient") => prediction.latency_ms / 5_000,
                _ => prediction.latency_ms / 1_000,
            };
            let quality_score = if input.budget_preference.as_deref() == Some("quality") {
                (10_000 - pass).saturating_mul(10)
            } else {
                0
            };
            let expected_cost_score =
                cost_score + latency_score + quality_score + i64::from(prediction.retry_risk_bps);
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

pub fn sandbox_mode_for(request: &DelegationRequest) -> crate::model::SandboxMode {
    use crate::model::SandboxMode;
    match request.write_mode {
        crate::delegation::WriteMode::ReadOnly => SandboxMode::ReadOnly,
        crate::delegation::WriteMode::Shared | crate::delegation::WriteMode::Isolated => {
            SandboxMode::WorkspaceWrite
        }
        crate::delegation::WriteMode::Full => SandboxMode::DangerFullAccess,
    }
}

fn sandbox_for(request: &DelegationRequest) -> String {
    sandbox_mode_for(request).as_str().into()
}

fn build_candidates(
    descriptors: &[AdapterDescriptor],
    request: &DelegationRequest,
    unavailable_by_harness: &BTreeMap<String, (bool, bool)>,
) -> Vec<RouteCandidate> {
    let minimum_rank = tier_rank(request.capability_tier);
    let sandbox_mode = sandbox_mode_for(request);
    descriptors
        .iter()
        .flat_map(|descriptor| {
            // A harness that cannot start in the requested sandbox mode is not a
            // candidate at all: reserving it would create a worker session that
            // the adapter is guaranteed to reject at startup.
            let sandbox_supported = descriptor.supports_sandbox(sandbox_mode);
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
                    permission_eligible: sandbox_supported,
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
    let (workspace_id, trace_id): (Option<String>, Option<String>) = db.query_row(
        "SELECT workspace_id,trace_id FROM sessions WHERE id=?1",
        params![parent_session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let workspace_id = workspace_id
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            BridgeError::Invalid(
                "learning router cannot route a session without a workspace; direct chats are excluded from policy learning".into(),
            )
        })?;
    let fingerprint = task_fingerprint(request);
    let preferences = load_preferences(db, &workspace_id)?;
    let (policy_version, preferred_candidates) = active_policy(db, &workspace_id, &fingerprint)?;
    let budget = policy::load_request_budget(db, &workspace_id, turn_id)?;
    let remaining =
        PolicyConfig::default().max_capability_units_per_turn - budget.capability_units_used;
    let resolved_profile = if request.harness.is_none() && request.model.is_none() {
        crate::model_profiles::resolve_for_role(db, descriptors, request.role)?
    } else {
        None
    };
    let mut profiled_request = request.clone();
    if let Some(profile) = &resolved_profile {
        profiled_request.effort = profile.effort;
    }
    let availability = harness_capacity(db, &workspace_id)?;
    let candidates = build_candidates(descriptors, &profiled_request, &availability);
    let histories = load_histories(db, &workspace_id, policy::role_name(request.role))?;
    let required_capabilities = vec!["tools".into(), "commands".into()];
    let mut evaluations = evaluate(EvaluationInput {
        candidates,
        preferences: preferences.clone(),
        histories,
        required_capabilities,
        remaining_capability_units: remaining,
        budget_preference: resolved_profile
            .as_ref()
            .and_then(|profile| profile.budget_preference.clone()),
        latency_preference: resolved_profile
            .as_ref()
            .and_then(|profile| profile.latency_preference.clone()),
    });
    let implementer_family: Option<String> = if request.role == WorkerRole::Verification {
        db.query_row(
            "SELECT implementer_family FROM eval_attempts WHERE session_id=?1 AND status IN ('verifying','changes_requested') ORDER BY started_at DESC LIMIT 1",
            params![parent_session_id],
            |row| row.get(0),
        ).optional()?.flatten()
    } else {
        None
    };
    if let Some(implementer_family) = implementer_family.as_deref() {
        for evaluation in &mut evaluations {
            if evaluation
                .candidate
                .harness
                .eq_ignore_ascii_case(implementer_family)
            {
                evaluation
                    .exclusions
                    .push(CandidateExclusion::SameAsImplementer);
                evaluation.exclusions.sort();
                evaluation.exclusions.dedup();
            }
        }
    }
    let profile_locked = resolved_profile
        .as_ref()
        .is_some_and(|profile| profile.pinned || !profile.learning_enabled);
    let profile_baseline = resolved_profile
        .as_ref()
        .map(|profile| format!("{}:{}", profile.provider, profile.model))
        .filter(|key| candidate_for_key(&evaluations, key).is_some());
    let baseline = profile_baseline.or_else(|| baseline_key(descriptors, request));
    let policy_preference = preferred_candidates
        .get(&fingerprint)
        .or_else(|| preferred_candidates.get(policy::role_name(request.role)))
        .filter(|key| {
            candidate_for_key(&evaluations, key).is_some_and(CandidateEvaluation::eligible)
        })
        .cloned();
    let recommendation = if profile_locked {
        baseline
            .as_ref()
            .filter(|key| {
                candidate_for_key(&evaluations, key).is_some_and(CandidateEvaluation::eligible)
            })
            .cloned()
    } else {
        policy_preference.or_else(|| {
            evaluations
                .iter()
                .find(|candidate| candidate.eligible())
                .map(|candidate| candidate.candidate.key())
        })
    };
    let manual_override = request.harness.is_some() || request.model.is_some();
    let independent_verification = implementer_family.is_some();
    let eligible_manual_baseline = baseline.as_deref().filter(|key| {
        candidate_for_key(&evaluations, key).is_some_and(CandidateEvaluation::eligible)
    });
    let executed =
        if independent_verification && manual_override && eligible_manual_baseline.is_some() {
            baseline.clone()
        } else if independent_verification {
            recommendation.clone()
        } else if manual_override || profile_locked || preferences.mode != RouterMode::Autonomous {
            baseline.clone()
        } else {
            recommendation.clone()
        };
    let explanation = if independent_verification
        && manual_override
        && eligible_manual_baseline.is_some()
    {
        format!(
            "Manual different-family verifier {} retained",
            baseline.as_deref().unwrap_or("unknown")
        )
    } else if independent_verification {
        recommendation.as_ref().map(|candidate| format!("Independent verification requires a different harness family; selected {candidate}"))
            .unwrap_or_else(|| "No different-family verifier satisfies the deterministic route constraints".into())
    } else if manual_override {
        "Manual harness/model override retained and recorded".to_owned()
    } else if profile_locked {
        "Pinned or learning-disabled role profile retained as a deterministic route constraint"
            .to_owned()
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
        trace_id,
        task_family: policy::role_name(request.role).into(),
        task_fingerprint: fingerprint,
        repository_revision: repository_revision(db, parent_session_id)?,
        profile_version: resolved_profile
            .as_ref()
            .map(|profile| profile.profile_version),
        profile_purpose: resolved_profile
            .as_ref()
            .map(|profile| profile.purpose.as_str().to_owned()),
        policy_version,
        catalog_snapshot: serde_json::to_value(descriptors)
            .map_err(|error| BridgeError::Invalid(error.to_string()))?,
        mode: preferences.mode,
        manual_override,
        baseline_candidate: baseline,
        recommended_candidate: recommendation,
        executed_candidate: executed.clone(),
        explanation,
        candidates: evaluations,
        actual_provider: None,
        actual_model: None,
        actual_effort: None,
        created_at: Utc::now().to_rfc3339(),
    };
    persist_decision(db, &decision)?;
    let mut routed = request.clone();
    if let Some(key) = executed {
        let selected = candidate_for_key(&decision.candidates, &key).ok_or_else(|| {
            BridgeError::Invalid(format!("router selected unknown candidate {key}"))
        })?;
        // A permission ceiling is a hard incompatibility, not a preference. A
        // manual or pinned route must fail here with something the caller can
        // act on rather than reserving a worker the adapter will refuse.
        if selected
            .exclusions
            .contains(&CandidateExclusion::PermissionCeiling)
        {
            let sandbox = sandbox_mode_for(request);
            let alternatives = descriptors
                .iter()
                .filter(|descriptor| {
                    descriptor.available
                        && !descriptor.models.is_empty()
                        && descriptor.supports_sandbox(sandbox)
                })
                .map(|descriptor| descriptor.id.as_str())
                .collect::<Vec<_>>();
            return Err(BridgeError::Invalid(format!(
                "{} cannot run a {} worker, so this delegation was not started. {}",
                selected.candidate.harness,
                sandbox.as_str(),
                if alternatives.is_empty() {
                    "No installed harness supports this sandbox mode; change the write mode or install another harness.".to_owned()
                } else {
                    format!(
                        "Re-delegate with a compatible harness ({}) or change the write mode.",
                        alternatives.join(", ")
                    )
                }
            )));
        }
        routed.harness = Some(selected.candidate.harness.clone());
        routed.model = Some(selected.candidate.model.clone());
        routed.capability_tier = selected.candidate.tier;
        routed.effort = selected.candidate.effort;
    } else if preferences.mode == RouterMode::Autonomous || independent_verification {
        let sandbox = sandbox_mode_for(request);
        let sandbox_blocked_every_candidate = !decision.candidates.is_empty()
            && decision.candidates.iter().all(|candidate| {
                candidate
                    .exclusions
                    .contains(&CandidateExclusion::PermissionCeiling)
            });
        return Err(BridgeError::Invalid(if sandbox_blocked_every_candidate {
            format!(
                "no installed harness can run a {} worker, so this delegation was not started; change the write mode or install a compatible harness",
                sandbox.as_str()
            )
        } else {
            "learning router found no eligible route under the required constraints".into()
        }));
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
        "SELECT COUNT(*),COALESCE(SUM(CASE WHEN d.manual_override OR d.recommended_candidate IS NULL THEN 1 ELSE 0 END),0)
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
        "INSERT INTO router_decisions(id,workspace_id,parent_session_id,turn_id,trace_id,task_family,task_fingerprint,repository_revision,profile_version,profile_purpose,policy_version,catalog_snapshot,selection_reason,actual_provider,actual_model,actual_effort,mode,manual_override,baseline_candidate,recommended_candidate,executed_candidate,decision,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
        params![
            decision.id,
            decision.workspace_id,
            decision.parent_session_id,
            decision.turn_id,
            decision.trace_id,
            decision.task_family,
            decision.task_fingerprint,
            decision.repository_revision,
            decision.profile_version,
            decision.profile_purpose,
            decision.policy_version,
            decision.catalog_snapshot.to_string(),
            decision.explanation,
            decision.actual_provider,
            decision.actual_model,
            decision.actual_effort.map(Effort::as_str),
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

pub fn record_actual_execution(
    db: &Connection,
    decision_id: &str,
    provider: &str,
    model: &str,
    effort: Effort,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE router_decisions SET actual_provider=?2,actual_model=?3,actual_effort=?4 WHERE id=?1",
        params![decision_id, provider, model, effort.as_str()],
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
    let fallback_normalized_cost: i64 = db.query_row(
        "SELECT COALESCE(SUM(capability_units),0) FROM usage_ledger WHERE session_id=?1",
        params![child_session_id],
        |row| row.get::<_, i64>(0),
    )? * 1_000;
    let cost: Option<(i64, Option<String>)> = db
        .query_row(
            "SELECT cost_microusd,cost_source FROM usage_ledger WHERE session_id=?1 AND cost_microusd IS NOT NULL ORDER BY id DESC LIMIT 1",
            params![child_session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let total_tokens: Option<i64> = db
        .query_row(
            "SELECT COALESCE(input_tokens,0)+COALESCE(output_tokens,0)+COALESCE(cache_read_tokens,0)+COALESCE(cache_write_tokens,0)
             FROM usage_ledger WHERE session_id=?1 AND (input_tokens IS NOT NULL OR output_tokens IS NOT NULL OR cache_read_tokens IS NOT NULL OR cache_write_tokens IS NOT NULL)
             ORDER BY id DESC LIMIT 1",
            params![child_session_id],
            |row| row.get(0),
        )
        .optional()?;
    let succeeded = matches!(result.status, WorkerResultStatus::Completed);
    let success_state = match result.status {
        WorkerResultStatus::Completed => "success",
        WorkerResultStatus::Failed => "failure",
        WorkerResultStatus::Cancelled
        | WorkerResultStatus::Blocked
        | WorkerResultStatus::NeedsDelegation
        // A transport error says nothing about the route that was chosen, so it
        // must not train the router either way.
        | WorkerResultStatus::ProtocolInvalid => "unknown",
    };
    let has_failed_test = result
        .tests
        .iter()
        .any(|test| test.status == TestStatus::Failed);
    let has_passed_test = result
        .tests
        .iter()
        .any(|test| test.status == TestStatus::Passed);
    let acceptance_state = if has_failed_test || result.status == WorkerResultStatus::Failed {
        "rejected"
    } else if succeeded && has_passed_test {
        "accepted"
    } else {
        "unknown"
    };
    let confidence_bps = if has_failed_test || has_passed_test {
        9_500
    } else if matches!(
        result.status,
        WorkerResultStatus::Completed | WorkerResultStatus::Failed
    ) {
        7_000
    } else {
        4_000
    };
    let evidence_entry_ids = db
        .query_row(
            "SELECT request FROM worker_completion_inputs WHERE child_session_id=?1",
            params![child_session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .and_then(|request| serde_json::from_str::<DelegationRequest>(&request).ok())
        .map(|request| request.evidence_ids)
        .unwrap_or_default();
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO router_outcomes(decision_id,child_session_id,candidate,succeeded,status,runtime_ms,normalized_cost,retry_count,human_intervention,success_state,acceptance_state,cost_microusd,cost_source,confidence_bps,edit_count,override_signal,total_tokens,latency_source,recorded_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)
         ON CONFLICT(decision_id) DO UPDATE SET succeeded=excluded.succeeded,status=excluded.status,runtime_ms=excluded.runtime_ms,normalized_cost=excluded.normalized_cost,retry_count=excluded.retry_count,human_intervention=excluded.human_intervention,success_state=excluded.success_state,acceptance_state=excluded.acceptance_state,cost_microusd=excluded.cost_microusd,cost_source=excluded.cost_source,confidence_bps=excluded.confidence_bps,edit_count=excluded.edit_count,override_signal=excluded.override_signal,total_tokens=excluded.total_tokens,latency_source=excluded.latency_source,recorded_at=excluded.recorded_at",
        params![
            decision_id,
            child_session_id,
            executed_candidate,
            succeeded,
            result.status.as_str(),
            runtime.0,
            fallback_normalized_cost,
            runtime.1,
            human_intervention,
            success_state,
            acceptance_state,
            cost.as_ref().map(|value| value.0),
            cost.as_ref().and_then(|value| value.1.clone()),
            confidence_bps,
            result.files_changed.len() as i64,
            human_intervention,
            total_tokens,
            "session_runtime",
            now,
        ],
    )?;
    db.execute(
        "INSERT INTO routing_evaluations(id,decision_id,evaluator_kind,evaluator_version,score_bps,confidence_bps,evidence_entry_ids,bounded_metrics,status,created_at)
         VALUES(?1,?2,'deterministic','worker-result-v1',?3,?4,?5,?6,'completed',?7)
         ON CONFLICT(id) DO UPDATE SET score_bps=excluded.score_bps,confidence_bps=excluded.confidence_bps,evidence_entry_ids=excluded.evidence_entry_ids,bounded_metrics=excluded.bounded_metrics,status=excluded.status,created_at=excluded.created_at",
        params![
            format!("deterministic:{decision_id}"),
            decision_id,
            match success_state { "success" => Some(10_000_i64), "failure" => Some(0_i64), _ => None },
            confidence_bps,
            serde_json::to_string(&evidence_entry_ids).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            serde_json::json!({
                "successState": success_state,
                "acceptanceState": acceptance_state,
                "runtimeMs": runtime.0,
                "retryCount": runtime.1,
                "editCount": result.files_changed.len(),
                "overrideSignal": human_intervention,
                "costReported": cost.is_some(),
                "tokensReported": total_tokens.is_some(),
            }).to_string(),
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
    workspace_id: &str,
    task_family: &str,
) -> Result<BTreeMap<String, HistoricalOutcome>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT o.candidate,COUNT(*),SUM(CASE WHEN o.succeeded THEN 1 ELSE 0 END),
                COALESCE(SUM(o.runtime_ms),0),COALESCE(SUM(o.normalized_cost),0),
                SUM(CASE WHEN o.retry_count>0 THEN 1 ELSE 0 END),
                SUM(CASE WHEN o.human_intervention THEN 1 ELSE 0 END)
         FROM router_outcomes o JOIN router_decisions d ON d.id=o.decision_id
         WHERE d.workspace_id=?1 AND d.task_family=?2 GROUP BY o.candidate",
    )?;
    let rows = statement.query_map(params![workspace_id, task_family], |row| {
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
        model::{ModelOption, SandboxMode, UsageLedgerRow, WorkerRuntimeRecord},
        store,
    };
    use serde_json::json;
    use std::path::Path;

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct HeldOutObservation {
        id: String,
        recommended: bool,
        manual: bool,
        actual_passed: bool,
        normalized_cost: i64,
        latency_ms: i64,
        policy_violation: bool,
    }

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
            budget_preference: None,
            latency_preference: None,
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
            network_access: false,
            writable_output_paths: vec![],
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
                sandbox_modes: crate::model::SandboxMode::ALL.to_vec(),
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

    /// Autonomous mode normally requires 20 completed shadow outcomes. These
    /// tests exercise the sandbox constraint, not the activation gate, so the
    /// preference row is written directly.
    fn force_autonomous(db: &Connection, workspace_id: &str) {
        db.execute(
            "INSERT INTO router_preferences(workspace_id,mode,preferences,updated_at) VALUES(?1,'autonomous',?2,'now')
             ON CONFLICT(workspace_id) DO UPDATE SET mode=excluded.mode,preferences=excluded.preferences",
            params![
                workspace_id,
                serde_json::to_string(&RouterPreferences {
                    mode: RouterMode::Autonomous,
                    ..RouterPreferences::default()
                })
                .unwrap()
            ],
        )
        .unwrap();
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
    fn profile_quality_preference_changes_ranking_without_widening_eligibility() {
        let result = evaluate(EvaluationInput {
            candidates: vec![
                candidate("codex", "fast", CapabilityTier::Fast, 2),
                candidate("claude", "standard", CapabilityTier::Standard, 4),
            ],
            preferences: RouterPreferences::default(),
            histories: BTreeMap::new(),
            required_capabilities: vec!["tools".into()],
            remaining_capability_units: 24,
            budget_preference: Some("quality".into()),
            latency_preference: Some("patient".into()),
        });
        assert!(result.iter().all(CandidateEvaluation::eligible));
        assert_eq!(result[0].candidate.key(), "claude:standard");
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
            budget_preference: None,
            latency_preference: None,
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
        let evidence: (i64, i64, String, String) = db.query_row(
            "SELECT policy_version,LENGTH(catalog_snapshot),task_fingerprint,selection_reason FROM router_decisions LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(evidence.0, 0);
        assert!(evidence.1 > 2);
        assert_eq!(evidence.2.len(), 64);
        assert!(!evidence.3.is_empty());
    }

    #[test]
    fn learned_preference_cannot_revive_an_excluded_candidate() {
        let db = routing_db();
        db.execute(
            "UPDATE routing_policies SET weights=?1 WHERE version=1",
            params![
                json!({"preferredCandidates":{"implementation":"claude:claude-standard"}})
                    .to_string()
            ],
        )
        .unwrap();
        save_preferences(
            &db,
            "w",
            &RouterPreferences {
                excluded_harnesses: vec!["claude".into()],
                ..RouterPreferences::default()
            },
        )
        .unwrap();
        let routed = route(&db, "parent", "turn", &request(), &descriptors()).unwrap();
        assert_eq!(
            routed.decision.recommended_candidate.as_deref(),
            Some("codex:codex-standard")
        );
        let claude = routed
            .decision
            .candidates
            .iter()
            .find(|candidate| candidate.candidate.harness == "claude")
            .unwrap();
        assert!(claude
            .exclusions
            .contains(&CandidateExclusion::UserExcludedHarness));
    }

    #[test]
    fn canary_assignment_is_deterministic_and_bounded_to_twenty_percent() {
        assert_eq!(canary_bucket("same-task"), canary_bucket("same-task"));
        let assigned = (0..10_000)
            .filter(|index| canary_bucket(&format!("task-{index}")) < 20)
            .count();
        assert!(
            (1_800..=2_200).contains(&assigned),
            "unexpected canary sample {assigned}"
        );
    }

    #[test]
    fn active_completion_gate_forces_a_different_verifier_family_even_in_shadow_mode() {
        let db = routing_db();
        db.execute("INSERT INTO completion_contracts(id,workspace_id,session_id,schema_version,acceptance_criteria,markdown_committed,status,created_at,updated_at) VALUES('c','w','parent',1,'[]',0,'active','now','now')", []).unwrap();
        db.execute("INSERT INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at) VALUES('p','c',1,'high','{}','now')", []).unwrap();
        db.execute("INSERT INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,repository_path,status,implementer_family,started_at) VALUES('a','p','parent','head','dirty','/tmp','verifying','codex','now')", []).unwrap();
        let mut verification = request();
        verification.role = WorkerRole::Verification;
        let routed = route(&db, "parent", "turn", &verification, &descriptors()).unwrap();
        assert_eq!(routed.request.runtime_harness(), "claude");
        assert_eq!(
            routed.decision.executed_candidate.as_deref(),
            Some("claude:claude-standard")
        );
        let codex = routed
            .decision
            .candidates
            .iter()
            .find(|candidate| candidate.candidate.harness == "codex")
            .unwrap();
        assert!(codex
            .exclusions
            .contains(&CandidateExclusion::SameAsImplementer));
        let mut pinned = verification;
        pinned.harness = Some("claude".into());
        pinned.model = Some("claude-standard".into());
        let routed = route(&db, "parent", "turn-2", &pinned, &descriptors()).unwrap();
        assert_eq!(
            routed.decision.executed_candidate.as_deref(),
            Some("claude:claude-standard")
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
                last_activity_at: None,
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
                uncached_input_tokens: None,
                context_percent: None,
                capability_units: 0,
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
                source: "policy.spawn.standard".into(),
                created_at: "now".into(),
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
                input_tokens: Some(100),
                output_tokens: Some(20),
                cache_read_tokens: None,
                cache_write_tokens: None,
                uncached_input_tokens: Some(100),
                context_percent: None,
                capability_units: 3,
                runtime_ms: Some(90),
                cost_microusd: Some(12_345),
                cost_source: Some("provider_reported".into()),
                stable_prefix_id: None,
                stable_prefix_hash: None,
                prompt_schema_version: None,
                prefix_token_estimate: None,
                harness: Some("codex".into()),
                model: Some("codex-standard".into()),
                role: Some("implementation".into()),
                task_family: Some("implementation".into()),
                restoration_mode: Some("fresh".into()),
                cross_harness_reuse: Some("not_applicable".into()),
                source: "usage.updated".into(),
                created_at: "later".into(),
            },
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_completion_inputs(child_session_id,request,updated_at) VALUES('child',?1,'now')",
            params![serde_json::to_string(&request()).unwrap()],
        ).unwrap();
        record_actual_execution(
            &db,
            &routed.decision.id,
            "codex",
            "codex-standard",
            Effort::Medium,
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
        let outcome: (bool, i64, i64, Option<i64>, Option<i64>, String, String) = db
            .query_row(
                "SELECT succeeded,normalized_cost,retry_count,cost_microusd,total_tokens,success_state,acceptance_state FROM router_outcomes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
            )
            .unwrap();
        assert_eq!(
            outcome,
            (
                true,
                3_000,
                1,
                Some(12_345),
                Some(120),
                "success".into(),
                "unknown".into()
            )
        );
        let evaluation: (String, i64, String) = db.query_row(
            "SELECT evaluator_kind,confidence_bps,bounded_metrics FROM routing_evaluations WHERE decision_id=?1",
            params![routed.decision.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(evaluation.0, "deterministic");
        assert_eq!(evaluation.1, 7_000);
        assert!(!evaluation.2.contains("done"));
        assert_eq!(
            db.query_row(
                "SELECT actual_model FROM router_decisions LIMIT 1",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "codex-standard"
        );
        assert_eq!(
            load_histories(&db, "w", "implementation").unwrap()["codex:codex-standard"].samples,
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
        let fixture: Vec<HeldOutObservation> = serde_json::from_str(include_str!(
            "../../../testing/fixtures/router-benchmark-v1.json"
        ))
        .unwrap();
        assert_eq!(fixture.len(), 20);
        assert!(fixture.iter().all(|row| !row.id.is_empty()));
        assert!(fixture.iter().all(|row| row.recommended));
        assert!(fixture.iter().filter(|row| row.manual).count() * 20 < fixture.len());
        assert!(fixture.iter().all(|row| !row.policy_violation));
        assert!(fixture.iter().filter(|row| row.actual_passed).count() >= 18);
        assert!(fixture
            .iter()
            .all(|row| row.normalized_cost > 0 && row.latency_ms > 0));
    }

    /// A harness that cannot start read-only must be excluded before a worker
    /// session exists, not discovered when its adapter refuses to launch.
    #[test]
    fn sandbox_incompatible_harness_is_excluded_with_a_permission_ceiling() {
        let mut descriptors = descriptors();
        descriptors[1].sandbox_modes =
            vec![SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];
        let read_only = request();
        assert_eq!(read_only.write_mode, WriteMode::ReadOnly);
        let candidates = build_candidates(&descriptors, &read_only, &BTreeMap::new());
        let claude = candidates
            .iter()
            .find(|candidate| candidate.harness == "claude")
            .unwrap();
        let codex = candidates
            .iter()
            .find(|candidate| candidate.harness == "codex")
            .unwrap();
        assert!(!claude.permission_eligible);
        assert!(codex.permission_eligible);

        let mut writing = read_only.clone();
        writing.write_mode = WriteMode::Isolated;
        writing.owned_paths = vec!["src/**".into()];
        assert!(build_candidates(&descriptors, &writing, &BTreeMap::new())
            .iter()
            .all(|candidate| candidate.permission_eligible));

        let evaluated = evaluate(EvaluationInput {
            candidates,
            preferences: RouterPreferences::default(),
            histories: BTreeMap::new(),
            required_capabilities: vec!["tools".into(), "commands".into()],
            remaining_capability_units: 24,
            budget_preference: None,
            latency_preference: None,
        });
        let claude = evaluated
            .iter()
            .find(|item| item.candidate.harness == "claude")
            .unwrap();
        assert!(claude
            .exclusions
            .contains(&CandidateExclusion::PermissionCeiling));
        assert!(evaluated
            .iter()
            .find(|item| item.candidate.harness == "codex")
            .unwrap()
            .eligible());
    }

    /// Autonomous routing must pick the compatible harness rather than the
    /// harness that would fail at startup.
    #[test]
    fn autonomous_routing_avoids_a_read_only_incompatible_harness() {
        let db = routing_db();
        let mut descriptors = descriptors();
        descriptors[0].sandbox_modes =
            vec![SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];
        force_autonomous(&db, "w");
        let routed = route(&db, "parent", "turn-sandbox", &request(), &descriptors).unwrap();
        assert_eq!(routed.request.harness.as_deref(), Some("claude"));
    }

    /// A manual pin at an incompatible harness must return an actionable
    /// incompatibility instead of reserving a doomed worker.
    #[test]
    fn pinned_incompatible_harness_returns_an_actionable_incompatibility() {
        let db = routing_db();
        let mut descriptors = descriptors();
        descriptors[0].sandbox_modes =
            vec![SandboxMode::WorkspaceWrite, SandboxMode::DangerFullAccess];
        let mut pinned = request();
        pinned.harness = Some("codex".into());
        pinned.model = Some("codex-standard".into());
        let error = route(&db, "parent", "turn-pinned", &pinned, &descriptors)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("codex cannot run a read_only worker"),
            "{error}"
        );
        assert!(error.contains("claude"), "{error}");
    }

    #[test]
    fn no_compatible_harness_explains_the_sandbox_mode() {
        let db = routing_db();
        let descriptors = descriptors()
            .into_iter()
            .map(|mut descriptor| {
                descriptor.sandbox_modes = vec![SandboxMode::WorkspaceWrite];
                descriptor
            })
            .collect::<Vec<_>>();
        force_autonomous(&db, "w");
        let error = route(&db, "parent", "turn-none", &request(), &descriptors)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("no installed harness can run a read_only worker"),
            "{error}"
        );
    }

    #[test]
    fn direct_chats_fail_closed_instead_of_joining_a_null_learning_scope() {
        let db = routing_db();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('direct',NULL,'codex','Direct','working','reported')", []).unwrap();
        let error = route(&db, "direct", "turn", &request(), &descriptors())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("without a workspace"),
            "{error}"
        );
    }

    #[test]
    fn histories_do_not_cross_workspaces() {
        let db = routing_db();
        db.execute(
            "INSERT INTO workspaces(id,title,status,created_at) VALUES('other','Other','idle','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('other-parent','other','codex','Other','working','reported')", []).unwrap();
        let routed = route(&db, "parent", "turn", &request(), &descriptors()).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES('child-w','w','codex','Worker','completed','reported','parent')", []).unwrap();
        db.execute(
            "INSERT INTO router_outcomes(decision_id,child_session_id,candidate,succeeded,status,runtime_ms,normalized_cost,retry_count,human_intervention,success_state,acceptance_state,recorded_at)
             VALUES(?1,'child-w','codex:codex-standard',1,'completed',90,1000,0,0,'success','accepted','now')",
            params![routed.decision.id],
        )
        .unwrap();
        assert_eq!(
            load_histories(&db, "w", "implementation").unwrap()["codex:codex-standard"].samples,
            1
        );
        assert!(load_histories(&db, "other", "implementation")
            .unwrap()
            .is_empty());
    }
}
