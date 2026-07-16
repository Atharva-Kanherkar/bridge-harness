//! Read-only replay and measurement for learning-router decisions.

use crate::learning_router::{
    CandidateEvaluation, CandidateExclusion, RouterDecision, RouterMode, RouterPreferences,
};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::Path};

const REPLAY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvalidDecision {
    id: String,
    error: String,
}

#[derive(Debug)]
struct RecordedDecision {
    decision: RouterDecision,
    outcome: Option<RecordedOutcome>,
}

#[derive(Debug)]
struct RecordedOutcome {
    succeeded: bool,
    runtime_ms: i64,
    normalized_cost: i64,
    retry_count: i64,
    human_intervention: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayReport {
    replay_schema_version: u32,
    source_decisions: usize,
    replayable_decisions: usize,
    invalid_decisions: Vec<InvalidDecision>,
    modes: BTreeMap<String, usize>,
    recorded_recommendations: usize,
    candidate_recommendations: usize,
    changed_recommendations: usize,
    shadow_alternatives: usize,
    manual_overrides: usize,
    no_route: usize,
    recommendation_coverage_bps: u16,
    completed_outcomes: usize,
    successful_outcomes: usize,
    success_bps: u16,
    average_runtime_ms: i64,
    average_normalized_cost: i64,
    retried_outcomes: usize,
    human_interventions: usize,
    policy_violations: usize,
    candidate_preferences: RouterPreferences,
    limitation: &'static str,
}

pub fn run_cli<I>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let program = args.next().unwrap_or_else(|| "router-replay".into());
    let Some(database) = args.next() else {
        return Err(usage(&program));
    };
    if database == "-h" || database == "--help" {
        println!("{}", usage(&program));
        return Ok(());
    }
    let mut workspace = None;
    let mut preferences = RouterPreferences::default();
    let remaining = args.collect::<Vec<_>>();
    let mut index = 0;
    while index < remaining.len() {
        let value = remaining.get(index + 1).ok_or_else(|| usage(&program))?;
        match remaining[index].as_str() {
            "--workspace" => workspace = Some(value.clone()),
            "--preferences" => {
                let body = fs::read_to_string(value)
                    .map_err(|error| format!("cannot read router preferences {value}: {error}"))?;
                preferences = serde_json::from_str(&body)
                    .map_err(|error| format!("invalid router preferences {value}: {error}"))?;
                preferences.validate()?;
            }
            _ => return Err(usage(&program)),
        }
        index += 2;
    }
    let (source_decisions, invalid_decisions, decisions) =
        load_database(Path::new(&database), workspace.as_deref())?;
    let report = replay(source_decisions, invalid_decisions, decisions, preferences);
    println!(
        "{}",
        serde_json::to_string_pretty(&report)
            .map_err(|error| format!("cannot serialize router replay: {error}"))?
    );
    Ok(())
}

fn usage(program: &str) -> String {
    format!(
        "usage: {program} <bridge.db> [--workspace <workspace-id>] [--preferences <router-preferences.json>]"
    )
}

fn load_database(
    path: &Path,
    workspace: Option<&str>,
) -> Result<(usize, Vec<InvalidDecision>, Vec<RecordedDecision>), String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open {} read-only: {error}", path.display()))?;
    let mut statement = db
        .prepare(
            "SELECT d.id,d.decision,o.succeeded,o.runtime_ms,o.normalized_cost,o.retry_count,o.human_intervention
             FROM router_decisions d LEFT JOIN router_outcomes o ON o.decision_id=d.id
             WHERE (?1 IS NULL OR d.workspace_id=?1) ORDER BY d.created_at,d.id",
        )
        .map_err(|error| format!("cannot read router decisions: {error}"))?;
    let rows = statement
        .query_map([workspace], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<bool>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<bool>>(6)?,
            ))
        })
        .map_err(|error| format!("cannot query router decisions: {error}"))?;
    let mut source = 0;
    let mut invalid = Vec::new();
    let mut decisions = Vec::new();
    for row in rows {
        source += 1;
        let (id, body, succeeded, runtime, cost, retries, human) =
            row.map_err(|error| format!("cannot read router decision row: {error}"))?;
        let decision: RouterDecision = match serde_json::from_str(&body) {
            Ok(decision) => decision,
            Err(error) => {
                invalid.push(InvalidDecision {
                    id,
                    error: error.to_string(),
                });
                continue;
            }
        };
        if decision.schema_version != crate::learning_router::ROUTER_SCHEMA_VERSION {
            invalid.push(InvalidDecision {
                id,
                error: format!(
                    "unsupported router decision schema {}",
                    decision.schema_version
                ),
            });
            continue;
        }
        let outcome = succeeded.map(|succeeded| RecordedOutcome {
            succeeded,
            runtime_ms: runtime.unwrap_or(0),
            normalized_cost: cost.unwrap_or(0),
            retry_count: retries.unwrap_or(0),
            human_intervention: human.unwrap_or(false),
        });
        decisions.push(RecordedDecision { decision, outcome });
    }
    Ok((source, invalid, decisions))
}

