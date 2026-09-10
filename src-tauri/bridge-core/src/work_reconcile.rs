//! Committing a briefing: one transaction, or nothing.
//!
//! Every earlier piece decided something. This applies them all at once, because the
//! pieces disagree if they land separately: coverage saying a source was read while the
//! tasks from it are un-aged, or a board half-replaced by a brief whose evidence is not
//! stored yet. A reader must see the previous board or the whole new one.
//!
//! It is also the only writer of `work_tasks`. Slice 4 deliberately wrote none, and a test
//! there pins that; the identity rules from step 1 and the state rules from step 2 reach
//! the table through here or not at all.

use std::collections::{BTreeMap, BTreeSet};

use bridge_protocol::messages as wire;
use rusqlite::{params, Connection, OptionalExtension};

use crate::work_brief_parser::WorkBrief;
use crate::work_brief_store::{record_ledger_in, RunOutcome};
use crate::work_evidence::{EvidenceEntry, RunLedger};
use crate::work_fingerprint::fingerprint_for;
use crate::work_task_state::{
    reconcile_absent, reconcile_present, SourceOutcome, TaskSnapshot, TaskState,
};
use crate::BridgeError;

/// A task as it exists in the table, for reconciliation.
#[derive(Debug, Clone)]
struct StoredTask {
    id: String,
    fingerprint: Option<String>,
    connector_instance_id: String,
    snapshot: TaskSnapshot,
}

/// What one accepted brief plus its ledger commits.
pub struct Commit<'a> {
    pub run_id: &'a str,
    pub brief: &'a WorkBrief,
    pub ledger: &'a RunLedger,
    pub outcome: RunOutcome,
    pub now: &'a str,
}

/// Everything a task needs that the brief does not carry.
///
/// Resolved from the cited evidence rather than from the model, which is the whole reason
/// slice 4 hands references back: a task's source, resource, and observed time come from
/// the ledger row its citation names.
struct Resolved<'a> {
    entry: &'a EvidenceEntry,
    fingerprint: Option<String>,
}

fn resolve<'a>(ledger: &'a RunLedger, evidence_ref: &str) -> Option<Resolved<'a>> {
    let entry = ledger.entry(evidence_ref)?;
    Some(Resolved {
        fingerprint: fingerprint_for(&entry.connector_instance_id, entry.canonical_resource_id.as_deref()),
        entry,
    })
}

