//! The attempt record and the sweep. The sweep asks the harness to compact
//! through the one dispatch path of the manual `/compact`, and the attempt
//! row tells what happened after.

use std::collections::HashSet;
use std::sync::{Arc, PoisonError};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use super::rule::{self, Decision, FireReason, Suppression};
use super::{
    parse_time, span, ChatState, DueMark, Fill, Inputs, OpenAttempt, Outcome, Probe, Reading,
    Receipt, SessionRow, Settlement, Snapshot, IDLE_STATUSES, NEXT_READING_LIMIT, SETTLE_LIMIT,
    THRESHOLD_TOKENS, TOOL_SCAN_CAP,
};
use crate::adapters::NativeCompaction;
use crate::live_turn::{forward_native_compaction, CompactOrigin, Forwarded};
use crate::runtime::BridgeCore;

/// The table and its two partial indexes. No schema version, so an open
/// makes no backup copy of the store and an older Bridge still opens it.
const STORE: &str = "CREATE TABLE IF NOT EXISTS early_compaction_attempts(
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    harness TEXT NOT NULL,
    model TEXT,
    launch_started_at TEXT NOT NULL,
    provider_session_id TEXT,
    trigger_turn_id TEXT,
    trigger_reading_id INTEGER NOT NULL,
    trigger_used_tokens INTEGER NOT NULL,
    trigger_window_tokens INTEGER NOT NULL,
    threshold_tokens INTEGER NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL,
    dispatched_at TEXT,
    outcome TEXT CHECK (outcome IN ('compacted','ended_without_boundary','failed','unsupported','interrupted','unsettled')),
    boundary_pre_tokens INTEGER,
    boundary_post_tokens INTEGER,
    compaction_turn_id TEXT,
    next_reading_id INTEGER,
    next_used_tokens INTEGER,
    error TEXT,
    settled_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_early_compaction_attempts_open
    ON early_compaction_attempts(session_id) WHERE outcome IS NULL;
CREATE INDEX IF NOT EXISTS idx_attention_events_open_needs_you
    ON attention_events(session_id) WHERE kind='needs_you' AND audience='human' AND resolved_at IS NULL;";

/// A launch in one of these statuses has ended, so its attempt cannot end.
const ENDED_STATUSES: [&str; 4] = ["stopped", "failed", "completed", "cancelled"];
const TOOL_STARTS: [&str; 3] = ["tool.started", "command.started", "file_change.started"];
const TOOL_COMPLETIONS: [&str; 3] = ["tool.completed", "command.completed", "file_change.completed"];

/// Whether the chat is still on the launch of a compacted attempt, and the
/// first reading after it. The reading comes after the boundary, and the
/// reading of the compaction turn itself never counts. Both times come from
/// `to_rfc3339` in UTC, so their text order is their time order.
const NEXT_READING: &str = "SELECT s.started_at IS a.launch_started_at, r.id, r.used_tokens
    FROM early_compaction_attempts a
    JOIN sessions s ON s.id=a.session_id
    LEFT JOIN context_readings r ON r.session_id=a.session_id AND r.id>a.trigger_reading_id
      AND r.created_at>=a.settled_at
      AND r.harness=a.harness AND r.model IS a.model AND r.provider_session_id IS a.provider_session_id
      AND (a.compaction_turn_id IS NULL OR r.turn_id IS NOT a.compaction_turn_id)
    WHERE a.id=?1 AND a.outcome='compacted' AND a.next_reading_id IS NULL
    ORDER BY r.id IS NULL, r.id LIMIT 1";

pub(crate) fn install_store(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(STORE)
}

/// The frame path calls this after its lock. A due mark goes to memory. A
/// settlement runs one UPDATE by primary key, and only for an open attempt.
pub(crate) fn accept(core: &BridgeCore, receipt: Receipt, now: DateTime<Utc>) {
    match receipt {
        Receipt::Due(mark) => core.early_compaction.accept(Receipt::Due(mark)),
        Receipt::Settle(settlement) => {
            settle(core, &settlement, settlement.error.as_deref(), now);
        }
    }
}

