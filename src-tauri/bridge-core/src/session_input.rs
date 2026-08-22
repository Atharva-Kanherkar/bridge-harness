//! The active-turn input contract.
//!
//! A user typing into a working agent is supervision, not a race. Bridge has
//! exactly three answers, and this module owns the durable state and the pure
//! policy behind them:
//!
//! - nothing is running → start a normal turn;
//! - a turn is running and the provider takes input natively → steer it;
//! - a turn is running and the provider cannot → queue the text durably and
//!   deliver it at the next phase boundary, exactly once.
//!
//! The queue is SQLite, not a field on a runtime struct, because a reconnect or
//! a daemon restart must not lose what the user typed — and a replayed drain
//! must not deliver it twice.

use chrono::Utc;
use rusqlite::{params, Connection};
use uuid::Uuid;

pub use bridge_protocol::messages::InputDisposition;

use crate::{slash, BridgeError};

/// Waiting for a phase boundary.
pub const STATE_QUEUED: &str = "queued";
/// Claimed by one drain and being written to the provider right now. The
/// compare-and-swap into this state is what makes delivery exactly-once.
pub const STATE_CLAIMING: &str = "claiming";
pub const STATE_DELIVERED: &str = "delivered";
/// Claimed but never confirmed — the process died mid-write. Redelivering could
/// duplicate and dropping silently could lose, so the row is surfaced instead.
pub const STATE_ABANDONED: &str = "abandoned";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedInput {
    pub id: String,
    pub sequence: i64,
    pub session_id: String,
    /// What goes to the provider: sanitized, slash-expanded, with any `@file`
    /// context already appended at submission time.
    pub provider_text: String,
    /// What the transcript shows the user.
    pub display_text: String,
}

/// What Bridge should do with submitted input, decided from durable state and
/// advertised capability alone.
///
/// Pure on purpose: the same decision has to be reachable from a test without a
/// live provider, and every caller has to get the same answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRoute {
    NewTurn,
    Steer,
    Queue,
}

impl InputRoute {
    pub const fn disposition(self) -> InputDisposition {
        match self {
            Self::NewTurn => InputDisposition::StartedNewTurn,
            Self::Steer => InputDisposition::SteeredActiveTurn,
            Self::Queue => InputDisposition::QueuedForPhaseBoundary,
        }
    }
}

/// Never "send another turn and hope": a provider that cannot take input
/// mid-turn gets the queue, not a second `turn/start`.
pub const fn route(turn_active: bool, adapter_supports_steering: bool) -> InputRoute {
    if !turn_active {
        InputRoute::NewTurn
    } else if adapter_supports_steering {
        InputRoute::Steer
    } else {
        InputRoute::Queue
    }
}

/// Why a worker would not take what a human typed.
///
/// A worker is not a chat, but it is also not deaf. Only three states make user
/// input meaningless or actively harmful; in every other state the worker routes
/// through the same table as any other session. Each variant names itself so the
/// person typing learns why their words went nowhere instead of reading one
/// blanket refusal about the worker pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerSteerRefusal {
    /// The typed result is already reported. Nothing said now can change it, and
    /// pretending otherwise would leave the user believing they redirected work
    /// that had already been handed to the orchestrator.
    AlreadyReported,
    /// Bridge's own checkpoint turn owns the provider. User text landing inside
    /// it would be folded into the checkpoint summary rather than the objective.
    Checkpointing,
    /// No live provider process. A worker is launched by the policy-controlled
    /// pool, never resumed as a side effect of somebody sending a message.
    NotRunning,
}

impl WorkerSteerRefusal {
    pub const fn message(self) -> &'static str {
        match self {
            Self::AlreadyReported => {
                "This worker already reported its typed result, so guidance cannot reach it. Ask the orchestrator to delegate a follow-up."
            }
            Self::Checkpointing => {
                "This worker is checkpointing its context. Steer it once the checkpoint finishes."
            }
            Self::NotRunning => {
                "This worker is not running. Workers are started by the worker pool, not by a message."
            }
        }
    }
}

