//! Persisting a briefing run: the run row, its source coverage, and its evidence.
//!
//! Slice 1 declared these tables and left them empty. This is the writer, and it is
//! deliberately the *only* writer — the constraints that keep a run readable
//! (`UNIQUE(run_id, connector_instance_id)`, `UNIQUE(run_id, evidence_ref)`) are
//! declared by the schema, and going through one module is what keeps them from being
//! worked around by whichever caller is in a hurry.
//!
//! Two things it will not store: a credential, and a connector payload. What reaches
//! these tables is digests and Bridge-derived identity, which is everything a board
//! needs and nothing a leak could use.

use bridge_protocol::messages as wire;
use rusqlite::{params, Connection, OptionalExtension};

use crate::work_connectors::EvidenceTarget;
use crate::work_evidence::{EvidenceEntry, RunLedger, SourceCoverage};
use crate::BridgeError;

/// What starting a run needs. The limits are copied onto the row rather than
/// referenced, so a run stays readable as the run it was even after the settings that
/// produced it change.
#[derive(Debug, Clone)]
pub struct RunStart {
    pub run_id: String,
    pub trigger: wire::WorkBriefTrigger,
    pub profile_reference: Option<String>,
    pub session_id: Option<String>,
    pub limits: wire::WorkBriefLimits,
    /// Set to make a run at-most-once for a given cause. The schema's partial unique
    /// index is what enforces it.
    pub idempotency_key: Option<String>,
    pub started_at: String,
}

fn trigger_str(trigger: wire::WorkBriefTrigger) -> &'static str {
    match trigger {
        wire::WorkBriefTrigger::Manual => "manual",
        wire::WorkBriefTrigger::Focus => "focus",
        wire::WorkBriefTrigger::Schedule => "schedule",
    }
}

fn parse_trigger(value: &str) -> wire::WorkBriefTrigger {
    match value {
        "focus" => wire::WorkBriefTrigger::Focus,
        "schedule" => wire::WorkBriefTrigger::Schedule,
        // An unknown trigger reads as manual: a run somebody caused is the safe
        // assumption, and it is the only one that never implies a schedule exists.
        _ => wire::WorkBriefTrigger::Manual,
    }
}

fn status_str(status: wire::WorkBriefRunStatus) -> &'static str {
    match status {
        wire::WorkBriefRunStatus::Running => "running",
        wire::WorkBriefRunStatus::Succeeded => "succeeded",
        wire::WorkBriefRunStatus::Failed => "failed",
        wire::WorkBriefRunStatus::Cancelled => "cancelled",
        wire::WorkBriefRunStatus::Skipped => "skipped",
    }
}

fn parse_status(value: &str) -> wire::WorkBriefRunStatus {
    match value {
        "succeeded" => wire::WorkBriefRunStatus::Succeeded,
        "failed" => wire::WorkBriefRunStatus::Failed,
        "cancelled" => wire::WorkBriefRunStatus::Cancelled,
        "skipped" => wire::WorkBriefRunStatus::Skipped,
        // Anything unrecognised is treated as still running rather than as finished:
        // reporting an unknown state as done would claim an outcome nobody recorded.
        _ => wire::WorkBriefRunStatus::Running,
    }
}

fn source_status_str(status: wire::WorkSourceStatus) -> &'static str {
    match status {
        wire::WorkSourceStatus::Ineligible => "ineligible",
        wire::WorkSourceStatus::Eligible => "eligible",
        wire::WorkSourceStatus::Consulted => "consulted",
        wire::WorkSourceStatus::Succeeded => "succeeded",
        wire::WorkSourceStatus::Failed => "failed",
        wire::WorkSourceStatus::AuthRequired => "auth_required",
    }
}

fn parse_source_status(value: &str) -> wire::WorkSourceStatus {
    match value {
        "eligible" => wire::WorkSourceStatus::Eligible,
        "consulted" => wire::WorkSourceStatus::Consulted,
        "succeeded" => wire::WorkSourceStatus::Succeeded,
        "failed" => wire::WorkSourceStatus::Failed,
        "auth_required" => wire::WorkSourceStatus::AuthRequired,
        // A state this build cannot read is not offered to a model, which is the
        // fail-closed reading of an unknown row.
        _ => wire::WorkSourceStatus::Ineligible,
    }
}