/// Commit a validated brief.
///
/// Returns how many tasks the board now holds. Everything inside happens in one
/// transaction: on any error nothing is written, and the previous board is what a reader
/// still sees.
pub fn commit_brief(db: &mut Connection, commit: Commit<'_>) -> Result<usize, BridgeError> {
    let transaction = db.transaction()?;

    record_ledger_in(&transaction, commit.ledger)?;

    // What each source got up to in this run, so ageing is source-scoped.
    let outcomes: BTreeMap<String, SourceOutcome> = commit
        .ledger
        .coverage()
        .into_iter()
        .map(|row| {
            (
                row.connector_instance_id,
                SourceOutcome::from_coverage(row.status),
            )
        })
        .collect();

    // Which fingerprints this brief mentions, and what backs each.
    let mut present: BTreeMap<String, (i64, &crate::work_brief_parser::BriefTask, &EvidenceEntry)> =
        BTreeMap::new();
    let mut ephemeral: Vec<(&crate::work_brief_parser::BriefTask, &EvidenceEntry)> = Vec::new();
    for task in &commit.brief.tasks {
        // A multi-citation task names one primary citation explicitly. The parser permits
        // omission only for a single citation, where identity is unambiguous. Supporting
        // evidence can then change without creating a second durable task.
        let primary_ref = task.primary_evidence.as_deref().or_else(|| task.evidence.first().map(String::as_str));
        let Some(primary) = primary_ref.and_then(|reference| resolve(commit.ledger, reference)) else {
            continue;
        };
        match primary.fingerprint {
            Some(value) => {
                // If the model repeats one resource as two tasks, keep the higher-ranked
                // row rather than silently replacing it with the later duplicate.
                present
                    .entry(value)
                    .and_modify(|current| {
                        if task.rank < current.0 {
                            *current = (task.rank, task, primary.entry);
                        }
                    })
                    .or_insert((task.rank, task, primary.entry));
            }
            None => ephemeral.push((task, primary.entry)),
        }
    }

    // Read only rows this run can change: the previous run's ephemerals, tasks this brief
    // mentioned, and live tasks from sources that succeeded and have not exhausted their
    // two meaningful misses. Historical done/stale/dismissed rows stay off the hot path.
    let mut sql = String::from(
        "SELECT id,fingerprint,connector_instance_id,state,pinned,miss_count,ephemeral,resolved_at,
                evidence_digest,resolution
           FROM work_tasks
          WHERE ephemeral=1
             OR (state IN ('active','snoozed') AND miss_count < 2 AND connector_instance_id IN (
                    SELECT connector_instance_id FROM work_brief_sources
                     WHERE run_id=?1 AND status='succeeded'))",
    );
    let mut bindings = vec![commit.run_id.to_owned()];
    if !present.is_empty() {
        let placeholders = present.keys().map(|fingerprint| {
            bindings.push(fingerprint.clone());
            format!("?{}", bindings.len())
        }).collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" OR fingerprint IN ({placeholders})"));
    }
    let mut stored = Vec::new();
    {
        let mut statement = transaction.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(bindings.iter()), |row| {
            Ok(StoredTask {
                id: row.get(0)?, fingerprint: row.get(1)?, connector_instance_id: row.get(2)?,
                snapshot: TaskSnapshot {
                    state: TaskState::parse(&row.get::<_, String>(3)?),
                    pinned: row.get::<_, i64>(4)? != 0,
                    miss_count: row.get(5)?, ephemeral: row.get::<_, i64>(6)? != 0,
                    resolved_at: row.get(7)?,
                    evidence_digest: row.get(8)?,
                    resolved_evidence_digest: row.get(9)?,
                },
            })
        })?;
        for row in rows { stored.push(row?); }
    }

    // Age or refresh every task already on the board.
    let mentioned: BTreeSet<&String> = present.keys().collect();
    for existing in &stored {
        let next = match existing.fingerprint.as_ref().filter(|value| mentioned.contains(value)) {
            Some(value) => {
                let (_, _, entry) = &present[value.as_str()];
                reconcile_present(&existing.snapshot, Some(entry.result_digest.as_str()))
            }
            None if existing.snapshot.ephemeral => {
                // Ephemeral tasks last until the next committed briefing, and this function
                // only runs on one — so they go. A failed run reaches `abandon_run`, which
                // touches no task at all, and that is where they survive.
                transaction.execute("DELETE FROM work_tasks WHERE id=?1", params![existing.id])?;
                continue;
            }
            None => {
                let outcome = outcomes
                    .get(&existing.connector_instance_id)
                    .copied()
                    // A source this run never mentioned was not read, so it ages nothing.
                    .unwrap_or(SourceOutcome::NotRead);
                reconcile_absent(&existing.snapshot, outcome)
            }
        };
        if next == existing.snapshot {
            continue;
        }
        transaction.execute(
            "UPDATE work_tasks SET state=?2,pinned=?3,miss_count=?4,resolved_at=?5,resolution=?6,
                    last_run_id=?7,updated_at=?8
               WHERE id=?1",
            params![
                existing.id,
                next.state.as_str(),
                i64::from(next.pinned),
                next.miss_count,
                next.resolved_at,
                next.resolved_evidence_digest,
                commit.run_id,
                commit.now,
            ],
        )?;
    }

    // Insert or refresh what the brief proposed. A task the user has put away keeps its
    // state: the upsert deliberately does not touch state, pinned, miss_count or
    // resolved_at or the completion's evidence snapshot, all of which were just decided above.
    for (value, (rank, task, entry)) in &present {
        upsert_task(
            &transaction,
            commit.run_id,
            commit.now,
            Some(value.as_str()),
            rank,
            task,
            entry,
            false,
        )?;
    }
    for (task, entry) in &ephemeral {
        upsert_task(&transaction, commit.run_id, commit.now, None, &task.rank, task, entry, true)?;
    }

    finish_run_in(&transaction, commit.run_id, &commit.outcome)?;
    let total: i64 = transaction.query_row("SELECT COUNT(*) FROM work_tasks", [], |row| row.get(0))?;
    transaction.commit()?;
    Ok(total.max(0) as usize)
}

