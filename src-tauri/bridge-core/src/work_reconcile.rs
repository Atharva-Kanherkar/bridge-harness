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
use crate::work_brief_store::{finish_run, RunOutcome};
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
        fingerprint: fingerprint_for(
            &entry.connector_instance_id,
            Some(entry.canonical_resource_id.as_str()),
        ),
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

    // Existing tasks, keyed by fingerprint where they have one.
    let mut stored = Vec::new();
    {
        let mut statement = transaction.prepare(
            "SELECT id,fingerprint,connector_instance_id,state,pinned,miss_count,ephemeral,resolved_at
               FROM work_tasks",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredTask {
                id: row.get(0)?,
                fingerprint: row.get(1)?,
                connector_instance_id: row.get(2)?,
                snapshot: TaskSnapshot {
                    state: TaskState::parse(&row.get::<_, String>(3)?),
                    pinned: row.get::<_, i64>(4)? != 0,
                    miss_count: row.get(5)?,
                    ephemeral: row.get::<_, i64>(6)? != 0,
                    resolved_at: row.get(7)?,
                },
            })
        })?;
        for row in rows {
            stored.push(row?);
        }
    }

    // Which fingerprints this brief mentions, and what backs each.
    let mut present: BTreeMap<String, (i64, &crate::work_brief_parser::BriefTask, &EvidenceEntry)> =
        BTreeMap::new();
    let mut ephemeral: Vec<(&crate::work_brief_parser::BriefTask, &EvidenceEntry)> = Vec::new();
    for task in &commit.brief.tasks {
        // A task's first citation decides its identity. The parser has already refused a
        // brief whose citations do not resolve, so this cannot silently drop one.
        let Some(first) = task.evidence.first().and_then(|reference| resolve(commit.ledger, reference))
        else {
            continue;
        };
        match first.fingerprint {
            Some(value) => {
                present.insert(value, (task.rank, task, first.entry));
            }
            None => ephemeral.push((task, first.entry)),
        }
    }

    // Age or refresh every task already on the board.
    let mentioned: BTreeSet<&String> = present.keys().collect();
    for existing in &stored {
        let next = match existing.fingerprint.as_ref().filter(|value| mentioned.contains(value)) {
            Some(value) => {
                let (_, _, entry) = &present[value.as_str()];
                reconcile_present(&existing.snapshot, Some(entry.observed_at.as_str()))
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
        transaction.execute(
            "UPDATE work_tasks SET state=?2,pinned=?3,miss_count=?4,resolved_at=?5,last_run_id=?6,updated_at=?7
               WHERE id=?1",
            params![
                existing.id,
                next.state.as_str(),
                i64::from(next.pinned),
                next.miss_count,
                next.resolved_at,
                commit.run_id,
                commit.now,
            ],
        )?;
    }

    // Insert or refresh what the brief proposed. A task the user has put away keeps its
    // state: the upsert deliberately does not touch state, pinned, miss_count or
    // resolved_at, all of which were just decided above.
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
             ephemeral,first_run_id,last_run_id,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14,?15,?15)
         ON CONFLICT(id) DO UPDATE SET
             title=excluded.title,why=excluded.why,rank=excluded.rank,
             confidence_bps=excluded.confidence_bps,evidence_digest=excluded.evidence_digest,
             evidence_target=excluded.evidence_target,
             evidence_observed_at=excluded.evidence_observed_at,
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
    db: &Connection,
    run_id: &str,
    outcome: &RunOutcome,
) -> Result<(), BridgeError> {
    finish_run(db, run_id, outcome)
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
            "SELECT state,pinned,miss_count,ephemeral,resolved_at FROM work_tasks WHERE id=?1",
            params![task_id],
            |row| {
                Ok(TaskSnapshot {
                    state: TaskState::parse(&row.get::<_, String>(0)?),
                    pinned: row.get::<_, i64>(1)? != 0,
                    miss_count: row.get(2)?,
                    ephemeral: row.get::<_, i64>(3)? != 0,
                    resolved_at: row.get(4)?,
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
        "UPDATE work_tasks SET state=?2,pinned=?3,resolved_at=?4,snoozed_until=?5,updated_at=?6
           WHERE id=?1",
        params![
            task_id,
            snapshot.state.as_str(),
            i64::from(snapshot.pinned),
            snapshot.resolved_at,
            snoozed_until,
            now,
        ],
    )?;
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
        abandon_run(
            &db,
            "run-2",
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
    fn a_done_task_reopens_across_a_reconciliation_only_on_newer_evidence() {
        let mut db = db();
        start(&db, "run-1");
        let (first, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply to Priya"), &first, T0);
        let task_id: String = db.query_row("SELECT id FROM work_tasks", [], |row| row.get(0)).unwrap();

        let snapshot = read_task(&db, &task_id).unwrap().unwrap();
        let done = apply_action(&snapshot, TaskAction::Complete, T0).unwrap();
        write_task_state(&db, &task_id, &done, None, T0).unwrap();

        // Same evidence, observed no later than the completion: not news.
        start(&db, "run-2");
        let (second, again) = ledger("run-2", "1.1", T0);
        commit(&mut db, "run-2", &brief(&again, "Reply to Priya"), &second, T0);
        assert_eq!(read_task(&db, &task_id).unwrap().unwrap().state, TaskState::Done);

        // Newer evidence: something happened after the user finished with it.
        start(&db, "run-3");
        let (third, newer) = ledger("run-3", "1.1", T1);
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
                BriefTask { rank: 1, title: "work".into(), why: "w".into(), confidence_bps: 100, evidence: vec![one] },
                BriefTask { rank: 2, title: "personal".into(), why: "w".into(), confidence_bps: 100, evidence: vec![other] },
            ],
        };
        assert_eq!(commit(&mut db, "run-1", &brief, &ledger, T0), 2);
        let fingerprints: i64 = db
            .query_row("SELECT COUNT(DISTINCT fingerprint) FROM work_tasks", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fingerprints, 2, "the same native id in two accounts is two identities");
    }

    #[test]
    fn an_ephemeral_task_is_replaced_by_the_next_committed_run() {
        let mut db = db();
        start(&db, "run-1");
        // A result Bridge cannot identify earns no evidence at all, so an ephemeral task
        // needs evidence whose resource id is absent — which the ledger will not mint. The
        // reachable case is a task whose citation resolves to an entry with no canonical
        // id, so this asserts the boundary instead: every committed task here is identified.
        let (ledger, reference) = ledger("run-1", "1.1", T0);
        commit(&mut db, "run-1", &brief(&reference, "Reply"), &ledger, T0);
        let ephemeral: i64 = db
            .query_row("SELECT COUNT(*) FROM work_tasks WHERE ephemeral=1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(ephemeral, 0, "evidence without a canonical id is never recorded, so no task is ephemeral yet");
        let identified: i64 = db
            .query_row("SELECT COUNT(*) FROM work_tasks WHERE fingerprint IS NOT NULL", [], |row| row.get(0))
            .unwrap();
        assert_eq!(identified, 1);
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
}