/// Close the open attempt in memory first, so only one caller writes the
/// row. Settlement ignores the switch. Returns whether this call closed it.
fn settle(core: &BridgeCore, settlement: &Settlement, error: Option<&str>, now: DateTime<Utc>) -> bool {
    let tracker = &core.early_compaction;
    let Some(open) = tracker.settle(&settlement.session_id, &settlement.attempt_id, settlement.outcome) else {
        return false;
    };
    let written = core.db.lock().unwrap_or_else(PoisonError::into_inner).execute(
        "UPDATE early_compaction_attempts
         SET outcome=?2,boundary_pre_tokens=?3,boundary_post_tokens=?4,compaction_turn_id=?5,error=?6,settled_at=?7
         WHERE id=?1 AND outcome IS NULL",
        params![
            settlement.attempt_id,
            settlement.outcome.as_str(),
            settlement.boundary_pre_tokens,
            settlement.boundary_post_tokens,
            settlement.compaction_turn_id,
            error,
            now.to_rfc3339(),
        ],
    );
    if let Err(error) = written {
        eprintln!("[early-compaction] settle {}: {error}", settlement.attempt_id);
    }
    if settlement.outcome == Outcome::Compacted {
        tracker.add_fill(
            &settlement.session_id,
            Fill { attempt_id: open.attempt_id, launch: open.launch, until: now + span(NEXT_READING_LIMIT) },
        );
    }
    true
}

fn closing(session_id: &str, attempt_id: &str, outcome: Outcome) -> Settlement {
    Settlement {
        session_id: session_id.to_owned(),
        attempt_id: attempt_id.to_owned(),
        outcome,
        boundary_pre_tokens: None,
        boundary_post_tokens: None,
        compaction_turn_id: None,
        error: None,
    }
}

/// A row that a stopped process left open can get no terminal signal. The
/// maintenance loop calls this once, before its first tick. It sends nothing.
pub(crate) fn settle_at_startup(core: &BridgeCore, now: DateTime<Utc>) {
    let settled = core.db.lock().unwrap_or_else(PoisonError::into_inner).execute(
        "UPDATE early_compaction_attempts SET outcome='interrupted',settled_at=?1 WHERE outcome IS NULL",
        params![now.to_rfc3339()],
    );
    if let Err(error) = settled {
        eprintln!("[early-compaction] startup settle: {error}");
    }
}

/// One pass of the trigger, from the maintenance loop. The settle step
/// always runs. With the switch off, the pass forgets each due mark and runs
/// no SQL when no attempt waits.
pub(crate) fn sweep(core: &Arc<BridgeCore>, now: DateTime<Utc>) {
    settle_open_attempts(core, now);
    fill_next_readings(core, now);
    let tracker = &core.early_compaction;
    if !tracker.enabled() {
        tracker.clear_marks();
        return;
    }
    for mark in tracker.marks() {
        consider(core, &mark, now);
    }
}