/// Whether a worker can take user guidance at all, decided from durable state
/// and adapter liveness alone.
///
/// Separate from [`route`] on purpose: this answers "may these words reach this
/// worker", and `route` answers "how". Keeping them apart is what stops a
/// worker-specific routing table from growing beside the general one.
pub fn worker_steer_gate(
    result_status: &str,
    lifecycle_state: &str,
    has_live_runtime: bool,
) -> Result<(), WorkerSteerRefusal> {
    if result_status == "reported" {
        return Err(WorkerSteerRefusal::AlreadyReported);
    }
    if lifecycle_state == "checkpointing" {
        return Err(WorkerSteerRefusal::Checkpointing);
    }
    if !has_live_runtime {
        return Err(WorkerSteerRefusal::NotRunning);
    }
    Ok(())
}

/// Session-control commands rewrite the session itself — they drop the provider
/// process, start a checkpoint turn, or reset history. None of that is safe to
/// do underneath a running turn, so they need an idle session.
///
/// This lives beside the routing decision so new-turn, steered, and queued input
/// all get the same answer from the same place.
pub const fn requires_idle_session(dispatch: &slash::SlashDispatch) -> bool {
    matches!(
        dispatch,
        slash::SlashDispatch::Usage
            | slash::SlashDispatch::Compact { .. }
            | slash::SlashDispatch::Clear
            | slash::SlashDispatch::Unsupported { .. }
    )
}

pub fn enqueue(
    db: &Connection,
    session_id: &str,
    provider_text: &str,
    display_text: &str,
) -> Result<QueuedInput, BridgeError> {
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO queued_session_input
            (id,session_id,provider_text,display_text,state,created_at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            id,
            session_id,
            provider_text,
            display_text,
            STATE_QUEUED,
            Utc::now().to_rfc3339()
        ],
    )?;
    let sequence = db.query_row(
        "SELECT sequence FROM queued_session_input WHERE id=?1",
        params![id],
        |row| row.get(0),
    )?;
    Ok(QueuedInput {
        id,
        sequence,
        session_id: session_id.to_owned(),
        provider_text: provider_text.to_owned(),
        display_text: display_text.to_owned(),
    })
}

/// The oldest waiting row for a session, in submission order.
pub fn next_queued(db: &Connection, session_id: &str) -> Result<Option<QueuedInput>, BridgeError> {
    let row = db
        .query_row(
            "SELECT id,sequence,session_id,provider_text,display_text
             FROM queued_session_input
             WHERE session_id=?1 AND state=?2
             ORDER BY sequence LIMIT 1",
            params![session_id, STATE_QUEUED],
            |row| {
                Ok(QueuedInput {
                    id: row.get(0)?,
                    sequence: row.get(1)?,
                    session_id: row.get(2)?,
                    provider_text: row.get(3)?,
                    display_text: row.get(4)?,
                })
            },
        )
        .ok();
    Ok(row)
}

/// Take ownership of one row. Returns false when someone else already has it,
/// which is exactly what stops a replayed or concurrent drain from delivering
/// the same follow-up twice.
pub fn claim(db: &Connection, id: &str) -> Result<bool, BridgeError> {
    let changed = db.execute(
        "UPDATE queued_session_input SET state=?2 WHERE id=?1 AND state=?3",
        params![id, STATE_CLAIMING, STATE_QUEUED],
    )?;
    Ok(changed == 1)
}

pub fn mark_delivered(db: &Connection, id: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE queued_session_input SET state=?2,delivered_at=?3 WHERE id=?1 AND state=?4",
        params![
            id,
            STATE_DELIVERED,
            Utc::now().to_rfc3339(),
            STATE_CLAIMING
        ],
    )?;
    Ok(())
}

/// Hand a claimed row back to the queue after a failed write, so a transient
/// adapter error postpones the follow-up instead of eating it.
pub fn release(db: &Connection, id: &str) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE queued_session_input SET state=?2 WHERE id=?1 AND state=?3",
        params![id, STATE_QUEUED, STATE_CLAIMING],
    )?;
    Ok(())
}

/// Sessions holding waiting input. The maintenance sweep uses this to find work
/// after a reconnect, when no turn-completion event is coming.
pub fn sessions_with_queued_input(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT DISTINCT session_id FROM queued_session_input WHERE state=?1 ORDER BY session_id",
    )?;
    let rows = statement
        .query_map(params![STATE_QUEUED], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn pending_count(db: &Connection, session_id: &str) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM queued_session_input WHERE session_id=?1 AND state=?2",
        params![session_id, STATE_QUEUED],
        |row| row.get(0),
    )?)
}