/// A token counter from a row this build did not necessarily write.
///
/// `JsSafeU64` refuses a value JavaScript cannot represent exactly, and rightly — but
/// dropping a whole run's report over one implausible counter would lose the coverage,
/// the failure code, and the timings alongside it. So an unusable count reads as zero
/// and the rest of the run still renders.
fn js_safe(value: i64) -> wire::JsSafeU64 {
    wire::JsSafeU64::new(value.max(0) as u64)
        .unwrap_or_else(|_| wire::JsSafeU64::new(0).expect("zero is representable"))
}

/// Open a run. The row exists from this moment so a crash mid-run is visible as a run
/// that never completed rather than as nothing having happened.
pub fn begin_run(db: &Connection, start: &RunStart) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO work_brief_runs(
             id,trigger_kind,status,profile_reference,session_id,
             max_wall_seconds,max_turns,max_tool_calls,max_output_tokens,cost_ceiling_microusd,
             idempotency_key,started_at)
         VALUES(?1,?2,'running',?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![
            start.run_id,
            trigger_str(start.trigger),
            start.profile_reference,
            start.session_id,
            start.limits.max_wall_seconds,
            start.limits.max_turns,
            start.limits.max_tool_calls,
            start.limits.max_output_tokens,
            start.limits.cost_ceiling_microusd,
            start.idempotency_key,
            start.started_at,
        ],
    )?;
    Ok(())
}

/// How a run ended.
#[derive(Debug, Clone)]
pub struct RunOutcome {
    pub status: wire::WorkBriefRunStatus,
    /// A digest of the accepted output. Not the output: a board does not need the
    /// model's prose kept twice, and an unbounded copy of it is a liability.
    pub output_digest: Option<String>,
    /// A stable code, never payload text.
    pub failure_code: Option<String>,
    pub failure_detail: Option<String>,
    pub usage: Option<wire::WorkRunUsage>,
    pub tool_calls: i64,
    pub turns: i64,
    pub completed_at: String,
}

/// Close a run.
pub fn finish_run(db: &Connection, run_id: &str, outcome: &RunOutcome) -> Result<(), BridgeError> {
    let usage = outcome.usage.as_ref();
    db.execute(
        "UPDATE work_brief_runs SET
             status=?2,output_digest=?3,failure_code=?4,failure_detail=?5,
             input_tokens=?6,output_tokens=?7,cached_input_tokens=?8,cost_microusd=?9,
             tool_calls=?10,turns=?11,completed_at=?12
           WHERE id=?1",
        params![
            run_id,
            status_str(outcome.status),
            outcome.output_digest,
            outcome.failure_code,
            outcome.failure_detail,
            usage.map(|value| value.input_tokens.get() as i64).unwrap_or(0),
            usage.map(|value| value.output_tokens.get() as i64).unwrap_or(0),
            usage.map(|value| value.cached_input_tokens.get() as i64).unwrap_or(0),
            usage.and_then(|value| value.cost_microusd),
            outcome.tool_calls,
            outcome.turns,
            outcome.completed_at,
        ],
    )?;
    Ok(())
}

