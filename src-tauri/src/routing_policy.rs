//! Immutable learned routing policies and realized-outcome replay.
//!
//! This module consumes only typed routing decisions, normalized metrics, and
//! evidence identifiers. It never reads prompt or transcript text and it never
//! widens the deterministic eligibility decisions recorded by the router.

use crate::{learning_router::RouterDecision, BridgeError};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub const MIN_EVIDENCE_SAMPLES: i64 = 5;
const MIN_GROUP_SAMPLES: i64 = 2;
const MIN_CONFIDENCE_BPS: i64 = 6_500;
const MIN_REPLAY_COVERAGE_BPS: i64 = 8_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetrics {
    pub samples: i64,
    pub quality_bps: Option<i64>,
    pub cost_per_success_microusd: Option<i64>,
    pub average_latency_ms: Option<i64>,
    pub retry_rate_bps: Option<i64>,
    pub intervention_rate_bps: Option<i64>,
    pub average_confidence_bps: Option<i64>,
    pub cost_complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyReplayReport {
    pub baseline: ReplayMetrics,
    pub candidate: ReplayMetrics,
    pub held_out_samples: i64,
    pub covered_samples: i64,
    pub coverage_bps: i64,
    pub unavailable_selections: i64,
    pub quality_guard_passed: bool,
    pub cost_guard_passed: bool,
    pub latency_guard_passed: bool,
    pub retry_guard_passed: bool,
    pub intervention_guard_passed: bool,
    pub passed: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CandidatePolicy {
    pub weights: serde_json::Value,
    pub thresholds: serde_json::Value,
    pub replay: PolicyReplayReport,
    pub evidence_groups: i64,
}

#[derive(Debug, Clone)]
struct EvidenceRow {
    rowid: i64,
    fingerprint: String,
    task_family: String,
    profile_key: String,
    candidate: String,
    effort: String,
    success: Option<bool>,
    cost_microusd: Option<i64>,
    latency_ms: Option<i64>,
    retried: bool,
    intervention: bool,
    confidence_bps: Option<i64>,
    decision: RouterDecision,
}

#[derive(Debug, Clone, Default)]
struct Aggregate {
    samples: i64,
    known_outcomes: i64,
    successes: i64,
    cost_reported: i64,
    cost_total: i64,
    latency_reported: i64,
    latency_total: i64,
    retries: i64,
    interventions: i64,
    confidence_reported: i64,
    confidence_total: i64,
}

#[derive(Debug, Clone, Default)]
struct ProjectedAggregate {
    samples: i64,
    quality_total_bps: i64,
    cost_total_microusd: i64,
    expected_success_total_bps: i64,
    cost_reported: i64,
    latency_total_ms: i64,
    latency_reported: i64,
    retry_total_bps: i64,
    intervention_total_bps: i64,
    confidence_total_bps: i64,
    confidence_reported: i64,
}

impl ProjectedAggregate {
    fn add(&mut self, aggregate: &Aggregate) {
        self.samples += 1;
        let quality = aggregate
            .quality_bps()
            .expect("eligible learning aggregate has a known quality rate");
        self.quality_total_bps += quality;
        self.expected_success_total_bps += quality;
        if aggregate.cost_reported == aggregate.samples {
            self.cost_reported += 1;
            self.cost_total_microusd += aggregate.cost_total * 10_000 / aggregate.samples;
        }
        if let Some(latency) = aggregate.latency_ms() {
            self.latency_reported += 1;
            self.latency_total_ms += latency;
        }
        self.retry_total_bps += aggregate.retries * 10_000 / aggregate.samples;
        self.intervention_total_bps += aggregate.interventions * 10_000 / aggregate.samples;
        if let Some(confidence) = aggregate.confidence_bps() {
            self.confidence_reported += 1;
            self.confidence_total_bps += confidence;
        }
    }

    fn metrics(&self) -> ReplayMetrics {
        ReplayMetrics {
            samples: self.samples,
            quality_bps: (self.samples > 0)
                .then(|| self.quality_total_bps / self.samples),
            cost_per_success_microusd: (self.samples > 0
                && self.cost_reported == self.samples
                && self.expected_success_total_bps > 0)
                .then(|| self.cost_total_microusd / self.expected_success_total_bps),
            average_latency_ms: (self.latency_reported > 0)
                .then(|| self.latency_total_ms / self.latency_reported),
            retry_rate_bps: (self.samples > 0)
                .then(|| self.retry_total_bps / self.samples),
            intervention_rate_bps: (self.samples > 0)
                .then(|| self.intervention_total_bps / self.samples),
            average_confidence_bps: (self.confidence_reported > 0)
                .then(|| self.confidence_total_bps / self.confidence_reported),
            cost_complete: self.samples > 0 && self.cost_reported == self.samples,
        }
    }
}

impl Aggregate {
    fn add(&mut self, row: &EvidenceRow) {
        self.samples += 1;
        if let Some(success) = row.success {
            self.known_outcomes += 1;
            self.successes += i64::from(success);
        }
        if let Some(cost) = row.cost_microusd {
            self.cost_reported += 1;
            self.cost_total += cost;
        }
        if let Some(latency) = row.latency_ms {
            self.latency_reported += 1;
            self.latency_total += latency;
        }
        self.retries += i64::from(row.retried);
        self.interventions += i64::from(row.intervention);
        if let Some(confidence) = row.confidence_bps {
            self.confidence_reported += 1;
            self.confidence_total += confidence;
        }
    }

    fn quality_bps(&self) -> Option<i64> {
        (self.known_outcomes > 0).then(|| self.successes * 10_000 / self.known_outcomes)
    }

    fn cost_per_success(&self) -> Option<i64> {
        (self.samples > 0 && self.cost_reported == self.samples && self.successes > 0)
            .then(|| self.cost_total / self.successes)
    }

    fn latency_ms(&self) -> Option<i64> {
        (self.latency_reported > 0).then(|| self.latency_total / self.latency_reported)
    }

    fn confidence_bps(&self) -> Option<i64> {
        (self.confidence_reported > 0)
            .then(|| self.confidence_total / self.confidence_reported)
    }

    fn eligible_for_learning(&self) -> bool {
        self.samples >= MIN_GROUP_SAMPLES
            && self.known_outcomes == self.samples
            && self.confidence_bps().is_some_and(|value| value >= MIN_CONFIDENCE_BPS)
    }

    fn metrics(&self) -> ReplayMetrics {
        ReplayMetrics {
            samples: self.samples,
            quality_bps: self.quality_bps(),
            cost_per_success_microusd: self.cost_per_success(),
            average_latency_ms: self.latency_ms(),
            retry_rate_bps: (self.samples > 0).then(|| self.retries * 10_000 / self.samples),
            intervention_rate_bps: (self.samples > 0)
                .then(|| self.interventions * 10_000 / self.samples),
            average_confidence_bps: self.confidence_bps(),
            cost_complete: self.samples > 0 && self.cost_reported == self.samples,
        }
    }
}

fn load_evidence(db: &Connection, boundary: i64) -> Result<Vec<EvidenceRow>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT o.rowid,d.task_fingerprint,d.task_family,COALESCE(d.profile_version,0),COALESCE(d.profile_purpose,''),
                o.candidate,COALESCE(d.actual_effort,''),o.success_state,
                (SELECT e.score_bps FROM routing_evaluations e WHERE e.decision_id=d.id AND e.evaluator_kind='model_based' AND e.status='completed' ORDER BY e.created_at DESC LIMIT 1),
                o.cost_microusd,o.runtime_ms,o.retry_count,o.human_intervention,
                COALESCE((SELECT e.confidence_bps FROM routing_evaluations e WHERE e.decision_id=d.id AND e.evaluator_kind='model_based' AND e.status='completed' ORDER BY e.created_at DESC LIMIT 1),o.confidence_bps),
                d.decision
         FROM router_outcomes o JOIN router_decisions d ON d.id=o.decision_id
         WHERE o.rowid<=?1 ORDER BY o.rowid",
    )?;
    let rows = statement.query_map(params![boundary], |row| {
        let decision_body = row.get::<_, String>(14)?;
        let decision = serde_json::from_str(&decision_body).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
        let model_score = row.get::<_, Option<i64>>(8)?;
        let success = match row.get::<_, String>(7)?.as_str() {
            "success" => Some(true),
            "failure" => Some(false),
            _ => model_score.map(|score| score >= 5_000),
        };
        Ok(EvidenceRow {
            rowid: row.get(0)?,
            fingerprint: row.get(1)?,
            task_family: row.get(2)?,
            profile_key: format!("{}:{}", row.get::<_, i64>(3)?, row.get::<_, String>(4)?),
            candidate: row.get(5)?,
            effort: row.get(6)?,
            success,
            cost_microusd: row.get(9)?,
            latency_ms: row.get(10)?,
            retried: row.get::<_, i64>(11)? > 0,
            intervention: row.get(12)?,
            confidence_bps: row.get(13)?,
            decision,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(BridgeError::from)
}

fn aggregate_key(row: &EvidenceRow) -> String {
    format!(
        "{}|{}|{}|{}",
        row.fingerprint, row.profile_key, row.candidate, row.effort
    )
}

fn learning_context_key(row: &EvidenceRow) -> String {
    format!("{}|{}|{}", row.fingerprint, row.profile_key, row.effort)
}

fn rank_key(candidate: &str, aggregate: &Aggregate) -> (i64, i64, i64, i64, String) {
    (
        -aggregate.quality_bps().unwrap_or_default(),
        aggregate.cost_per_success().unwrap_or(i64::MAX / 4),
        aggregate.latency_ms().unwrap_or(i64::MAX / 4),
        aggregate.retries * 10_000 / aggregate.samples.max(1),
        candidate.to_owned(),
    )
}

fn candidate_is_eligible(row: &EvidenceRow, candidate: &str) -> bool {
    row.decision
        .candidates
        .iter()
        .find(|evaluation| evaluation.candidate.key() == candidate)
        .is_some_and(|evaluation| evaluation.exclusions.is_empty())
}

fn metric_guard(
    candidate: Option<i64>,
    baseline: Option<i64>,
    multiplier_bps: i64,
) -> bool {
    match (candidate, baseline) {
        (Some(candidate), Some(baseline)) => candidate * 10_000 <= baseline * multiplier_bps,
        (None, None) => true,
        _ => false,
    }
}

pub fn build_candidate(
    db: &Connection,
    boundary: i64,
    base_weights: &serde_json::Value,
) -> Result<Option<CandidatePolicy>, BridgeError> {
    let evidence = load_evidence(db, boundary)?;
    if evidence.len() < MIN_EVIDENCE_SAMPLES as usize {
        return Ok(None);
    }
    let (mut training, mut held_out): (Vec<_>, Vec<_>) = evidence
        .iter()
        .partition(|row| row.rowid % 5 != 0);
    if held_out.is_empty() {
        held_out.push(training.pop().expect("evidence is non-empty"));
    }

    let mut aggregate_by_exact = BTreeMap::<String, Aggregate>::new();
    let mut candidates_by_context = BTreeMap::<String, BTreeSet<String>>::new();
    let mut rows_by_context = BTreeMap::<String, Vec<&EvidenceRow>>::new();
    for row in &training {
        aggregate_by_exact
            .entry(aggregate_key(row))
            .or_default()
            .add(row);
        candidates_by_context
            .entry(learning_context_key(row))
            .or_default()
            .insert(row.candidate.clone());
        rows_by_context
            .entry(learning_context_key(row))
            .or_default()
            .push(row);
    }

    let mut context_preferred = BTreeMap::<String, String>::new();
    let mut evidence_groups = 0_i64;
    for (context, candidates) in &candidates_by_context {
        if candidates.len() < 2 {
            continue;
        }
        let best = candidates
            .iter()
            .filter_map(|candidate| {
                let key = format!("{}|{}", context, candidate);
                let aggregate = aggregate_by_exact.get(&key)?;
                aggregate
                    .eligible_for_learning()
                    .then(|| (candidate, aggregate))
            })
            .min_by_key(|(candidate, aggregate)| rank_key(candidate, aggregate));
        if let Some((candidate, _)) = best {
            context_preferred.insert(context.clone(), candidate.clone());
            evidence_groups += 1;
        }
    }

    // The runtime policy keys by stable task fingerprint. Keep profile/model/effort
    // aggregation exact, and publish a fingerprint preference only when every
    // eligible profile context for that fingerprint agrees on the same candidate.
    let mut preferred = BTreeMap::<String, String>::new();
    let mut preferences_by_fingerprint = BTreeMap::<String, BTreeSet<String>>::new();
    for (context, candidate) in &context_preferred {
        let fingerprint = rows_by_context[context][0].fingerprint.clone();
        preferences_by_fingerprint
            .entry(fingerprint)
            .or_default()
            .insert(candidate.clone());
    }
    for (fingerprint, candidates) in preferences_by_fingerprint {
        if candidates.len() == 1 {
            preferred.insert(fingerprint, candidates.into_iter().next().unwrap());
        }
    }

    let mut family_contexts = BTreeMap::<String, Vec<&EvidenceRow>>::new();
    for row in &training {
        family_contexts
            .entry(format!("{}|{}|{}", row.task_family, row.profile_key, row.effort))
            .or_default()
            .push(row);
    }
    let mut preferences_by_family = BTreeMap::<String, BTreeSet<String>>::new();
    for rows in family_contexts.values() {
        let family = rows[0].task_family.clone();
        let mut aggregate_by_candidate = BTreeMap::<String, Aggregate>::new();
        for row in rows {
            aggregate_by_candidate
                .entry(row.candidate.clone())
                .or_default()
                .add(row);
        }
        if aggregate_by_candidate.len() < 2 {
            continue;
        }
        if let Some((candidate, _)) = aggregate_by_candidate
            .iter()
            .filter(|(_, aggregate)| aggregate.eligible_for_learning())
            .min_by_key(|(candidate, aggregate)| rank_key(candidate, aggregate))
        {
            preferences_by_family
                .entry(family)
                .or_default()
                .insert(candidate.clone());
            evidence_groups += 1;
        }
    }
    for (family, candidates) in preferences_by_family {
        if candidates.len() == 1 {
            preferred.insert(family, candidates.into_iter().next().unwrap());
        }
    }
    if preferred.is_empty() {
        return Ok(None);
    }

    let base_preferred = base_weights
        .get("preferredCandidates")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    if preferred.iter().all(|(key, value)| {
        base_preferred.get(key).and_then(serde_json::Value::as_str) == Some(value.as_str())
    }) {
        return Ok(None);
    }

    let mut baseline = Aggregate::default();
    let mut candidate = ProjectedAggregate::default();
    let mut covered = 0_i64;
    let mut unavailable = 0_i64;
    for row in &held_out {
        baseline.add(row);
        let selected = preferred
            .get(&row.fingerprint)
            .or_else(|| preferred.get(&row.task_family));
        let Some(selected) = selected else {
            continue;
        };
        if !candidate_is_eligible(row, selected) {
            unavailable += 1;
            continue;
        }
        let exact = format!(
            "{}|{}|{}|{}",
            row.fingerprint, row.profile_key, selected, row.effort
        );
        let stats = aggregate_by_exact.get(&exact).or_else(|| {
            // A task-family fallback remains bound to the same profile/model/effort
            // dimensions before it may influence replay.
            training
                .iter()
                .find(|candidate_row| {
                    candidate_row.task_family == row.task_family
                        && candidate_row.profile_key == row.profile_key
                        && candidate_row.candidate == *selected
                        && candidate_row.effort == row.effort
                })
                .and_then(|candidate_row| aggregate_by_exact.get(&aggregate_key(candidate_row)))
        });
        let Some(stats) = stats.filter(|stats| stats.eligible_for_learning()) else {
            continue;
        };
        // Replay the rates observed in the training partition without converting
        // them into synthetic pass/fail or retry outcomes. The result remains an
        // observational estimate, not a claim that an unexecuted model succeeded.
        candidate.add(stats);
        covered += 1;
    }

    let baseline_metrics = baseline.metrics();
    let candidate_metrics = candidate.metrics();
    let coverage_bps = covered * 10_000 / held_out.len().max(1) as i64;
    let quality_guard = candidate_metrics
        .quality_bps
        .zip(baseline_metrics.quality_bps)
        .is_some_and(|(candidate, baseline)| candidate >= baseline);
    let cost_guard = if baseline_metrics.cost_complete
        && baseline_metrics.quality_bps == Some(0)
        && candidate_metrics.cost_per_success_microusd.is_some()
    {
        true
    } else {
        metric_guard(
            candidate_metrics.cost_per_success_microusd,
            baseline_metrics.cost_per_success_microusd,
            11_000,
        )
    };
    let latency_guard = metric_guard(
        candidate_metrics.average_latency_ms,
        baseline_metrics.average_latency_ms,
        12_000,
    );
    let retry_guard = candidate_metrics
        .retry_rate_bps
        .zip(baseline_metrics.retry_rate_bps)
        .is_some_and(|(candidate, baseline)| candidate <= baseline + 500);
    let intervention_guard = candidate_metrics
        .intervention_rate_bps
        .zip(baseline_metrics.intervention_rate_bps)
        .is_some_and(|(candidate, baseline)| candidate <= baseline + 500);
    let mut reasons = Vec::new();
    for (passed, reason) in [
        (coverage_bps >= MIN_REPLAY_COVERAGE_BPS, "held-out replay coverage is below 80%"),
        (unavailable == 0, "candidate selected an unavailable or excluded model"),
        (quality_guard, "candidate quality regressed"),
        (cost_guard, "candidate cost per successful task regressed or is unknown"),
        (latency_guard, "candidate latency regressed or is unknown"),
        (retry_guard, "candidate retry rate regressed"),
        (intervention_guard, "candidate intervention rate regressed"),
    ] {
        if !passed {
            reasons.push(reason.into());
        }
    }
    let passed = reasons.is_empty();
    let replay = PolicyReplayReport {
        baseline: baseline_metrics,
        candidate: candidate_metrics,
        held_out_samples: held_out.len() as i64,
        covered_samples: covered,
        coverage_bps,
        unavailable_selections: unavailable,
        quality_guard_passed: quality_guard,
        cost_guard_passed: cost_guard,
        latency_guard_passed: latency_guard,
        retry_guard_passed: retry_guard,
        intervention_guard_passed: intervention_guard,
        passed,
        reasons,
    };
    Ok(Some(CandidatePolicy {
        weights: json!({
            "schemaVersion": 1,
            "preferredCandidates": preferred,
            "rankingScope": "eligible_candidates_only",
        }),
        thresholds: json!({
            "minimumSamples": MIN_EVIDENCE_SAMPLES,
            "minimumGroupSamples": MIN_GROUP_SAMPLES,
            "minimumConfidenceBps": MIN_CONFIDENCE_BPS,
            "minimumReplayCoverageBps": MIN_REPLAY_COVERAGE_BPS,
        }),
        replay,
        evidence_groups,
    }))
}