/// Startup recovery for rows claimed by a process that died before confirming
/// the write. Redelivery could duplicate and silence could lose, so they are
/// marked abandoned and returned for the caller to surface.
pub fn recover_claimed(db: &Connection) -> Result<Vec<QueuedInput>, BridgeError> {
    let stranded = {
        let mut statement = db.prepare(
            "SELECT id,sequence,session_id,provider_text,display_text
             FROM queued_session_input WHERE state=?1 ORDER BY sequence",
        )?;
        let rows = statement
            .query_map(params![STATE_CLAIMING], |row| {
                Ok(QueuedInput {
                    id: row.get(0)?,
                    sequence: row.get(1)?,
                    session_id: row.get(2)?,
                    provider_text: row.get(3)?,
                    display_text: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    if !stranded.is_empty() {
        db.execute(
            "UPDATE queued_session_input SET state=?1 WHERE state=?2",
            params![STATE_ABANDONED, STATE_CLAIMING],
        )?;
    }
    Ok(stranded)
}

/// Drop a session's waiting input, returning the rows that were dropped.
///
/// `/clear` and session teardown mean the conversation the follow-up belonged to
/// is gone. The ids come back so the caller can record one durable row per
/// dropped follow-up — the client folds those rows to decide what is still
/// waiting, and a summary count would not tell it which.
pub fn discard_for_session(db: &Connection, session_id: &str) -> Result<Vec<String>, BridgeError> {
    let ids = {
        let mut statement = db.prepare(
            "SELECT id FROM queued_session_input
             WHERE session_id=?1 AND state IN (?2,?3) ORDER BY sequence",
        )?;
        let rows = statement
            .query_map(params![session_id, STATE_QUEUED, STATE_CLAIMING], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    if !ids.is_empty() {
        db.execute(
            "UPDATE queued_session_input SET state=?2 WHERE session_id=?1 AND state IN (?3,?4)",
            params![
                session_id,
                STATE_ABANDONED,
                STATE_QUEUED,
                STATE_CLAIMING
            ],
        )?;
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE queued_session_input (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                session_id TEXT NOT NULL,
                provider_text TEXT NOT NULL,
                display_text TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('queued','claiming','delivered','abandoned')),
                created_at TEXT NOT NULL,
                delivered_at TEXT
            );",
        )
        .unwrap();
        db
    }

    #[test]
    fn route_never_starts_a_second_turn_against_a_busy_provider() {
        assert_eq!(route(false, false), InputRoute::NewTurn);
        assert_eq!(route(false, true), InputRoute::NewTurn);
        assert_eq!(route(true, true), InputRoute::Steer);
        assert_eq!(route(true, false), InputRoute::Queue);
        assert_eq!(
            route(true, false).disposition(),
            InputDisposition::QueuedForPhaseBoundary
        );
        assert_eq!(
            route(true, true).disposition(),
            InputDisposition::SteeredActiveTurn
        );
        assert_eq!(
            route(false, false).disposition(),
            InputDisposition::StartedNewTurn
        );
    }

    #[test]
    fn worker_steer_gate_refuses_a_reported_worker_and_a_dead_one() {
        assert_eq!(
            worker_steer_gate("reported", "working", true),
            Err(WorkerSteerRefusal::AlreadyReported),
            "a reported result is final; guidance cannot rewrite it"
        );
        assert_eq!(
            worker_steer_gate("pending", "checkpointing", true),
            Err(WorkerSteerRefusal::Checkpointing)
        );
        assert_eq!(
            worker_steer_gate("pending", "working", false),
            Err(WorkerSteerRefusal::NotRunning),
            "a send must never be the thing that launches a worker"
        );
        // The reported check outranks liveness: a worker whose process is gone
        // *and* whose result is in should say the useful thing.
        assert_eq!(
            worker_steer_gate("reported", "completed", false),
            Err(WorkerSteerRefusal::AlreadyReported)
        );
        assert_eq!(worker_steer_gate("pending", "working", true), Ok(()));
        assert_eq!(worker_steer_gate("pending", "waiting", true), Ok(()));
    }

    #[test]
    fn worker_steer_uses_the_same_route_table_as_any_other_session() {
        // Once the gate says yes there is no worker-specific routing: the same
        // three answers, chosen the same way.
        assert!(worker_steer_gate("pending", "working", true).is_ok());
        assert_eq!(route(true, true), InputRoute::Steer);
        assert_eq!(route(true, false), InputRoute::Queue);
        assert_eq!(route(false, true), InputRoute::NewTurn);
    }

    #[test]
    fn session_control_commands_are_the_ones_that_need_an_idle_session() {
        assert!(requires_idle_session(&slash::SlashDispatch::Clear));
        assert!(requires_idle_session(&slash::SlashDispatch::Usage));
        assert!(requires_idle_session(&slash::SlashDispatch::Compact {
            focus: None
        }));
        assert!(requires_idle_session(&slash::SlashDispatch::Unsupported {
            name: "resume".into(),
            harness: "Codex".into(),
        }));
        assert!(!requires_idle_session(&slash::SlashDispatch::Forward {
            text: "keep going".into()
        }));
        assert!(!requires_idle_session(&slash::SlashDispatch::Expand {
            text: "expanded skill".into()
        }));
    }

    #[test]
    fn queued_input_is_delivered_exactly_once_under_concurrent_drains() {
        let db = db();
        let queued = enqueue(&db, "s-1", "provider", "display").unwrap();
        assert_eq!(pending_count(&db, "s-1").unwrap(), 1);

        // Two drains race for the same row; the compare-and-swap picks one.
        assert!(claim(&db, &queued.id).unwrap());
        assert!(
            !claim(&db, &queued.id).unwrap(),
            "a second drain must not win an already-claimed row"
        );
        mark_delivered(&db, &queued.id).unwrap();

        assert_eq!(next_queued(&db, "s-1").unwrap(), None);
        assert_eq!(pending_count(&db, "s-1").unwrap(), 0);
        // A replayed drain after delivery finds nothing to send.
        assert!(!claim(&db, &queued.id).unwrap());
    }

    #[test]
    fn queued_input_keeps_submission_order_and_survives_a_failed_write() {
        let db = db();
        let first = enqueue(&db, "s-1", "one", "one").unwrap();
        let second = enqueue(&db, "s-1", "two", "two").unwrap();
        assert!(first.sequence < second.sequence);

        assert_eq!(next_queued(&db, "s-1").unwrap().unwrap().id, first.id);
        assert!(claim(&db, &first.id).unwrap());
        // The adapter write failed: the follow-up goes back to the front of the
        // queue rather than being lost.
        release(&db, &first.id).unwrap();
        assert_eq!(next_queued(&db, "s-1").unwrap().unwrap().id, first.id);

        assert!(claim(&db, &first.id).unwrap());
        mark_delivered(&db, &first.id).unwrap();
        assert_eq!(next_queued(&db, "s-1").unwrap().unwrap().id, second.id);
    }

    #[test]
    fn queued_input_outlives_the_process_that_accepted_it() {
        let db = db();
        enqueue(&db, "s-1", "after restart", "after restart").unwrap();
        enqueue(&db, "s-2", "other session", "other session").unwrap();

        // A restart re-reads durable state; nothing was held in memory.
        assert_eq!(
            sessions_with_queued_input(&db).unwrap(),
            vec!["s-1".to_owned(), "s-2".to_owned()]
        );
        assert_eq!(
            next_queued(&db, "s-1").unwrap().unwrap().provider_text,
            "after restart"
        );
    }

    #[test]
    fn a_row_claimed_by_a_dead_process_is_surfaced_not_silently_resent() {
        let db = db();
        let queued = enqueue(&db, "s-1", "mid-write", "mid-write").unwrap();
        assert!(claim(&db, &queued.id).unwrap());

        let stranded = recover_claimed(&db).unwrap();
        assert_eq!(stranded.len(), 1);
        assert_eq!(stranded[0].id, queued.id);
        // Neither redelivered (would duplicate) nor dropped quietly.
        assert_eq!(next_queued(&db, "s-1").unwrap(), None);
        assert!(recover_claimed(&db).unwrap().is_empty());
    }

    #[test]
    fn discarding_a_session_clears_only_its_own_waiting_input() {
        let db = db();
        enqueue(&db, "s-1", "mine", "mine").unwrap();
        enqueue(&db, "s-2", "theirs", "theirs").unwrap();

        assert_eq!(discard_for_session(&db, "s-1").unwrap().len(), 1);
        assert_eq!(next_queued(&db, "s-1").unwrap(), None);
        assert!(next_queued(&db, "s-2").unwrap().is_some());
    }
}