/// Write a ledger's coverage and evidence for a run.
///
/// One transaction: coverage that disagreed with the evidence beside it would be a
/// report nobody could act on, and a partial write is exactly how that happens.
pub fn record_ledger(db: &mut Connection, ledger: &RunLedger) -> Result<(), BridgeError> {
    let transaction = db.transaction()?;
    for source in ledger.coverage() {
        transaction.execute(
            "INSERT INTO work_brief_sources(run_id,connector_instance_id,connector_family,status,detail,observed_at)
             VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(run_id,connector_instance_id) DO UPDATE SET
                 status=excluded.status,detail=excluded.detail,observed_at=excluded.observed_at",
            params![
                ledger.run_id(),
                source.connector_instance_id,
                source.connector_family,
                source_status_str(source.status),
                source.detail,
                source.observed_at,
            ],
        )?;
    }
    for entry in ledger.entries() {
        transaction.execute(
            "INSERT INTO work_evidence(
                 run_id,evidence_ref,tool_call_id,connector_instance_id,canonical_resource_id,
                 source_kind,target,tool_definition_digest,result_digest,succeeded,observed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10)
             ON CONFLICT(run_id,evidence_ref) DO NOTHING",
            params![
                ledger.run_id(),
                entry.evidence_ref,
                entry.tool_call_id,
                entry.connector_instance_id,
                entry.canonical_resource_id,
                entry.source_kind,
                target_json(&entry.target),
                entry.tool_definition_digest,
                entry.result_digest,
                entry.observed_at,
            ],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

/// A target is stored as its tagged JSON, so reading one back cannot lose which kind
/// it was — a bare URL column would make "no target" and "a target we dropped"
/// indistinguishable.
fn target_json(target: &EvidenceTarget) -> Option<String> {
    match target {
        EvidenceTarget::None => None,
        other => serde_json::to_string(other).ok(),
    }
}

/// Read one run, for **Inspect run** and for the board's `latestRun`.
///
/// Note what this does not need: the hidden session. Usage and coverage are readable
/// without selecting it, which is the whole point of keeping the transcript separate.
pub fn read_run(db: &Connection, run_id: &str) -> Result<Option<wire::WorkBriefRun>, BridgeError> {
    let run = db
        .query_row(
            "SELECT id,trigger_kind,status,profile_reference,session_id,output_digest,
                    failure_code,failure_detail,input_tokens,output_tokens,cached_input_tokens,
                    cost_microusd,started_at,completed_at,tool_calls,turns
               FROM work_brief_runs WHERE id=?1",
            params![run_id],
            |row| {
                let input: i64 = row.get(8)?;
                let output: i64 = row.get(9)?;
                let cached: i64 = row.get(10)?;
                let cost: Option<i64> = row.get(11)?;
                Ok(wire::WorkBriefRun {
                    id: row.get(0)?,
                    trigger: parse_trigger(&row.get::<_, String>(1)?),
                    status: parse_status(&row.get::<_, String>(2)?),
                    profile_reference: row.get(3)?,
                    session_id: row.get(4)?,
                    output_digest: row.get(5)?,
                    failure_code: row.get(6)?,
                    failure_detail: row.get(7)?,
                    usage: Some(wire::WorkRunUsage {
                        input_tokens: js_safe(input),
                        output_tokens: js_safe(output),
                        cached_input_tokens: js_safe(cached),
                        cost_microusd: cost,
                        tool_calls: row.get(14)?,
                        turns: row.get(15)?,
                    }),
                    started_at: row.get(12)?,
                    completed_at: row.get(13)?,
                })
            },
        )
        .optional()?;
    Ok(run)
}

/// The most recently started run, whatever became of it.
pub fn latest_run(db: &Connection) -> Result<Option<wire::WorkBriefRun>, BridgeError> {
    let id: Option<String> = db
        .query_row(
            "SELECT id FROM work_brief_runs ORDER BY started_at DESC, rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        Some(id) => read_run(db, &id),
        None => Ok(None),
    }
}

/// A run's source coverage, ordered by connector so a report is stable.
pub fn read_coverage(db: &Connection, run_id: &str) -> Result<Vec<wire::WorkSourceCoverage>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT connector_instance_id,connector_family,status,detail,observed_at
           FROM work_brief_sources WHERE run_id=?1 ORDER BY connector_instance_id",
    )?;
    let rows = statement.query_map(params![run_id], |row| {
        Ok(wire::WorkSourceCoverage {
            connector_instance_id: row.get(0)?,
            connector_family: row.get(1)?,
            status: parse_source_status(&row.get::<_, String>(2)?),
            detail: row.get(3)?,
            observed_at: row.get(4)?,
        })
    })?;
    let mut coverage = Vec::new();
    for row in rows {
        coverage.push(row?);
    }
    Ok(coverage)
}

/// One run's evidence, as stored. For **Inspect run** and for slice 5's reconciliation.
pub fn read_evidence(db: &Connection, run_id: &str) -> Result<Vec<EvidenceEntry>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT evidence_ref,tool_call_id,connector_instance_id,canonical_resource_id,
                source_kind,target,tool_definition_digest,result_digest,observed_at
           FROM work_evidence WHERE run_id=?1 AND succeeded=1 ORDER BY evidence_ref",
    )?;
    let rows = statement.query_map(params![run_id], |row| {
        let target: Option<String> = row.get(5)?;
        Ok(EvidenceEntry {
            evidence_ref: row.get(0)?,
            tool_call_id: row.get(1)?,
            connector_instance_id: row.get(2)?,
            // Not stored on the evidence row: it belongs to the connector instance, and
            // duplicating it here would let the two disagree.
            account_identity: None,
            canonical_resource_id: row.get(3)?,
            source_kind: row.get(4)?,
            tool_definition_digest: row.get(6)?,
            result_digest: row.get(7)?,
            target: target
                .and_then(|value| serde_json::from_str(&value).ok())
                .unwrap_or(EvidenceTarget::None),
            observed_at: row.get(8)?,
        })
    })?;
    let mut evidence = Vec::new();
    for row in rows {
        evidence.push(row?);
    }
    Ok(evidence)
}

