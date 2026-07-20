//! Read-only replay of persisted policy decisions.
//!
//! This module deliberately has no adapter or provider dependency. It turns the
//! append-only decision log into a deterministic regression and sensitivity
//! harness for policy configuration changes.

use crate::policy::{PolicyConfig, PolicyEngine, PolicyInput, PolicyOutcome, RouteDecision};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};

const REPLAY_SCHEMA_VERSION: u64 = 1;

#[derive(Debug)]
struct ReplayCase {
    input: PolicyInput,
    recorded: PolicyOutcome,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvalidRecord {
    entry_id: String,
    error: String,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct OutcomeSummary {
    routes: BTreeMap<String, usize>,
    capability_units_assessed: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReplayReport {
    replay_schema_version: u64,
    source_decisions: usize,
    replayable_decisions: usize,
    legacy_skipped: usize,
    invalid_records: Vec<InvalidRecord>,
    candidate: PolicyConfig,
    exact_matches: usize,
    changed: usize,
    transitions: BTreeMap<String, usize>,
    recorded: OutcomeSummary,
    candidate_summary: OutcomeSummary,
    capability_units_delta: i64,
    limitation: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FullReplayReport {
    deterministic_safety_replay: ReplayReport,
    realized_outcome_replay: Option<crate::routing_policy::CandidatePolicy>,
}

struct LoadedCases {
    source_decisions: usize,
    legacy_skipped: usize,
    invalid_records: Vec<InvalidRecord>,
    cases: Vec<ReplayCase>,
}

/// CLI boundary kept public so the standalone binary does not expose internal
/// policy model types as library API.
pub fn run_cli<I>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let program = args.next().unwrap_or_else(|| "policy-replay".into());
    let Some(database) = args.next() else {
        return Err(usage(&program));
    };
    if database == "-h" || database == "--help" {
        println!("{}", usage(&program));
        return Ok(());
    }

    let mut candidate = PolicyConfig::default();
    let remaining = args.collect::<Vec<_>>();
    match remaining.as_slice() {
        [] => {}
        [flag, path] if flag == "--candidate" => {
            let body = fs::read_to_string(path)
                .map_err(|error| format!("cannot read candidate config {path}: {error}"))?;
            candidate = serde_json::from_str(&body)
                .map_err(|error| format!("invalid candidate config {path}: {error}"))?;
        }
        _ => return Err(usage(&program)),
    }

    let loaded = load_database(Path::new(&database))?;
    let report = replay(loaded, candidate);
    let realized_outcome_replay = load_realized_outcome_replay(Path::new(&database))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&FullReplayReport {
            deterministic_safety_replay: report,
            realized_outcome_replay,
        })
        .map_err(|error| format!("cannot serialize replay report: {error}"))?
    );
    Ok(())
}

fn load_realized_outcome_replay(
    path: &Path,
) -> Result<Option<crate::routing_policy::CandidatePolicy>, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open {} for outcome replay: {error}", path.display()))?;
    let boundary: i64 = db
        .query_row(
            "SELECT COALESCE(MAX(rowid),0) FROM router_outcomes",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("cannot freeze realized-outcome boundary: {error}"))?;
    let weights: String = db
        .query_row(
            "SELECT weights FROM routing_policies WHERE status IN ('active','canary') LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("cannot load active learned policy: {error}"))?;
    let weights = serde_json::from_str(&weights)
        .map_err(|error| format!("active learned policy weights are invalid: {error}"))?;
    crate::routing_policy::build_candidate(&db, boundary, &weights)
        .map_err(|error| format!("realized-outcome replay failed: {error}"))
}

fn usage(program: &str) -> String {
    format!("usage: {program} <bridge.db> [--candidate <policy-config.json>]")
}

fn load_database(path: &Path) -> Result<LoadedCases, String> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open {} read-only: {error}", path.display()))?;
    let mut statement = db
        .prepare(
            "SELECT id,payload FROM session_entries
             WHERE kind IN ('delegation.approved','approval.requested',
                            'delegation.requested','delegation.rejected')
             ORDER BY session_id,sequence",
        )
        .map_err(|error| format!("cannot read policy decisions: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("cannot query policy decisions: {error}"))?;

    let mut payloads = Vec::new();
    for row in rows {
        payloads.push(row.map_err(|error| format!("cannot read policy decision row: {error}"))?);
    }
    Ok(parse_payloads(payloads))
}

fn parse_payloads(payloads: Vec<(String, String)>) -> LoadedCases {
    let mut loaded = LoadedCases {
        source_decisions: 0,
        legacy_skipped: 0,
        invalid_records: Vec::new(),
        cases: Vec::new(),
    };
    for (entry_id, body) in payloads {
        let payload: Value = match serde_json::from_str(&body) {
            Ok(value) => value,
            Err(error) => {
                loaded.invalid_records.push(InvalidRecord {
                    entry_id,
                    error: format!("invalid payload JSON: {error}"),
                });
                continue;
            }
        };
        if payload.get("decision").is_none() || payload.get("reason").is_none() {
            continue;
        }
        loaded.source_decisions += 1;
        let Some(version) = payload.get("replaySchemaVersion") else {
            loaded.legacy_skipped += 1;
            continue;
        };
        if version.as_u64() != Some(REPLAY_SCHEMA_VERSION) {
            loaded.invalid_records.push(InvalidRecord {
                entry_id,
                error: format!("unsupported replay schema version {version}"),
            });
            continue;
        }
        match parse_case(&payload) {
            Ok(case) => loaded.cases.push(case),
            Err(error) => loaded
                .invalid_records
                .push(InvalidRecord { entry_id, error }),
        }
    }
    loaded
}

fn parse_case(payload: &Value) -> Result<ReplayCase, String> {
    let input = serde_json::from_value(
        payload
            .get("replayInput")
            .cloned()
            .ok_or_else(|| "missing replayInput".to_string())?,
    )
    .map_err(|error| format!("invalid replayInput: {error}"))?;
    let recorded = serde_json::from_value(serde_json::json!({
        "decision": payload.get("decision"),
        "reason": payload.get("reason"),
        "capabilityUnits": payload.get("capabilityUnits"),
    }))
    .map_err(|error| format!("invalid recorded outcome: {error}"))?;
    Ok(ReplayCase { input, recorded })
}

fn replay(loaded: LoadedCases, candidate: PolicyConfig) -> ReplayReport {
    let mut exact_matches = 0;
    let mut transitions = BTreeMap::new();
    let mut recorded = OutcomeSummary::default();
    let mut candidate_summary = OutcomeSummary::default();
    let engine = PolicyEngine::new(candidate.clone());

    for case in &loaded.cases {
        let evaluated = engine.decide(&case.input);
        summarize(&mut recorded, &case.recorded);
        summarize(&mut candidate_summary, &evaluated);
        if evaluated == case.recorded {
            exact_matches += 1;
        } else {
            let transition = format!(
                "{}/{} -> {}/{}",
                route_name(&case.recorded.decision),
                serde_json::to_value(case.recorded.reason)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".into()),
                route_name(&evaluated.decision),
                serde_json::to_value(evaluated.reason)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_else(|| "unknown".into()),
            );
            *transitions.entry(transition).or_insert(0) += 1;
        }
    }
    let capability_units_delta =
        candidate_summary.capability_units_assessed - recorded.capability_units_assessed;
    ReplayReport {
        replay_schema_version: REPLAY_SCHEMA_VERSION,
        source_decisions: loaded.source_decisions,
        replayable_decisions: loaded.cases.len(),
        legacy_skipped: loaded.legacy_skipped,
        invalid_records: loaded.invalid_records,
        candidate,
        exact_matches,
        changed: loaded.cases.len() - exact_matches,
        transitions,
        recorded,
        candidate_summary,
        capability_units_delta,
        limitation: "This section replays deterministic safety structure. The sibling realizedOutcomeReplay section uses typed held-out outcomes and reported provider costs without claiming causal model superiority.",
    }
}

fn summarize(summary: &mut OutcomeSummary, outcome: &PolicyOutcome) {
    *summary
        .routes
        .entry(route_name(&outcome.decision).into())
        .or_insert(0) += 1;
    summary.capability_units_assessed += outcome.capability_units;
}

fn route_name(decision: &RouteDecision) -> &'static str {
    match decision {
        RouteDecision::ExecuteInParent => "execute_in_parent",
        RouteDecision::ResumeWorker { .. } => "resume_worker",
        RouteDecision::SpawnWorker(_) => "spawn_worker",
        RouteDecision::Queue => "queue",
        RouteDecision::Reject => "reject",
        RouteDecision::RequireUserApproval => "require_user_approval",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        delegation::{
            CapabilityTier, DelegationRequest, Effort, OutputContract, WorkerRole, WriteMode,
            SCHEMA_VERSION,
        },
        policy::{record_decision, OwnedPathProvenance, RequestBudget},
        store,
    };
    use tempfile::NamedTempFile;

    #[derive(serde::Deserialize)]
    struct FixtureRow {
        id: String,
        payload: Value,
    }

    fn input(parent_depth: i64) -> PolicyInput {
        PolicyInput {
            workspace_id: "workspace".into(),
            worktree_id: "workspace".into(),
            parent_session_id: "parent".into(),
            turn_id: "turn-1".into(),
            parent_depth,
            request: DelegationRequest {
                schema_version: SCHEMA_VERSION,
                role: WorkerRole::Verification,
                objective: "Verify replay".into(),
                acceptance_criteria: vec!["Report exact result".into()],
                known_facts: vec![],
                decisions: vec![],
                evidence_ids: vec![],
                relevant_files: vec![],
                owned_paths: vec![],
                write_mode: WriteMode::ReadOnly,
                capability_tier: CapabilityTier::Fast,
                effort: Effort::Low,
                network_access: false,
                writable_output_paths: vec![],
                verification: vec![],
                output_contract: OutputContract::VerificationResult,
                harness: None,
                model: None,
            },
            owned_path_provenance: OwnedPathProvenance::default(),
            requested_harness: "codex".into(),
            task_family: "verification".into(),
            active_workers: vec![],
            warm_workers: vec![],
            budget: RequestBudget::default(),
            retry_count: 0,
            parent_can_execute: false,
            requires_user_approval: false,
            child_worktrees_available: true,
        }
    }

    fn payload(input: &PolicyInput) -> String {
        let outcome = PolicyEngine::default().decide(input);
        serde_json::json!({
            "decision": outcome.decision,
            "reason": outcome.reason,
            "capabilityUnits": outcome.capability_units,
            "replaySchemaVersion": REPLAY_SCHEMA_VERSION,
            "replayInput": input,
        })
        .to_string()
    }

    #[test]
    fn current_defaults_replay_exactly_and_candidate_changes_are_explicit() {
        let loaded = parse_payloads(vec![("entry-1".into(), payload(&input(0)))]);
        let report = replay(loaded, PolicyConfig::default());
        assert_eq!(report.exact_matches, 1);
        assert_eq!(report.changed, 0);

        let loaded = parse_payloads(vec![("entry-1".into(), payload(&input(0)))]);
        let mut candidate = PolicyConfig::default();
        candidate.max_depth = 0;
        let report = replay(loaded, candidate);
        assert_eq!(report.exact_matches, 0);
        assert_eq!(report.changed, 1);
        assert_eq!(
            report
                .transitions
                .get("spawn_worker/eligible_fresh_spawn -> reject/depth_limit"),
            Some(&1)
        );
    }

    #[test]
    fn checked_in_v1_fixture_is_an_exact_default_regression() {
        let rows: Vec<FixtureRow> =
            serde_json::from_str(include_str!("../../testing/fixtures/policy-replay-v1.json"))
                .unwrap();
        let loaded = parse_payloads(
            rows.into_iter()
                .map(|row| (row.id, row.payload.to_string()))
                .collect(),
        );
        let report = replay(loaded, PolicyConfig::default());
        assert_eq!(report.replayable_decisions, 1);
        assert_eq!(report.exact_matches, 1);
        assert_eq!(report.changed, 0);
    }

    #[test]
    fn legacy_and_invalid_records_are_not_silent_evidence() {
        let loaded = parse_payloads(vec![
            (
                "legacy".into(),
                serde_json::json!({"decision":"queue","reason":"concurrency_limit"}).to_string(),
            ),
            (
                "future".into(),
                serde_json::json!({
                    "decision":"queue",
                    "reason":"concurrency_limit",
                    "replaySchemaVersion": 99
                })
                .to_string(),
            ),
            ("bad-json".into(), "{".into()),
        ]);
        assert_eq!(loaded.source_decisions, 2);
        assert_eq!(loaded.legacy_skipped, 1);
        assert_eq!(loaded.invalid_records.len(), 2);
        assert!(loaded.cases.is_empty());
    }

    #[test]
    fn newly_recorded_decision_replays_from_the_database_exactly() {
        let file = NamedTempFile::new().unwrap();
        {
            let db = store::open(file.path()).unwrap();
            db.execute(
                "INSERT INTO projects(id,name,path,created_at) VALUES('project','Replay','/tmp/replay','now')",
                [],
            ).unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('workspace','project','Pune','Replay','bridge/replay','/tmp/replay','idle','now')",
                [],
            ).unwrap();
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('parent','workspace','codex','Parent','idle','reported')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO session_heads(session_id,restoration_mode,updated_at)
                 VALUES('parent','fresh','now')",
                [],
            )
            .unwrap();
            let input = input(0);
            let outcome = PolicyEngine::default().decide(&input);
            record_decision(&db, "parent", "turn-1", &input, &outcome).unwrap();
        }
        let loaded = load_database(file.path()).unwrap();
        assert_eq!(loaded.cases.len(), 1);
        assert_eq!(replay(loaded, PolicyConfig::default()).exact_matches, 1);
    }
}