fn candidate_recommendation(
    decision: &RouterDecision,
    preferences: &RouterPreferences,
) -> Option<String> {
    decision
        .candidates
        .iter()
        .filter(|candidate| candidate_is_eligible(candidate, preferences))
        .min_by_key(|candidate| {
            (
                candidate.expected_cost_score,
                !candidate.candidate.default_for_tier,
                candidate.candidate.harness.as_str(),
                candidate.candidate.model.as_str(),
            )
        })
        .map(|candidate| candidate.candidate.key())
}

fn candidate_is_eligible(
    evaluation: &CandidateEvaluation,
    preferences: &RouterPreferences,
) -> bool {
    let deterministic_block = evaluation.exclusions.iter().any(|reason| {
        !matches!(
            reason,
            CandidateExclusion::UserExcludedHarness
                | CandidateExclusion::UserExcludedModel
                | CandidateExclusion::PinMismatch
                | CandidateExclusion::BelowQualityFloor
        )
    });
    !deterministic_block
        && evaluation.prediction.pass_probability_bps >= preferences.minimum_pass_bps
        && !preferences
            .excluded_harnesses
            .iter()
            .any(|value| evaluation.candidate.harness.eq_ignore_ascii_case(value))
        && !preferences
            .excluded_models
            .iter()
            .any(|value| evaluation.candidate.model.eq_ignore_ascii_case(value))
        && preferences
            .pinned_harness
            .as_ref()
            .is_none_or(|value| evaluation.candidate.harness.eq_ignore_ascii_case(value))
        && preferences
            .pinned_model
            .as_ref()
            .is_none_or(|value| evaluation.candidate.model.eq_ignore_ascii_case(value))
}