/// Coverage as this module's own type, for callers that want the counts rather than the
/// wire shape.
pub fn read_coverage_rows(db: &Connection, run_id: &str) -> Result<Vec<SourceCoverage>, BridgeError> {
    Ok(read_coverage(db, run_id)?
        .into_iter()
        .map(|row| SourceCoverage {
            connector_instance_id: row.connector_instance_id,
            connector_family: row.connector_family,
            status: row.status,
            detail: row.detail,
            observed_at: row.observed_at,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use crate::work_connectors::ConnectorFamily;
    use serde_json::json;

    const STARTED: &str = "2026-08-19T12:00:00+00:00";
    const FINISHED: &str = "2026-08-19T12:00:41+00:00";

    fn db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn limits() -> wire::WorkBriefLimits {
        wire::WorkBriefLimits {
            max_wall_seconds: 600,
            max_turns: 12,
            max_tool_calls: 24,
            max_output_tokens: Some(4_000),
            cost_ceiling_microusd: Some(50_000),
        }
    }

    fn start(run_id: &str) -> RunStart {
        RunStart {
            run_id: run_id.into(),
            trigger: wire::WorkBriefTrigger::Manual,
            profile_reference: Some("claude/sonnet/medium".into()),
            session_id: None,
            limits: limits(),
            idempotency_key: None,
            started_at: STARTED.into(),
        }
    }

    fn ledger(run_id: &str) -> RunLedger {
        let mut ledger = RunLedger::new(run_id);
        ledger.record_source("gmail-1", "gmail", wire::WorkSourceStatus::AuthRequired, None, None);
        ledger.record_source("github-1", "github", wire::WorkSourceStatus::Eligible, None, None);
        ledger.record_consulted("linear-1", "linear");
        ledger.record_failed("notion-1", "notion", "the connector returned 503");
        ledger
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-1",
                Some("T1/U1"),
                "call_1",
                "tool-digest",
                &json!({
                    "ts": "1723459200.123",
                    "permalink": "https://app.slack.com/archives/C1/p1",
                    "text": "sk-ant-a-secret",
                }),
                STARTED,
            )
            .unwrap();
        ledger
    }

    #[test]
    fn a_run_exists_from_the_moment_it_starts() {
        // So a crash mid-run reads as a run that never completed, rather than as
        // nothing having happened.
        let db = db();
        begin_run(&db, &start("run-1")).unwrap();
        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Running);
        assert!(run.completed_at.is_none());
        assert_eq!(run.profile_reference.as_deref(), Some("claude/sonnet/medium"));
        assert_eq!(run.started_at, STARTED);
    }

    #[test]
    fn finishing_records_the_outcome_the_usage_and_the_code() {
        let db = db();
        begin_run(&db, &start("run-1")).unwrap();
        finish_run(
            &db,
            "run-1",
            &RunOutcome {
                status: wire::WorkBriefRunStatus::Succeeded,
                output_digest: Some("a".repeat(64)),
                failure_code: None,
                failure_detail: None,
                usage: Some(wire::WorkRunUsage {
                    input_tokens: js_safe(1_200),
                    output_tokens: js_safe(340),
                    cached_input_tokens: js_safe(900),
                    cost_microusd: Some(4_100),
                    tool_calls: 3,
                    turns: 2,
                }),
                tool_calls: 3,
                turns: 2,
                completed_at: FINISHED.into(),
            },
        )
        .unwrap();
        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Succeeded);
        assert_eq!(run.completed_at.as_deref(), Some(FINISHED));
        let usage = run.usage.unwrap();
        assert_eq!(usage.input_tokens.get(), 1_200);
        assert_eq!(usage.cached_input_tokens.get(), 900);
        assert_eq!(usage.cost_microusd, Some(4_100));
        assert_eq!(usage.tool_calls, 3);
        assert_eq!(usage.turns, 2);
    }

    #[test]
    fn a_failed_run_keeps_its_code_and_no_output_digest() {
        let db = db();
        begin_run(&db, &start("run-1")).unwrap();
        finish_run(
            &db,
            "run-1",
            &RunOutcome {
                status: wire::WorkBriefRunStatus::Failed,
                output_digest: None,
                failure_code: Some("evidence_invalid".into()),
                failure_detail: Some("a task cited evidence this run did not earn".into()),
                usage: None,
                tool_calls: 1,
                turns: 2,
                completed_at: FINISHED.into(),
            },
        )
        .unwrap();
        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert_eq!(run.status, wire::WorkBriefRunStatus::Failed);
        assert_eq!(run.failure_code.as_deref(), Some("evidence_invalid"));
        assert!(run.output_digest.is_none());
    }

    #[test]
    fn an_idempotency_key_makes_a_run_at_most_once() {
        // The schema's partial unique index is what enforces it; this pins that the
        // constraint is actually reachable through this writer.
        let db = db();
        let mut first = start("run-1");
        first.idempotency_key = Some("focus:2026-08-19T12".into());
        begin_run(&db, &first).unwrap();
        let mut second = start("run-2");
        second.idempotency_key = Some("focus:2026-08-19T12".into());
        assert!(begin_run(&db, &second).is_err(), "the same cause must not start two runs");
        // A run with no key is unconstrained, which is what makes manual runs repeatable.
        begin_run(&db, &start("run-3")).unwrap();
        begin_run(&db, &start("run-4")).unwrap();
    }

    #[test]
    fn coverage_is_one_row_per_connector_and_survives_a_rewrite() {
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        let subject = ledger("run-1");
        record_ledger(&mut db, &subject).unwrap();
        // Writing the same ledger again must not double the rows: the unique constraint
        // plus the upsert is what makes a mid-run flush safe to repeat.
        record_ledger(&mut db, &subject).unwrap();

        let coverage = read_coverage(&db, "run-1").unwrap();
        assert_eq!(coverage.len(), 5);
        assert_eq!(
            coverage.iter().map(|row| row.connector_instance_id.as_str()).collect::<Vec<_>>(),
            vec!["github-1", "gmail-1", "linear-1", "notion-1", "slack-1"],
            "ordered by connector, so a report does not depend on call order"
        );
        let by_id = |id: &str| coverage.iter().find(|row| row.connector_instance_id == id).unwrap().status;
        assert_eq!(by_id("gmail-1"), wire::WorkSourceStatus::AuthRequired);
        assert_eq!(by_id("github-1"), wire::WorkSourceStatus::Eligible);
        assert_eq!(by_id("linear-1"), wire::WorkSourceStatus::Consulted);
        assert_eq!(by_id("notion-1"), wire::WorkSourceStatus::Failed);
        assert_eq!(by_id("slack-1"), wire::WorkSourceStatus::Succeeded);
    }

    #[test]
    fn evidence_round_trips_with_its_bridge_derived_provenance() {
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        record_ledger(&mut db, &ledger("run-1")).unwrap();

        let evidence = read_evidence(&db, "run-1").unwrap();
        assert_eq!(evidence.len(), 1);
        let entry = &evidence[0];
        assert_eq!(entry.canonical_resource_id, "slack:slack-1:1723459200.123");
        assert_eq!(entry.tool_definition_digest, "tool-digest");
        assert_eq!(entry.result_digest.len(), 64);
        assert_eq!(
            entry.target,
            EvidenceTarget::ExternalLink {
                url: "https://app.slack.com/archives/C1/p1".into(),
                host: "app.slack.com".into(),
            }
        );
    }

    #[test]
    fn the_tables_store_no_payload_and_no_credential() {
        // The result carried a secret. Dumping every value from both tables must not
        // turn it up: what is stored is digests and identity.
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        record_ledger(&mut db, &ledger("run-1")).unwrap();

        let mut dumped = String::new();
        for table in ["work_brief_runs", "work_brief_sources", "work_evidence"] {
            let mut statement = db.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let columns = statement.column_count();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for index in 0..columns {
                    if let Ok(value) = row.get::<_, Option<String>>(index) {
                        dumped.push_str(value.as_deref().unwrap_or(""));
                        dumped.push('\n');
                    }
                }
            }
        }
        assert!(!dumped.contains("sk-ant"), "no credential-shaped text is stored");
        assert!(!dumped.contains("a-secret"), "no payload text is stored");
        assert!(dumped.contains("slack:slack-1:"), "but the derived identity is");
    }

    #[test]
    fn a_target_that_was_never_safe_reads_back_as_none() {
        // Stored as tagged JSON, so "no target" and "a target we dropped" cannot become
        // the same thing on the way back out.
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        let mut subject = RunLedger::new("run-1");
        subject
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-1",
                None,
                "call_1",
                "d",
                &json!({"ts": "1.1", "permalink": "https://evil.example/x"}),
                STARTED,
            )
            .unwrap();
        record_ledger(&mut db, &subject).unwrap();
        assert_eq!(read_evidence(&db, "run-1").unwrap()[0].target, EvidenceTarget::None);
    }

    #[test]
    fn usage_and_coverage_are_readable_without_the_hidden_session() {
        // The transcript is for Inspect run and nothing else, so neither read touches
        // the sessions table.
        let mut db = db();
        let mut with_session = start("run-1");
        with_session.session_id = None;
        begin_run(&db, &with_session).unwrap();
        record_ledger(&mut db, &ledger("run-1")).unwrap();
        let run = read_run(&db, "run-1").unwrap().unwrap();
        assert!(run.session_id.is_none());
        assert!(run.usage.is_some());
        assert_eq!(read_coverage(&db, "run-1").unwrap().len(), 5);
    }

    #[test]
    fn the_latest_run_is_the_most_recently_started_whatever_became_of_it() {
        let db = db();
        begin_run(&db, &start("run-1")).unwrap();
        let mut later = start("run-2");
        later.started_at = "2026-08-19T13:00:00+00:00".into();
        begin_run(&db, &later).unwrap();
        finish_run(
            &db,
            "run-2",
            &RunOutcome {
                status: wire::WorkBriefRunStatus::Failed,
                output_digest: None,
                failure_code: Some("schema_invalid".into()),
                failure_detail: None,
                usage: None,
                tool_calls: 0,
                turns: 1,
                completed_at: FINISHED.into(),
            },
        )
        .unwrap();
        let latest = latest_run(&db).unwrap().unwrap();
        assert_eq!(latest.id, "run-2");
        assert_eq!(latest.status, wire::WorkBriefRunStatus::Failed);
    }

    #[test]
    fn an_unknown_stored_status_reads_as_running_not_as_finished() {
        // A row a later build wrote must not be reported as an outcome nobody recorded.
        let db = db();
        begin_run(&db, &start("run-1")).unwrap();
        db.execute("UPDATE work_brief_runs SET status='some_future_state' WHERE id='run-1'", []).unwrap();
        assert_eq!(
            read_run(&db, "run-1").unwrap().unwrap().status,
            wire::WorkBriefRunStatus::Running
        );
    }

    #[test]
    fn an_unknown_stored_source_status_reads_as_ineligible() {
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        record_ledger(&mut db, &ledger("run-1")).unwrap();
        db.execute("UPDATE work_brief_sources SET status='future' WHERE connector_instance_id='slack-1'", []).unwrap();
        let coverage = read_coverage(&db, "run-1").unwrap();
        let slack = coverage.iter().find(|row| row.connector_instance_id == "slack-1").unwrap();
        assert_eq!(slack.status, wire::WorkSourceStatus::Ineligible, "fail closed");
    }

    #[test]
    fn deleting_a_run_takes_its_coverage_and_evidence_with_it() {
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        record_ledger(&mut db, &ledger("run-1")).unwrap();
        db.execute("DELETE FROM work_brief_runs WHERE id='run-1'", []).unwrap();
        assert!(read_coverage(&db, "run-1").unwrap().is_empty());
        assert!(read_evidence(&db, "run-1").unwrap().is_empty());
    }

    #[test]
    fn an_unstarted_run_reads_as_nothing_rather_than_erroring() {
        let db = db();
        assert!(read_run(&db, "run-nope").unwrap().is_none());
        assert!(latest_run(&db).unwrap().is_none());
    }

    #[test]
    fn a_run_writes_no_tasks_because_committing_them_is_the_next_slice() {
        let mut db = db();
        begin_run(&db, &start("run-1")).unwrap();
        record_ledger(&mut db, &ledger("run-1")).unwrap();
        let tasks: i64 = db.query_row("SELECT COUNT(*) FROM work_tasks", [], |row| row.get(0)).unwrap();
        assert_eq!(tasks, 0);
    }
}
