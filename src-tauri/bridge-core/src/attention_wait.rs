//! Wait until a chat finishes, fails, needs the person, or goes idle.
//!
//! The attention ledger already decides what those moments are; a waiter
//! follows it. A wait holds no database lock while it waits: it follows the
//! core event bus, re-reads briefly when something moved, and re-reads every
//! two seconds anyway because the bus is lossy.

use crate::attention::{self, AttentionKind, AttentionRow};
use crate::{BridgeCore, BridgeError};
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub const MAX_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const POLL: Duration = Duration::from_millis(100);
const RECHECK: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitState {
    Finished,
    Failed,
    NeedsYou,
    Idle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitOutcome {
    pub state: WaitState,
    pub matched: bool,
    pub item: Option<AttentionRow>,
}

/// What the chat looks like right now, read in one short lock.
#[derive(Debug)]
struct Snapshot {
    in_flight: bool,
    open_request: Option<AttentionRow>,
    turn_end: Option<AttentionRow>,
}

fn snapshot(db: &Connection, session_id: &str, pin: i64) -> Result<Snapshot, BridgeError> {
    // The same "a turn exists" test the input path uses (`turn_is_active`).
    let in_flight: Option<bool> = db
        .query_row(
            "SELECT active_turn_id IS NOT NULL OR status IN ('working','checkpointing')
             FROM sessions WHERE id=?1 AND archived_at IS NULL",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(in_flight) = in_flight else {
        return Err(BridgeError::Invalid(format!("no chat {session_id} to wait on")));
    };
    Ok(Snapshot {
        in_flight,
        open_request: attention::open_request(db, session_id)?,
        turn_end: attention::first_turn_end_after(db, session_id, pin)?,
    })
}

/// Whether `until` is satisfied. `vouched` means the caller pinned the
/// sequence before sending, or a turn was already in flight at the call: only
/// then can a turn end after the pin be the turn being waited for.
fn evaluate(now: &Snapshot, until: &[WaitState], vouched: bool, pinned_by_caller: bool) -> Option<(WaitState, Option<AttentionRow>)> {
    if until.contains(&WaitState::NeedsYou) {
        if let Some(row) = &now.open_request {
            return Some((WaitState::NeedsYou, Some(row.clone())));
        }
    }
    if vouched {
        if let Some(row) = &now.turn_end {
            let state = if row.kind == AttentionKind::Failed { WaitState::Failed } else { WaitState::Finished };
            if until.contains(&state) {
                return Some((state, Some(row.clone())));
            }
            if until.contains(&WaitState::Idle) && !now.in_flight && now.open_request.is_none() {
                return Some((WaitState::Idle, Some(row.clone())));
            }
        }
    }
    // A sent turn that has not started yet is not idle, so a caller's own pin
    // needs the turn's end first (above).
    if !pinned_by_caller && until.contains(&WaitState::Idle) && !now.in_flight && now.open_request.is_none() {
        return Some((WaitState::Idle, None));
    }
    None
}

fn current(now: &Snapshot) -> WaitState {
    if now.open_request.is_some() {
        WaitState::NeedsYou
    } else {
        WaitState::Idle
    }
}

/// Wait on one chat. `after_sequence` is the caller's pin (read before it
/// sent); without it the pin is the ledger's newest sequence now.
pub fn wait(
    core: &Arc<BridgeCore>,
    session_id: &str,
    until: &[WaitState],
    timeout: Option<Duration>,
    after_sequence: Option<i64>,
) -> Result<WaitOutcome, BridgeError> {
    if until.is_empty() {
        return Err(BridgeError::Invalid("say what to wait for: finished, failed, needsYou or idle".into()));
    }
    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT).min(MAX_TIMEOUT);
    // Subscribe before the first read, so nothing between them is missed.
    let mut events = core.events.subscribe();
    let pinned_by_caller = after_sequence.is_some();
    let (pin, first) = {
        let db = core.db.lock().unwrap();
        let pin = match after_sequence {
            Some(pin) => pin,
            None => attention::latest_sequence(&db)?,
        };
        (pin, snapshot(&db, session_id, pin)?)
    };
    let vouched = pinned_by_caller || first.in_flight;
    if let Some((state, item)) = evaluate(&first, until, vouched, pinned_by_caller) {
        return Ok(WaitOutcome { state, matched: true, item });
    }
    // Nothing is running and the caller sent nothing: no state in `until` can
    // still arrive.
    if !vouched {
        return Ok(WaitOutcome { state: current(&first), matched: false, item: None });
    }
    let deadline = Instant::now() + timeout;
    let mut last_read = Instant::now();
    loop {
        let mut moved = false;
        while events.try_recv().is_ok() {
            moved = true;
        }
        if moved || last_read.elapsed() >= RECHECK {
            last_read = Instant::now();
            let now = snapshot(&core.db.lock().unwrap(), session_id, pin)?;
            if let Some((state, item)) = evaluate(&now, until, vouched, pinned_by_caller) {
                return Ok(WaitOutcome { state, matched: true, item });
            }
        }
        if Instant::now() >= deadline {
            return Err(BridgeError::Invalid(format!(
                "Timed out after {}s waiting on {session_id}. Input may already have been sent; read the chat before retrying.",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn core() -> (tempfile::TempDir, Arc<BridgeCore>) {
        let scratch = tempfile::tempdir().unwrap();
        let core = Arc::new(BridgeCore::for_tests(scratch.path()));
        core.db
            .lock()
            .unwrap()
            .execute("INSERT INTO sessions(id,harness,label,status,kind,title) VALUES('chat','claude','Chat','ready','direct','Ship it')", [])
            .unwrap();
        (scratch, core)
    }

    fn set_turn(core: &BridgeCore, in_flight: bool) {
        core.db
            .lock()
            .unwrap()
            .execute("UPDATE sessions SET active_turn_id=?1, status=?2 WHERE id='chat'", params![in_flight.then_some("turn-1"), if in_flight { "working" } else { "ready" }])
            .unwrap();
    }

    fn record(core: &BridgeCore, kind: &str, payload: serde_json::Value) {
        let mut event = crate::agent::NormalizedEvent::new(kind);
        event.status = payload.get("status").and_then(|value| value.as_str()).map(str::to_owned);
        crate::store::session_event(&core.db.lock().unwrap(), "chat", &event, &payload).unwrap();
        core.events.publish(crate::events::CoreEvent::AttentionChanged);
    }

    fn later(core: &Arc<BridgeCore>, millis: u64, work: impl FnOnce(&BridgeCore) + Send + 'static) -> std::thread::JoinHandle<()> {
        let core = core.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(millis));
            work(&core);
        })
    }

    #[test]
    fn a_finished_row_after_the_pin_matches() {
        let (_scratch, core) = core();
        set_turn(&core, true);
        let writer = later(&core, 150, |core| {
            set_turn(core, false);
            record(core, "turn.completed", json!({"status": "completed"}));
        });
        let outcome = wait(&core, "chat", &[WaitState::Finished, WaitState::NeedsYou], Some(Duration::from_secs(5)), None).unwrap();
        writer.join().unwrap();
        assert_eq!((outcome.state, outcome.matched), (WaitState::Finished, true));
        assert_eq!(outcome.item.unwrap().kind, AttentionKind::Finished);
    }

    #[test]
    fn a_turn_that_starts_after_the_call_does_not_match() {
        let (_scratch, core) = core();
        // An earlier turn finished before the call.
        record(&core, "turn.completed", json!({"status": "completed"}));
        let outcome = wait(&core, "chat", &[WaitState::Finished], Some(Duration::from_secs(5)), None).unwrap();
        assert_eq!((outcome.state, outcome.matched), (WaitState::Idle, false), "nothing running, nothing sent: return at once");
        // A caller that pinned before sending does see its own turn end.
        let pin = attention::latest_sequence(&core.db.lock().unwrap()).unwrap();
        let writer = later(&core, 100, |core| record(core, "turn.completed", json!({"status": "failed"})));
        let outcome = wait(&core, "chat", &[WaitState::Finished, WaitState::Failed], Some(Duration::from_secs(5)), Some(pin)).unwrap();
        writer.join().unwrap();
        assert_eq!((outcome.state, outcome.matched), (WaitState::Failed, true));
    }

    #[test]
    fn an_open_request_matches_at_once() {
        let (_scratch, core) = core();
        set_turn(&core, true);
        record(&core, "approval.requested", json!({}));
        let started = Instant::now();
        let outcome = wait(&core, "chat", &[WaitState::NeedsYou], Some(Duration::from_secs(5)), None).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!((outcome.state, outcome.matched), (WaitState::NeedsYou, true));
        assert_eq!(outcome.item.unwrap().kind, AttentionKind::NeedsYou);
    }

    #[test]
    fn idle_matches_at_once_and_not_before_a_sent_turn_ends() {
        let (_scratch, core) = core();
        let outcome = wait(&core, "chat", &[WaitState::Idle], Some(Duration::from_secs(5)), None).unwrap();
        assert_eq!((outcome.state, outcome.matched), (WaitState::Idle, true));
        // A just-sent turn has not started: with the caller's pin, idle waits for its end.
        let pin = attention::latest_sequence(&core.db.lock().unwrap()).unwrap();
        let writer = later(&core, 300, |core| record(core, "turn.completed", json!({"status": "completed"})));
        let started = Instant::now();
        let outcome = wait(&core, "chat", &[WaitState::Idle], Some(Duration::from_secs(5)), Some(pin)).unwrap();
        writer.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250), "it waited for the turn to end");
        assert_eq!((outcome.state, outcome.matched), (WaitState::Idle, true));
    }

    #[test]
    fn nothing_left_to_wait_for_returns_unmatched() {
        let (_scratch, core) = core();
        let outcome = wait(&core, "chat", &[WaitState::Finished, WaitState::Failed], Some(Duration::from_secs(5)), None).unwrap();
        assert_eq!((outcome.state, outcome.matched), (WaitState::Idle, false));
        assert!(matches!(wait(&core, "chat", &[], None, None), Err(BridgeError::Invalid(_))));
        assert!(matches!(wait(&core, "missing", &[WaitState::Idle], None, None), Err(BridgeError::Invalid(_))));
    }

    #[test]
    fn a_timeout_says_input_may_have_been_sent() {
        let (_scratch, core) = core();
        set_turn(&core, true);
        let error = wait(&core, "chat", &[WaitState::Finished], Some(Duration::from_millis(300)), None).unwrap_err();
        assert!(error.to_string().contains("Input may already have been sent"), "{error}");
    }

    #[test]
    fn waiters_hold_no_lock() {
        let (_scratch, core) = core();
        set_turn(&core, true);
        let waiters: Vec<_> = (0..8)
            .map(|_| {
                let core = core.clone();
                std::thread::spawn(move || wait(&core, "chat", &[WaitState::Finished], Some(Duration::from_secs(5)), None).unwrap())
            })
            .collect();
        std::thread::sleep(Duration::from_millis(200));
        // Readers and writers go straight through while eight waiters wait.
        let started = Instant::now();
        for _ in 0..20 {
            let _ = attention::latest_sequence(&core.db.lock().unwrap()).unwrap();
        }
        assert!(started.elapsed() < Duration::from_millis(500));
        set_turn(&core, false);
        record(&core, "turn.completed", json!({"status": "completed"}));
        for waiter in waiters {
            assert_eq!(waiter.join().unwrap().state, WaitState::Finished);
        }
    }
}