#[allow(clippy::too_many_arguments)]
fn upsert_task(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
    now: &str,
    fingerprint: Option<&str>,
    rank: &i64,
    task: &crate::work_brief_parser::BriefTask,
    entry: &EvidenceEntry,
    ephemeral: bool,
) -> Result<(), BridgeError> {
    let target = match &entry.target {
        crate::work_connectors::EvidenceTarget::None => None,
        other => serde_json::to_string(other).ok(),
    };
    // An ephemeral task has no fingerprint to conflict on, so it is always an insert — and
    // the previous committed run's ephemerals were deleted above.
    let id = match fingerprint {
        Some(value) => format!("task-{value}"),
        None => format!("task-ephemeral-{run_id}-{rank}"),
    };
    transaction.execute(
        "INSERT INTO work_tasks(
             id,fingerprint,connector_instance_id,canonical_resource_id,source_kind,
             title,why,rank,confidence_bps,evidence_digest,evidence_target,evidence_observed_at,
             ephemeral,first_run_id,last_run_id,created_at,updated_at,source_activity_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14,?15,?15,?16)
         ON CONFLICT(id) DO UPDATE SET
             title=excluded.title,why=excluded.why,rank=excluded.rank,
             confidence_bps=excluded.confidence_bps,evidence_digest=excluded.evidence_digest,
             evidence_target=excluded.evidence_target,
             evidence_observed_at=excluded.evidence_observed_at,
             source_activity_at=excluded.source_activity_at,
             last_run_id=excluded.last_run_id,updated_at=excluded.updated_at",
        params![
            id,
            fingerprint,
            entry.connector_instance_id,
            entry.canonical_resource_id,
            entry.source_kind,
            task.title,
            task.why,
            rank,
            task.confidence_bps,
            entry.result_digest,
            target,
            entry.observed_at,
            i64::from(ephemeral),
            run_id,
            now,
            entry.source_activity_at,
        ],
    )?;
    Ok(())
}

/// `finish_run`, inside the caller's transaction.
///
/// The run's own row has to land with the board: a succeeded run beside a board that was
/// never written is the inconsistency this whole function exists to prevent.
fn finish_run_in(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
    outcome: &RunOutcome,
) -> Result<(), BridgeError> {
    let usage = outcome.usage.as_ref();
    transaction.execute(
        "UPDATE work_brief_runs SET
             status=?2,output_digest=?3,failure_code=?4,failure_detail=?5,
             input_tokens=?6,output_tokens=?7,cached_input_tokens=?8,cost_microusd=?9,
             tool_calls=?10,turns=?11,completed_at=?12
           WHERE id=?1",
        params![
            run_id,
            match outcome.status {
                wire::WorkBriefRunStatus::Running => "running",
                wire::WorkBriefRunStatus::Succeeded => "succeeded",
                wire::WorkBriefRunStatus::Failed => "failed",
                wire::WorkBriefRunStatus::Cancelled => "cancelled",
                wire::WorkBriefRunStatus::Skipped => "skipped",
            },
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

/// Close a run that produced nothing, leaving the board exactly as it was.
///
/// Not a transaction over the board at all: there is nothing to reconcile, and a board that
/// emptied itself on a parse error would be worse than a stale one.
pub fn abandon_run(
    db: &mut Connection,
    run_id: &str,
    ledger: &RunLedger,
    outcome: &RunOutcome,
) -> Result<(), BridgeError> {
    let transaction = db.transaction()?;
    record_ledger_in(&transaction, ledger)?;
    finish_run_in(&transaction, run_id, outcome)?;
    transaction.commit()?;
    Ok(())
}

/// The board a reader sees: visible tasks in rank order.
pub fn board_tasks(db: &Connection) -> Result<Vec<(String, String, TaskState, bool, i64)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id,title,state,pinned,rank FROM work_tasks ORDER BY rank,id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            TaskState::parse(&row.get::<_, String>(2)?),
            row.get::<_, i64>(3)? != 0,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let mut tasks = Vec::new();
    for row in rows {
        let task = row?;
        if task.2.visible(task.3) {
            tasks.push(task);
        }
    }
    Ok(tasks)
}

/// One task's stored state, for a caller applying an action.
pub fn read_task(db: &Connection, task_id: &str) -> Result<Option<TaskSnapshot>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT state,pinned,miss_count,ephemeral,resolved_at,evidence_digest,resolution
               FROM work_tasks WHERE id=?1",
            params![task_id],
            |row| {
                Ok(TaskSnapshot {
                    state: TaskState::parse(&row.get::<_, String>(0)?),
                    pinned: row.get::<_, i64>(1)? != 0,
                    miss_count: row.get(2)?,
                    ephemeral: row.get::<_, i64>(3)? != 0,
                    resolved_at: row.get(4)?,
                    evidence_digest: row.get(5)?,
                    resolved_evidence_digest: row.get(6)?,
                })
            },
        )
        .optional()?)
}