fn replay(
    source_decisions: usize,
    invalid_decisions: Vec<InvalidDecision>,
    decisions: Vec<RecordedDecision>,
    preferences: RouterPreferences,
) -> ReplayReport {
    let mut modes = BTreeMap::new();
    let mut recorded_recommendations = 0;
    let mut candidate_recommendations = 0;
    let mut changed_recommendations = 0;
    let mut shadow_alternatives = 0;
    let mut manual_overrides = 0;
    let mut no_route = 0;
    let mut completed_outcomes = 0;
    let mut successful_outcomes = 0;
    let mut runtime_total = 0;
    let mut cost_total = 0;
    let mut retried_outcomes = 0;
    let mut human_interventions = 0;
    let mut policy_violations = 0;
    for record in &decisions {
        let mode = match record.decision.mode {
            RouterMode::Disabled => "disabled",
            RouterMode::Shadow => "shadow",
            RouterMode::Autonomous => "autonomous",
        };
        *modes.entry(mode.into()).or_insert(0) += 1;
        let candidate = candidate_recommendation(&record.decision, &preferences);
        recorded_recommendations += usize::from(record.decision.recommended_candidate.is_some());
        candidate_recommendations += usize::from(candidate.is_some());
        changed_recommendations += usize::from(candidate != record.decision.recommended_candidate);
        no_route += usize::from(candidate.is_none());
        manual_overrides += usize::from(record.decision.manual_override);
        shadow_alternatives += usize::from(
            record.decision.mode == RouterMode::Shadow
                && record.decision.recommended_candidate.is_some()
                && record.decision.recommended_candidate != record.decision.baseline_candidate,
        );
        if let Some(key) = &candidate {
            policy_violations += usize::from(
                record
                    .decision
                    .candidates
                    .iter()
                    .find(|item| item.candidate.key() == *key)
                    .is_some_and(|item| {
                        item.exclusions.iter().any(|reason| {
                            !matches!(
                                reason,
                                CandidateExclusion::UserExcludedHarness
                                    | CandidateExclusion::UserExcludedModel
                                    | CandidateExclusion::PinMismatch
                                    | CandidateExclusion::BelowQualityFloor
                            )
                        })
                    }),
            );
        }
        if let Some(outcome) = &record.outcome {
            completed_outcomes += 1;
            successful_outcomes += usize::from(outcome.succeeded);
            runtime_total += outcome.runtime_ms;
            cost_total += outcome.normalized_cost;
            retried_outcomes += usize::from(outcome.retry_count > 0);
            human_interventions += usize::from(outcome.human_intervention);
        }
    }
    let ratio = |numerator: usize, denominator: usize| {
        if denominator == 0 {
            0
        } else {
            (numerator * 10_000 / denominator) as u16
        }
    };
    ReplayReport {
        replay_schema_version: REPLAY_SCHEMA_VERSION,
        source_decisions,
        replayable_decisions: decisions.len(),
        invalid_decisions,
        modes,
        recorded_recommendations,
        candidate_recommendations,
        changed_recommendations,
        shadow_alternatives,
        manual_overrides,
        no_route,
        recommendation_coverage_bps: ratio(candidate_recommendations, decisions.len()),
        completed_outcomes,
        successful_outcomes,
        success_bps: ratio(successful_outcomes, completed_outcomes),
        average_runtime_ms: if completed_outcomes == 0 {
            0
        } else {
            runtime_total / completed_outcomes as i64
        },
        average_normalized_cost: if completed_outcomes == 0 {
            0
        } else {
            cost_total / completed_outcomes as i64
        },
        retried_outcomes,
        human_interventions,
        policy_violations,
        candidate_preferences: preferences,
        limitation: "Replay uses the candidate inventory, deterministic exclusions, and predictions recorded at decision time. It does not call providers or claim causal model superiority.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::Effort,
        learning_router::{CandidatePrediction, RouteCandidate, ROUTER_SCHEMA_VERSION},
        model::CapabilityTier,
    };

    fn decision() -> RouterDecision {
        let candidate = CandidateEvaluation {
            candidate: RouteCandidate {
                harness: "codex".into(),
                model: "fast".into(),
                tier: CapabilityTier::Fast,
                effort: Effort::Low,
                sandbox: "read_only".into(),
                capabilities: vec!["tools".into()],
                available: true,
                platform_supported: true,
                permission_eligible: true,
                quota_available: true,
                context_available: true,
                risk_eligible: true,
                capability_units: 1,
                default_for_tier: true,
            },
            exclusions: vec![],
            prediction: CandidatePrediction {
                pass_probability_bps: 7_000,
                latency_ms: 1_000,
                normalized_quota_cost: 1_000,
                retry_risk_bps: 3_000,
                samples: 0,
            },
            expected_cost_score: 1,
        };
        RouterDecision {
            schema_version: ROUTER_SCHEMA_VERSION,
            id: "d".into(),
            workspace_id: "w".into(),
            parent_session_id: "p".into(),
            turn_id: "t".into(),
            task_family: "implementation".into(),
            mode: RouterMode::Shadow,
            manual_override: false,
            baseline_candidate: Some("claude:standard".into()),
            recommended_candidate: Some("codex:fast".into()),
            executed_candidate: Some("claude:standard".into()),
            explanation: "shadow".into(),
            candidates: vec![candidate],
            created_at: "now".into(),
        }
    }

    #[test]
    fn replay_is_deterministic_and_preferences_cannot_revive_policy_exclusions() {
        let mut blocked = decision();
        blocked.candidates[0].exclusions = vec![CandidateExclusion::BudgetCeiling];
        let report = replay(
            2,
            vec![],
            vec![
                RecordedDecision {
                    decision: decision(),
                    outcome: Some(RecordedOutcome {
                        succeeded: true,
                        runtime_ms: 10,
                        normalized_cost: 1_000,
                        retry_count: 0,
                        human_intervention: false,
                    }),
                },
                RecordedDecision {
                    decision: blocked,
                    outcome: None,
                },
            ],
            RouterPreferences::default(),
        );
        assert_eq!(report.candidate_recommendations, 1);
        assert_eq!(report.completed_outcomes, 1);
        assert_eq!(report.policy_violations, 0);
        assert_eq!(report.success_bps, 10_000);
    }
}