/// A launch change, an ended status or `SETTLE_LIMIT` closes an attempt that
/// got no boundary and no turn end.
fn settle_open_attempts(core: &BridgeCore, now: DateTime<Utc>) {
    for (session_id, open) in core.early_compaction.open_attempts() {
        let row = core.db.lock().unwrap_or_else(PoisonError::into_inner).query_row(
            "SELECT a.created_at,s.status,s.started_at,s.provider_session_id
             FROM early_compaction_attempts a LEFT JOIN sessions s ON s.id=a.session_id
             WHERE a.id=?1 AND a.outcome IS NULL",
            params![open.attempt_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional();
        let (created_at, status, started_at, provider_session_id) = match row {
            Ok(Some(row)) => row,
            Ok(None) => {
                core.early_compaction.forget(&session_id, &open.attempt_id);
                continue;
            }
            Err(error) => {
                eprintln!("[early-compaction] read attempt {}: {error}", open.attempt_id);
                continue;
            }
        };
        let current = started_at.as_deref() == Some(open.launch.started_at.as_str())
            && provider_session_id.as_deref() == Some(open.launch.provider_session_id.as_str());
        let ended = status.as_deref().is_none_or(|status| ENDED_STATUSES.contains(&status));
        let expired = parse_time(&created_at)
            .is_none_or(|created| now.signed_duration_since(created) > span(SETTLE_LIMIT));
        let outcome = if !current || ended {
            Outcome::Interrupted
        } else if expired {
            Outcome::Unsettled
        } else {
            continue;
        };
        settle(core, &closing(&session_id, &open.attempt_id, outcome), None, now);
    }
}

/// The window after a compacted attempt, on the same base as the trigger
/// reading. A next reading at or above the threshold stops the launch.
fn fill_next_readings(core: &BridgeCore, now: DateTime<Utc>) {
    let tracker = &core.early_compaction;
    for (session_id, fill) in tracker.fills() {
        if now > fill.until {
            tracker.drop_fill(&session_id, &fill.attempt_id);
            continue;
        }
        let found = {
            let db = core.db.lock().unwrap_or_else(PoisonError::into_inner);
            let next = db
                .query_row(NEXT_READING, params![fill.attempt_id], |row| {
                    Ok((row.get::<_, bool>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, Option<i64>>(2)?))
                })
                .optional();
            match next {
                Ok(Some((true, Some(reading_id), Some(used_tokens)))) => db
                    .execute(
                        "UPDATE early_compaction_attempts SET next_reading_id=?2,next_used_tokens=?3
                         WHERE id=?1 AND next_reading_id IS NULL",
                        params![fill.attempt_id, reading_id, used_tokens],
                    )
                    .map(|_| (true, Some(used_tokens))),
                Ok(Some((current, _, _))) => Ok((current, None)),
                Ok(None) => Ok((true, None)),
                Err(error) => Err(error),
            }
        };
        match found {
            Ok((_, Some(used_tokens))) => {
                tracker.record_next_reading(&session_id, &fill.launch, used_tokens);
                tracker.drop_fill(&session_id, &fill.attempt_id);
            }
            // The chat left the launch of the attempt, so no next reading can come.
            Ok((false, None)) => tracker.drop_fill(&session_id, &fill.attempt_id),
            Ok((true, None)) => {}
            Err(error) => eprintln!("[early-compaction] next reading {}: {error}", fill.attempt_id),
        }
    }
}

/// One due mark: memory checks, the capability probe, the reservation,
/// one read of the chat, the rule, and at most one dispatch.
fn consider(core: &Arc<BridgeCore>, mark: &DueMark, now: DateTime<Utc>) {
    let tracker = &core.early_compaction;
    let Some(memory) = tracker.memory_inputs(&mark.session_id, now) else {
        return;
    };
    if let Some(suppression) = rule::precheck(&memory) {
        tracker.apply(&memory.mark, Decision::Suppress(suppression));
        return;
    }
    let session_id = memory.mark.session_id.clone();
    // Off every lock: for Codex this can read the app-server schema.
    let supported = core.adapter_registry.supports_native_compaction(&memory.mark.harness);
    // A send or a save in flight holds the lease. The mark stays.
    let Ok(_input) = core.input_activity.try_write() else {
        return;
    };
    let Ok(_lifecycle) = core.claim_session_lifecycle(&session_id, "early compaction") else {
        return;
    };
    let read = read_chat(&core.db.lock().unwrap_or_else(PoisonError::into_inner), &session_id);
    let (enabled, opted_out, session, state, reading) = match read {
        Ok(Some(read)) => read,
        Ok(None) => {
            // The chat is gone, so the mark can never bind.
            tracker.apply(&memory.mark, Decision::Suppress(Suppression::LaunchMismatch));
            return;
        }
        Err(error) => {
            eprintln!("[early-compaction] read chat {session_id}: {error}");
            return;
        }
    };
    let (has_runtime, native_compaction) = runtime_facts(core, &session_id, supported);
    let inputs = Inputs {
        memory,
        snapshot: Snapshot { enabled, opted_out, session, native_compaction, has_runtime, state, reading },
    };
    let decision = rule::decide(&inputs);
    // A boundary or a newer turn end can take the mark after the memory
    // check. Only the call that removes the mark can fire for it.
    let taken = tracker.apply(&inputs.memory.mark, decision);
    if let (Decision::Fire(reason), true) = (decision, taken) {
        fire(core, &inputs, reason, now);
    }
}

type ChatRead = (bool, bool, SessionRow, ChatState, Option<Reading>);

/// Each read of the sweep, in one hold of the database lock. Each statement
/// uses a key, a partial index or a cap.
fn read_chat(db: &Connection, session_id: &str) -> rusqlite::Result<Option<ChatRead>> {
    let Some(session) = session_row(db, session_id)? else {
        return Ok(None);
    };
    // A failed read of the switch reads as off, and of the opt-out as on.
    let enabled = crate::early_compaction_settings::global_enabled(db).unwrap_or(false);
    let opted_out = crate::early_compaction_settings::opted_out(db, session_id).unwrap_or(true);
    let state = chat_state(db, session_id, &session.status);
    let reading = latest_reading(db, session_id).unwrap_or(None);
    Ok(Some((enabled, opted_out, session, state, reading)))
}

pub(crate) fn session_row(db: &Connection, session_id: &str) -> rusqlite::Result<Option<SessionRow>> {
    db.query_row(
        "SELECT COALESCE(kind,''),parent_session_id,COALESCE(depth,0),harness,model,COALESCE(status,''),
                active_turn_id,started_at,provider_session_id
         FROM sessions WHERE id=?1",
        params![session_id],
        |row| {
            Ok(SessionRow {
                kind: row.get(0)?,
                parent_session_id: row.get(1)?,
                depth: row.get(2)?,
                harness: row.get(3)?,
                model: row.get(4)?,
                status: row.get(5)?,
                active_turn_id: row.get(6)?,
                started_at: row.get(7)?,
                provider_session_id: row.get(8)?,
            })
        },
    )
    .optional()
}

/// The latest reading, by the index `context_readings_session`.
fn latest_reading(db: &Connection, session_id: &str) -> rusqlite::Result<Option<Reading>> {
    let row = db
        .query_row(
            "SELECT id,turn_id,harness,model,provider_session_id,used_tokens,window_tokens,source,created_at
             FROM context_readings WHERE session_id=?1 ORDER BY id DESC LIMIT 1",
            params![session_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .optional()?;
    // A time that does not parse can bind to no mark.
    Ok(row.and_then(|(id, turn_id, harness, model, provider_session_id, used_tokens, window_tokens, source, created_at)| {
        Some(Reading {
            id,
            turn_id,
            harness,
            model,
            provider_session_id,
            used_tokens,
            window_tokens,
            source,
            created_at: parse_time(&created_at)?,
        })
    }))
}

/// The activity that holds a compaction back. A failed query or a scan
/// past its cap is `Unknown`, never `Clear`.
pub(crate) fn chat_state(db: &Connection, session_id: &str, status: &str) -> ChatState {
    let (children, queue) = match crate::runtime_budget::activity(db, session_id, &IDLE_STATUSES) {
        Ok(Some(activity)) => (
            probe(activity.worker_result || activity.subagent || activity.background_task),
            probe(activity.queued_input),
        ),
        Ok(None) | Err(_) => (Probe::Unknown, Probe::Unknown),
    };
    ChatState { approval: approval_probe(db, session_id, status), tool: tool_probe(db, session_id), children, queue }
}

fn probe(present: bool) -> Probe {
    if present {
        Probe::Present
    } else {
        Probe::Clear
    }
}

/// The attention module closes `needs_you` rows for each request id, so an
/// open row is an open request. The partial index holds open rows only.
fn approval_probe(db: &Connection, session_id: &str, status: &str) -> Probe {
    if status == "waiting" {
        return Probe::Present;
    }
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM attention_events
         WHERE session_id=?1 AND kind='needs_you' AND audience='human' AND resolved_at IS NULL)",
        params![session_id],
        |row| row.get::<_, bool>(0),
    )
    .map_or(Probe::Unknown, probe)
}

/// Walk the newest entries back to the last `turn.started`. A start with no
/// later completion of the same item is an open tool. Only a turn start
/// inside the cap makes the walk a proof.
fn tool_probe(db: &Connection, session_id: &str) -> Probe {
    let cap = i64::try_from(TOOL_SCAN_CAP).unwrap_or(i64::MAX);
    let Ok(mut statement) = db.prepare_cached(
        "SELECT kind,provider_event_id FROM session_entries WHERE session_id=?1 ORDER BY sequence DESC LIMIT ?2",
    ) else {
        return Probe::Unknown;
    };
    let Ok(rows) = statement.query_map(params![session_id, cap], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    }) else {
        return Probe::Unknown;
    };
    let mut completed: HashSet<String> = HashSet::new();
    let mut open = false;
    for row in rows {
        let Ok((kind, item)) = row else {
            return Probe::Unknown;
        };
        if kind == "turn.started" {
            return probe(open);
        }
        if TOOL_COMPLETIONS.contains(&kind.as_str()) {
            completed.extend(item);
        } else if TOOL_STARTS.contains(&kind.as_str())
            && item.as_ref().is_none_or(|item| !completed.contains(item))
        {
            open = true;
        }
    }
    // No turn start within the cap, so the walk proves nothing.
    Probe::Unknown
}

/// The runtime refines the static answer of the registry. Bridge never
/// starts a process to compact, so no runtime is `no_runtime`.
fn runtime_facts(core: &BridgeCore, session_id: &str, supported: bool) -> (bool, NativeCompaction) {
    let adapters = core.adapters.lock().unwrap_or_else(PoisonError::into_inner);
    match adapters.get(session_id) {
        Some(runtime) if supported => (true, runtime.native_compaction()),
        Some(_) => (true, NativeCompaction::Unsupported),
        // Only the static answer exists. Each supported value gives the same
        // decision here: `no_runtime`.
        None if supported => (false, NativeCompaction::WholeConversation),
        None => (false, NativeCompaction::Unsupported),
    }
}

/// The row, then the open entry, then the dispatch. So a boundary that
/// comes before the write of `dispatched_at` finds its attempt.
fn fire(core: &Arc<BridgeCore>, inputs: &Inputs, reason: FireReason, now: DateTime<Utc>) {
    let mark = &inputs.memory.mark;
    let session = &inputs.snapshot.session;
    let (Some(launch), Some(reading)) = (mark.launch.as_ref(), inputs.snapshot.reading.as_ref()) else {
        return;
    };
    let attempt_id = Uuid::new_v4().to_string();
    let inserted = core.db.lock().unwrap_or_else(PoisonError::into_inner).execute(
        "INSERT INTO early_compaction_attempts(id,session_id,harness,model,launch_started_at,provider_session_id,
             trigger_turn_id,trigger_reading_id,trigger_used_tokens,trigger_window_tokens,threshold_tokens,reason,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        params![
            attempt_id,
            mark.session_id,
            session.harness,
            session.model,
            launch.started_at,
            launch.provider_session_id,
            mark.turn_id,
            reading.id,
            reading.used_tokens,
            reading.window_tokens,
            THRESHOLD_TOKENS,
            reason.as_str(),
            now.to_rfc3339(),
        ],
    );
    if let Err(error) = inserted {
        eprintln!("[early-compaction] insert attempt for {}: {error}", mark.session_id);
        return;
    }
    core.early_compaction.open(&mark.session_id, OpenAttempt { attempt_id: attempt_id.clone(), launch: launch.clone() });
    match forward_native_compaction(core, &mark.session_id, &session.harness, None, CompactOrigin::Early) {
        Ok(Forwarded::Yes) => {
            let written = core.db.lock().unwrap_or_else(PoisonError::into_inner).execute(
                "UPDATE early_compaction_attempts SET dispatched_at=?2 WHERE id=?1",
                params![attempt_id, now.to_rfc3339()],
            );
            if let Err(error) = written {
                eprintln!("[early-compaction] dispatched {attempt_id}: {error}");
            }
        }
        Ok(Forwarded::Unsupported) => {
            settle(core, &closing(&mark.session_id, &attempt_id, Outcome::Unsupported), None, now);
        }
        // The status stays as it is: the harness may still be busy.
        Err(error) => {
            settle(core, &closing(&mark.session_id, &attempt_id, Outcome::Failed), Some(&error.to_string()), now);
        }
    }
}