/// Write one task's state back after an action.
pub fn write_task_state(
    db: &Connection,
    task_id: &str,
    snapshot: &TaskSnapshot,
    snoozed_until: Option<&str>,
    now: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE work_tasks SET state=?2,pinned=?3,resolved_at=?4,resolution=?5,
                snoozed_until=?6,updated_at=?7
           WHERE id=?1",
        params![
            task_id,
            snapshot.state.as_str(),
            i64::from(snapshot.pinned),
            snapshot.resolved_at,
            snapshot.resolved_evidence_digest,
            snoozed_until,
            now,
        ],
    )?;
    Ok(())
}

/// Write only the orthogonal pin flag, preserving snooze and resolution metadata.
pub fn write_task_pin(db: &Connection, task_id: &str, pinned: bool, now: &str) -> Result<(), BridgeError> {
    let changed = db.execute(
        "UPDATE work_tasks SET pinned=?2,updated_at=?3 WHERE id=?1",
        params![task_id, i64::from(pinned), now],
    )?;
    if changed == 0 {
        return Err(BridgeError::Invalid("that task is not on the board".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use crate::work_brief_parser::BriefTask;
    use crate::work_brief_store::{begin_run, RunStart};
    use crate::work_connectors::ConnectorFamily;
    use crate::work_task_state::{apply_action, TaskAction};
    use serde_json::json;

    const T0: &str = "2026-08-19T12:00:00+00:00";
    const T1: &str = "2026-08-19T13:00:00+00:00";

    fn db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn limits() -> wire::WorkBriefLimits {
        wire::WorkBriefLimits {
            max_wall_seconds: 600,
            max_turns: 12,
            max_tool_calls: 24,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        }
    }

    fn start(db: &Connection, run_id: &str) {
        begin_run(
            db,
            &RunStart {
                run_id: run_id.into(),
                trigger: wire::WorkBriefTrigger::Manual,
                profile_reference: Some("claude/sonnet".into()),
                session_id: None,
                limits: limits(),
                idempotency_key: None,
                started_at: T0.into(),
                lease_owner: None,
                lease_expires_at: None,
            },
        )
        .unwrap();
    }

    fn succeeded() -> RunOutcome {
        RunOutcome {
            status: wire::WorkBriefRunStatus::Succeeded,
            output_digest: Some("d".repeat(64)),
            failure_code: None,
            failure_detail: None,
            usage: None,
            tool_calls: 1,
            turns: 1,
            completed_at: T0.into(),
        }
    }

    /// A ledger holding one Slack message, read successfully.
    fn ledger(run_id: &str, ts: &str, observed_at: &str) -> (RunLedger, String) {
        let mut ledger = RunLedger::new(run_id);
        let reference = ledger
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-1",
                Some("T1/U1"),
                "call_1",
                "tool-digest",
                &json!({"ts": ts, "permalink": "https://app.slack.com/archives/C1/p1"}),
                observed_at,
            )
            .unwrap();
        (ledger, reference)
    }

    fn brief(reference: &str, title: &str) -> WorkBrief {
        WorkBrief {
            version: 1,
            tasks: vec![BriefTask {
                rank: 1,
                title: title.into(),
                why: "Asked twice.".into(),
                confidence_bps: 8_200,
                primary_evidence: None,
                evidence: vec![reference.to_owned()],
            }],
        }
    }

    fn empty_brief() -> WorkBrief {
        WorkBrief { version: 1, tasks: vec![] }
    }

    fn commit(db: &mut Connection, run_id: &str, brief: &WorkBrief, ledger: &RunLedger, now: &str) -> usize {
        commit_brief(
            db,
            Commit { run_id, brief, ledger, outcome: succeeded(), now },
        )
        .unwrap()
    }

    // -----------------------------------------------------------------------
    // One transaction
    // -----------------------------------------------------------------------

    #[test]
    fn a_committed_brief_lands_a_board_and_the_runs_own_row_together() {
        let mut db = db();
        start(&db, "run-1");
        let (ledger, reference) = ledger("run-1", "1.1", T0);
        assert_eq!(commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &ledger, T0), 1);

        let board = board_tasks(&db).unwrap();
        assert_eq!(board.len(), 1);
        assert_eq!(board[0].1, "Reply to Priya");
        // The run says succeeded, and it says so in the same transaction as the board — a
        // succeeded run beside a board that was never written is the inconsistency this
        // function exists to prevent.
        let status: String = db
            .query_row("SELECT status FROM work_brief_runs WHERE id='run-1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "succeeded");
    }

    #[test]
    fn replaying_the_same_run_changes_nothing() {
        let mut db = db();
        start(&db, "run-1");
        let (ledger, reference) = ledger("run-1", "1.1", T0);
        let brief = brief(&reference, "Reply to Priya");
        commit(&mut db, "run-1", &brief, &ledger, T0);
        let before = dump(&db);
        commit(&mut db, "run-1", &brief, &ledger, T0);
        assert_eq!(dump(&db), before, "a replay is a no-op, not a second task");
    }

    #[test]
    fn a_failed_run_keeps_the_previous_board() {
        let mut db = db();
        start(&db, "run-1");
        let (ledger, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &ledger, T0);
        let before = dump(&db);

        // A run that produced nothing usable closes itself and touches no task.
        start(&db, "run-2");
        let failed_ledger = RunLedger::new("run-2");
        abandon_run(
            &mut db,
            "run-2",
            &failed_ledger,
            &RunOutcome {
                status: wire::WorkBriefRunStatus::Failed,
                output_digest: None,
                failure_code: Some("evidence_invalid".into()),
                failure_detail: None,
                usage: None,
                tool_calls: 0,
                turns: 1,
                completed_at: T1.into(),
            },
        )
        .unwrap();
        assert_eq!(dump(&db), before, "a board must not empty itself on a parse error");
    }

    /// Every task row, for comparing a board before and after.
    fn dump(db: &Connection) -> Vec<String> {
        let mut statement = db
            .prepare("SELECT id,fingerprint,title,rank,state,pinned,miss_count,ephemeral FROM work_tasks ORDER BY id")
            .unwrap();
        let rows = statement
            .query_map([], |row| {
                Ok(format!(
                    "{}|{:?}|{}|{}|{}|{}|{}|{}",
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                ))
            })
            .unwrap();
        rows.map(Result::unwrap).collect()
    }

    // -----------------------------------------------------------------------
    // Ageing is source-scoped, end to end
    // -----------------------------------------------------------------------

    #[test]
    fn two_runs_that_read_the_source_and_omit_the_task_make_it_stale() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);

        // The source is read again — a different message — and the task is not mentioned.
        for run in ["run-2", "run-3"] {
            start(&db, run);
            let (ledger, _) = ledger(run, "9.9", T1);
            commit(&mut db, run, &empty_brief(), &ledger, T1);
        }
        let state: String = db
            .query_row("SELECT state FROM work_tasks WHERE fingerprint IS NOT NULL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(state, "stale");
        assert!(board_tasks(&db).unwrap().is_empty(), "a stale unpinned task leaves the board");
    }

    #[test]
    fn a_run_whose_source_failed_ages_nothing() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);

        // Three runs where Slack failed. The task is absent from every brief, and none of
        // that is evidence it is gone.
        for run in ["run-2", "run-3", "run-4"] {
            start(&db, run);
            let mut ledger = RunLedger::new(run);
            ledger.record_failed("slack-1", "slack", "503");
            commit(&mut db, run, &empty_brief(), &ledger, T1);
        }
        let (state, misses): (String, i64) = db
            .query_row(
                "SELECT state,miss_count FROM work_tasks WHERE fingerprint IS NOT NULL",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(misses, 0);
        assert_eq!(state, "active");
        assert_eq!(board_tasks(&db).unwrap().len(), 1, "it is still on the board");
    }

    #[test]
    fn a_source_this_run_never_mentioned_ages_nothing() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);
        // A run that read some other connector entirely.
        start(&db, "run-2");
        let mut other = RunLedger::new("run-2");
        other.record_source("github-1", "github", wire::WorkSourceStatus::Succeeded, None, Some(T1.into()));
        commit(&mut db, "run-2", &empty_brief(), &other, T1);
        let misses: i64 = db
            .query_row("SELECT miss_count FROM work_tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(misses, 0);
    }

    // -----------------------------------------------------------------------
    // Human state survives
    // -----------------------------------------------------------------------

    #[test]
    fn a_users_decision_survives_a_reconciliation() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);
        let task_id: String = db.query_row("SELECT id FROM work_tasks", [], |row| row.get(0)).unwrap();

        // The user dismisses it and pins something they care about.
        let snapshot = read_task(&db, &task_id).unwrap().unwrap();
        let dismissed = apply_action(&snapshot, TaskAction::Dismiss, T0).unwrap();
        let pinned = apply_action(&dismissed, TaskAction::Pin, T0).unwrap();
        write_task_state(&db, &task_id, &pinned, None, T0).unwrap();

        // A later run mentions it again. The dismissal is the user talking, so it stands.
        start(&db, "run-2");
        let (second, again) = ledger("run-2", "1.1", T1);
        commit(&mut db, "run-2", &brief(&again, "Reply to Priya, again"), &second, T1);

        let after = read_task(&db, &task_id).unwrap().unwrap();
        assert_eq!(after.state, TaskState::Dismissed, "a later brief does not undo a no");
        assert!(after.pinned, "and the pin survives too");
        // The text is refreshed, because that part is the model's to update.
        let title: String = db.query_row("SELECT title FROM work_tasks", [], |row| row.get(0)).unwrap();
        assert_eq!(title, "Reply to Priya, again");
    }

    #[test]
    fn a_done_task_reopens_across_a_reconciliation_only_on_changed_evidence() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);
        let task_id: String = db.query_row("SELECT id FROM work_tasks", [], |row| row.get(0)).unwrap();

        let snapshot = read_task(&db, &task_id).unwrap().unwrap();
        let done = apply_action(&snapshot, TaskAction::Complete, T0).unwrap();
        write_task_state(&db, &task_id, &done, None, T0).unwrap();

        // The same evidence, even when collected by another run, is not news.
        start(&db, "run-2");
        let (second, again) = ledger("run-2", "1.1", T0);
        commit(&mut db, "run-2", &brief(&again, "Reply to Priya"), &second, T0);
        assert_eq!(read_task(&db, &task_id).unwrap().unwrap().state, TaskState::Done);

        // A changed result for the same canonical resource reopens it.
        start(&db, "run-3");
        let mut third = RunLedger::new("run-3");
        let newer = third
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-1",
                Some("T1/U1"),
                "call_1",
                "tool-digest",
                &json!({"ts": "1.1", "text": "changed", "permalink": "https://app.slack.com/archives/C1/p1"}),
                T1,
            )
            .unwrap();
        commit(&mut db, "run-3", &brief(&newer, "Reply to Priya"), &third, T1);
        assert_eq!(read_task(&db, &task_id).unwrap().unwrap().state, TaskState::Active);
    }

    #[test]
    fn a_pinned_stale_task_stays_on_the_board() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);
        let task_id: String = db.query_row("SELECT id FROM work_tasks", [], |row| row.get(0)).unwrap();
        let snapshot = read_task(&db, &task_id).unwrap().unwrap();
        let pinned = apply_action(&snapshot, TaskAction::Pin, T0).unwrap();
        write_task_state(&db, &task_id, &pinned, None, T0).unwrap();

        for run in ["run-2", "run-3"] {
            start(&db, run);
            let (ledger, _) = ledger(run, "9.9", T1);
            commit(&mut db, run, &empty_brief(), &ledger, T1);
        }
        assert_eq!(read_task(&db, &task_id).unwrap().unwrap().state, TaskState::Stale);
        assert_eq!(board_tasks(&db).unwrap().len(), 1, "a pin keeps a stale task in front of you");
    }

    // -----------------------------------------------------------------------
    // Identity and ephemerality
    // -----------------------------------------------------------------------

    #[test]
    fn two_accounts_holding_the_same_message_are_two_tasks() {
        let mut db = db();
        start(&db, "run-1");
        let mut ledger = RunLedger::new("run-1");
        let one = ledger
            .record_succeeded(ConnectorFamily::Slack, "slack-work", None, "c1", "d", &json!({"ts": "1.1"}), T0)
            .unwrap();
        let other = ledger
            .record_succeeded(ConnectorFamily::Slack, "slack-personal", None, "c2", "d", &json!({"ts": "1.1"}), T0)
            .unwrap();
        let brief = WorkBrief {
            version: 1,
            tasks: vec![
                BriefTask { rank: 1, title: "work".into(), why: "w".into(), confidence_bps: 100, primary_evidence: None, evidence: vec![one] },
                BriefTask { rank: 2, title: "personal".into(), why: "w".into(), confidence_bps: 100, primary_evidence: None, evidence: vec![other] },
            ],
        };
        assert_eq!(commit(&mut db, "run-1", &brief, &ledger, T0), 2);
        let fingerprints: i64 = db
            .query_row("SELECT COUNT(DISTINCT fingerprint) FROM work_tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fingerprints, 2, "the same native id in two accounts is two identities");
    }

    #[test]
    fn citation_order_and_duplicate_rows_do_not_change_task_identity() {
        let mut db = db();
        start(&db, "run-1");
        let mut first_ledger = RunLedger::new("run-1");
        let one = first_ledger.record_succeeded(
            ConnectorFamily::Slack, "slack-work", None, "c1", "d", &json!({"ts": "1.1"}), T0,
        ).unwrap();
        let two = first_ledger.record_succeeded(
            ConnectorFamily::Slack, "slack-work", None, "c2", "d", &json!({"ts": "2.2"}), T0,
        ).unwrap();
        let first_brief = WorkBrief { version: 1, tasks: vec![
            BriefTask { rank: 2, title: "duplicate".into(), why: "w".into(), confidence_bps: 100, primary_evidence: Some(one.clone()), evidence: vec![two.clone(), one.clone()] },
            BriefTask { rank: 1, title: "primary".into(), why: "w".into(), confidence_bps: 100, primary_evidence: Some(one.clone()), evidence: vec![one.clone(), two.clone()] },
        ] };
        assert_eq!(commit(&mut db, "run-1", &first_brief, &first_ledger, T0), 1);
        let selected_title: String = db.query_row("SELECT title FROM work_tasks", [], |row| row.get(0)).unwrap();
        assert_eq!(selected_title, "primary", "the lowest rank wins regardless of array order");
        let first_id: String = db.query_row("SELECT id FROM work_tasks", [], |row| row.get(0)).unwrap();

        start(&db, "run-2");
        let mut second_ledger = RunLedger::new("run-2");
        let one = second_ledger.record_succeeded(
            ConnectorFamily::Slack, "slack-work", None, "c1", "d", &json!({"ts": "1.1"}), T1,
        ).unwrap();
        let two = second_ledger.record_succeeded(
            ConnectorFamily::Slack, "slack-work", None, "c2", "d", &json!({"ts": "2.2"}), T1,
        ).unwrap();
        let reversed = WorkBrief { version: 1, tasks: vec![BriefTask {
            rank: 1, title: "same task".into(), why: "w".into(), confidence_bps: 100,
            primary_evidence: Some(one.clone()),
            evidence: vec![two, one],
        }] };
        assert_eq!(commit(&mut db, "run-2", &reversed, &second_ledger, T1), 1);
        let second_id: String = db.query_row("SELECT id FROM work_tasks", [], |row| row.get(0)).unwrap();
        assert_eq!(second_id, first_id);
    }

    #[test]
    fn an_ephemeral_task_is_replaced_by_the_next_committed_run() {
        let mut db = db();
        start(&db, "run-1");
        let mut ledger = RunLedger::new("run-1");
        let reference = ledger
            .record_succeeded(
                ConnectorFamily::Slack,
                "slack-work",
                None,
                "c1",
                "d",
                &json!({"text": "a successful result with no stable provider id"}),
                T0,
            )
            .unwrap();
        commit(&mut db, "run-1", &brief(&reference, "Reply"), &ledger, T0);
        let ephemeral: (i64, i64, Option<String>) = db
            .query_row("SELECT COUNT(*),SUM(ephemeral),fingerprint FROM work_tasks", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(ephemeral, (1, 1, None));

        start(&db, "run-2");
        let next = RunLedger::new("run-2");
        commit(&mut db, "run-2", &empty_brief(), &next, T1);
        assert_eq!(board_tasks(&db).unwrap().len(), 0);
    }

    #[test]
    fn a_task_whose_citation_does_not_resolve_is_skipped_rather_than_stored_blind() {
        // The parser refuses such a brief, so this is belt: if one ever arrived, it must not
        // become a task with borrowed provenance.
        let mut db = db();
        start(&db, "run-1");
        let (ledger, _) = ledger("run-1", "1.1", T0);
        let fabricated = brief("run-1:ev-99", "Invented");
        assert_eq!(commit(&mut db, "run-1", &fabricated, &ledger, T0), 0);
        assert!(board_tasks(&db).unwrap().is_empty());
    }
    #[test]
    fn integration_activity_survives_commit_and_rereading_does_not_renew_it() {
        let mut db = db();
        let source_time = chrono::Utc::now() - chrono::Duration::hours(1);
        let ts = format!("{}.000000", source_time.timestamp());
        start(&db, "dated-run");
        let (ledger, reference) = ledger("dated-run", &ts, T0);
        commit(&mut db, "dated-run", &brief(&reference, "Recent message"), &ledger, T0);
        let board = crate::work::board(&db).unwrap();
        assert_eq!(board.tasks.len(), 1);
        assert_eq!(board.tasks[0].evidence_observed_at.as_deref(), Some(T0));
        assert_eq!(board.tasks[0].source_activity_at, ledger.entries()[0].source_activity_at);
        let stored = crate::work_brief_store::read_evidence(&db, "dated-run").unwrap();
        assert_eq!(stored[0].source_activity_at, ledger.entries()[0].source_activity_at);
        db.execute("UPDATE work_tasks SET source_activity_at='2020-01-01T00:00:00Z',pinned=1,updated_at=?1,evidence_observed_at=?1", [chrono::Utc::now().to_rfc3339()]).unwrap();
        assert!(crate::work::board(&db).unwrap().tasks.is_empty());
    }

}
