//! Orchestrator wait conditions: "wake me when my workers are done".
//!
//! Every worker result is otherwise its own orchestrator turn. A `bridge-wait`
//! arms a wait for some of the orchestrator's live workers; while it is armed,
//! their `bridge-worker-result` notices are held here instead of queued, and
//! when the wait fires they reach the orchestrator together in one
//! `bridge-worker-results` envelope through the ordinary input queue.
//!
//! Held results live in their own table rather than in
//! `queued_session_input`, so the queue's delivery rules stay exactly as they
//! were. Both tables are durable: a restart keeps an armed wait and what it
//! holds, and the sweep fires an overdue one afterwards.

use crate::delegation::{WaitBreak, WaitRequest, WaitUntil, WaitWorkers};
use crate::session_input;
use crate::BridgeError;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub fn install_store(transaction: &rusqlite::Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS orchestrator_waits(
            orchestrator_session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
            workers TEXT NOT NULL,
            until TEXT NOT NULL,
            break_on TEXT NOT NULL,
            deadline TEXT NOT NULL,
            armed_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS orchestrator_wait_results(
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            orchestrator_session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
            child_session_id TEXT NOT NULL,
            status TEXT NOT NULL,
            notice TEXT NOT NULL,
            received_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_orchestrator_wait_results
            ON orchestrator_wait_results(orchestrator_session_id, id);",
    )?;
    Ok(())
}

/// A result status that breaks a wait with `breakOn: ["failed"]`.
const FAILING_STATUSES: [&str; 3] = ["failed", "blocked", "protocol_invalid"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedWait {
    pub orchestrator_session_id: String,
    pub workers: Vec<String>,
    pub until: WaitUntil,
    pub break_on: Vec<WaitBreak>,
    pub deadline: DateTime<Utc>,
}

impl ArmedWait {
    pub fn covers(&self, child_session_id: &str) -> bool {
        self.workers.iter().any(|worker| worker == child_session_id)
    }

    pub fn breaks_on(&self, why: WaitBreak) -> bool {
        self.break_on.contains(&why)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmOutcome {
    Armed { workers: Vec<String>, adopted: usize },
    Refused(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireReason {
    AllDone,
    AnyDone,
    Failed,
    NeedsYou,
    Timeout,
    /// An essential routing notice arrived while the wait was armed.
    Notice,
}

impl FireReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllDone => "all_done",
            Self::AnyDone => "any_done",
            Self::Failed => "failed",
            Self::NeedsYou => "needs_you",
            Self::Timeout => "timeout",
            Self::Notice => "notice",
        }
    }
}

/// The orchestrator's live workers: its own children that have not reported.
pub fn live_workers(db: &Connection, orchestrator_session_id: &str) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT session_id FROM worker_runtime
         WHERE parent_session_id=?1 AND result_status='pending' ORDER BY rowid",
    )?;
    let rows = statement.query_map(params![orchestrator_session_id], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Arm (or replace) the orchestrator's wait. Ownership is checked here: a
/// model-supplied id that is not one of the caller's running workers refuses
/// the whole wait. Results for the listed workers that are queued but not yet
/// delivered move into the wait, so a result that raced the orchestrator's own
/// turn is not delivered on its own. Run inside one transaction.
pub fn arm(db: &Connection, orchestrator_session_id: &str, request: &WaitRequest, now: DateTime<Utc>) -> Result<ArmOutcome, BridgeError> {
    let live = live_workers(db, orchestrator_session_id)?;
    let workers = match &request.workers {
        WaitWorkers::All(_) => {
            if live.is_empty() {
                return Ok(ArmOutcome::Refused("none of your workers is running, so there is nothing to wait for".into()));
            }
            live
        }
        WaitWorkers::Listed(ids) => {
            let mut workers = Vec::new();
            for id in ids {
                if !live.contains(id) {
                    return Ok(ArmOutcome::Refused(format!("{id} is not one of your running workers, so no wait was armed")));
                }
                if !workers.contains(id) {
                    workers.push(id.clone());
                }
            }
            workers
        }
    };
    let deadline = now + chrono::Duration::seconds(request.timeout_seconds() as i64);
    let break_on: Vec<&str> = request
        .breaks_on()
        .into_iter()
        .map(|why| match why {
            WaitBreak::Failed => "failed",
            WaitBreak::NeedsYou => "needs_you",
        })
        .collect();
    db.execute(
        "INSERT INTO orchestrator_waits(orchestrator_session_id,workers,until,break_on,deadline,armed_at)
         VALUES(?1,?2,?3,?4,?5,?6)
         ON CONFLICT(orchestrator_session_id) DO UPDATE SET
           workers=excluded.workers, until=excluded.until, break_on=excluded.break_on,
           deadline=excluded.deadline, armed_at=excluded.armed_at",
        params![
            orchestrator_session_id,
            serde_json::to_string(&workers).unwrap_or_else(|_| "[]".into()),
            request.until.as_str(),
            serde_json::to_string(&break_on).unwrap_or_else(|_| "[]".into()),
            deadline.to_rfc3339(),
            now.to_rfc3339(),
        ],
    )?;
    let adopted = adopt_queued_results(db, orchestrator_session_id, &workers, now)?;
    Ok(ArmOutcome::Armed { workers, adopted })
}

fn adopt_queued_results(db: &Connection, orchestrator_session_id: &str, workers: &[String], now: DateTime<Utc>) -> Result<usize, BridgeError> {
    let queued: Vec<(String, String)> = {
        let mut statement = db.prepare(
            "SELECT id, provider_text FROM queued_session_input
             WHERE session_id=?1 AND state='queued' ORDER BY sequence",
        )?;
        let rows = statement.query_map(params![orchestrator_session_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut adopted = 0;
    for (id, provider_text) in queued {
        let Ok(notice) = serde_json::from_str::<Value>(&provider_text) else { continue };
        if notice["type"] != "bridge-worker-result" {
            continue;
        }
        let Some(child) = notice["childSessionId"].as_str() else { continue };
        if !workers.iter().any(|worker| worker == child) {
            continue;
        }
        let claimed = db.execute(
            "UPDATE queued_session_input SET state='delivered', delivered_at=?2 WHERE id=?1 AND state='queued'",
            params![id, now.to_rfc3339()],
        )?;
        if claimed == 1 {
            hold(db, orchestrator_session_id, child, notice["status"].as_str().unwrap_or("completed"), &provider_text, now)?;
            adopted += 1;
        }
    }
    Ok(adopted)
}

/// The orchestrator's armed wait, if it has one.
pub fn armed(db: &Connection, orchestrator_session_id: &str) -> Result<Option<ArmedWait>, BridgeError> {
    let row: Option<(String, String, String, String)> = db
        .query_row(
            "SELECT workers, until, break_on, deadline FROM orchestrator_waits WHERE orchestrator_session_id=?1",
            params![orchestrator_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((workers, until, break_on, deadline)) = row else { return Ok(None) };
    Ok(Some(ArmedWait {
        orchestrator_session_id: orchestrator_session_id.to_owned(),
        workers: serde_json::from_str(&workers).unwrap_or_default(),
        until: if until == "any_done" { WaitUntil::AnyDone } else { WaitUntil::AllDone },
        break_on: serde_json::from_str::<Vec<String>>(&break_on)
            .unwrap_or_default()
            .iter()
            .filter_map(|why| match why.as_str() {
                "failed" => Some(WaitBreak::Failed),
                "needs_you" => Some(WaitBreak::NeedsYou),
                _ => None,
            })
            .collect(),
        deadline: DateTime::parse_from_rfc3339(&deadline).map(|at| at.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
    }))
}

/// Orchestrators with an armed wait, for the sweep.
pub fn armed_orchestrators(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare("SELECT orchestrator_session_id FROM orchestrator_waits ORDER BY armed_at")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Hold one worker's result notice for the orchestrator's wait.
pub fn hold(db: &Connection, orchestrator_session_id: &str, child_session_id: &str, status: &str, notice: &str, now: DateTime<Utc>) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO orchestrator_wait_results(orchestrator_session_id,child_session_id,status,notice,received_at)
         VALUES(?1,?2,?3,?4,?5)",
        params![orchestrator_session_id, child_session_id, status, notice, now.to_rfc3339()],
    )?;
    Ok(())
}

fn held(db: &Connection, orchestrator_session_id: &str) -> Result<Vec<(String, String, String)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT child_session_id, status, notice FROM orchestrator_wait_results
         WHERE orchestrator_session_id=?1 ORDER BY id",
    )?;
    let rows = statement.query_map(params![orchestrator_session_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Why the wait should fire now, if it should: a failure it breaks on, its
/// condition, or its deadline, in that order.
pub fn due(db: &Connection, wait: &ArmedWait, now: DateTime<Utc>) -> Result<Option<FireReason>, BridgeError> {
    let held = held(db, &wait.orchestrator_session_id)?;
    if wait.breaks_on(WaitBreak::Failed) && held.iter().any(|(_, status, _)| FAILING_STATUSES.contains(&status.as_str())) {
        return Ok(Some(FireReason::Failed));
    }
    match wait.until {
        WaitUntil::AnyDone if !held.is_empty() => return Ok(Some(FireReason::AnyDone)),
        WaitUntil::AllDone => {
            let live = live_workers(db, &wait.orchestrator_session_id)?;
            // Done: it reported into this wait, or it is no longer running at
            // all (stopped, archived); either way nothing more is coming.
            let all_done = wait
                .workers
                .iter()
                .all(|worker| held.iter().any(|(child, _, _)| child == worker) || !live.contains(worker));
            if all_done {
                return Ok(Some(FireReason::AllDone));
            }
        }
        WaitUntil::AnyDone => {}
    }
    if now >= wait.deadline {
        return Ok(Some(FireReason::Timeout));
    }
    Ok(None)
}

/// Fire the orchestrator's wait: enqueue one `bridge-worker-results` envelope
/// with everything held and clear the wait, in the caller's transaction.
/// `None` when no wait is armed. The queue delivers the envelope at the next
/// idle boundary, as it does any other input.
pub fn fire(
    db: &Connection,
    orchestrator_session_id: &str,
    reason: FireReason,
    notice: Option<Value>,
    fleet: Value,
) -> Result<Option<session_input::QueuedInput>, BridgeError> {
    let Some(wait) = armed(db, orchestrator_session_id)? else { return Ok(None) };
    let held = held(db, orchestrator_session_id)?;
    let live = live_workers(db, orchestrator_session_id)?;
    let reported: Vec<&str> = held.iter().map(|(child, _, _)| child.as_str()).collect();
    let still_running: Vec<&String> = wait
        .workers
        .iter()
        .filter(|worker| !reported.contains(&worker.as_str()) && live.contains(worker))
        .collect();
    let results: Vec<Value> = held
        .iter()
        .map(|(_, _, notice)| serde_json::from_str(notice).unwrap_or_else(|_| json!({"type": "bridge-worker-result", "raw": notice})))
        .collect();
    let instruction = match (reason, still_running.is_empty()) {
        (_, true) => "One delivery for your bridge-wait: every result below is a bridge-worker-result, unchanged, and each one's referenced SQLite worker.result entry is canonical. Route what comes next from all of them together.",
        (FireReason::Timeout, false) => "Your bridge-wait timed out. The results below arrived; the workers in stillRunning have not reported. Do not claim the task is done: report what finished, say what is still running, and wait again or peek if you need them.",
        (_, false) => "Your bridge-wait was broken early. The results below arrived; the workers in stillRunning have not reported. Do not claim the task is done while any of them is running.",
    };
    let envelope = json!({
        "type": "bridge-worker-results",
        "reason": reason.as_str(),
        "results": results,
        "stillRunning": still_running,
        "notice": notice,
        "fleet": fleet,
        "instruction": instruction,
    })
    .to_string();
    let display = match results.len() {
        1 => "1 worker result".to_owned(),
        count => format!("{count} worker results"),
    };
    let queued = session_input::enqueue(db, orchestrator_session_id, &envelope, &display)?;
    db.execute("DELETE FROM orchestrator_wait_results WHERE orchestrator_session_id=?1", params![orchestrator_session_id])?;
    db.execute("DELETE FROM orchestrator_waits WHERE orchestrator_session_id=?1", params![orchestrator_session_id])?;
    Ok(Some(queued))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::WaitRequest;

    fn store() -> (tempfile::TempDir, Connection) {
        let scratch = tempfile::tempdir().unwrap();
        let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
        db.execute_batch(
            "INSERT INTO sessions(id,harness,label,status,kind) VALUES('orch','claude','Orchestrator','ready','orchestrator');
             INSERT INTO sessions(id,harness,label,status,kind) VALUES('other','claude','Other','ready','orchestrator');",
        )
        .unwrap();
        for (id, parent) in [("w1", "orch"), ("w2", "orch"), ("w3", "orch"), ("x1", "other")] {
            worker(&db, id, parent);
        }
        (scratch, db)
    }

    fn worker(db: &Connection, id: &str, parent: &str) {
        db.execute(
            "INSERT INTO sessions(id,harness,label,status,kind,parent_session_id,depth) VALUES(?1,'codex',?1,'working','worker',?2,1)",
            params![id, parent],
        )
        .unwrap();
        db.execute(
            "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,result_status,updated_at)
             VALUES(?1,?2,'running','implementation','k','pending',?3)",
            params![id, parent, Utc::now().to_rfc3339()],
        )
        .unwrap();
    }

    fn report(db: &Connection, id: &str) {
        db.execute("UPDATE worker_runtime SET result_status='reported' WHERE session_id=?1", params![id]).unwrap();
    }

    fn notice(child: &str, status: &str) -> String {
        json!({"type": "bridge-worker-result", "childSessionId": child, "status": status, "summary": format!("{child} {status}")}).to_string()
    }

    fn request(text: &str) -> WaitRequest {
        match crate::delegation::parse_wait_request(&format!("```bridge-wait\n{text}\n```")) {
            crate::delegation::ParseOutcome::Parsed(request) => request,
            _ => panic!("bad request {text}"),
        }
    }

    fn queued(db: &Connection) -> Vec<(String, String)> {
        db.prepare("SELECT provider_text, display_text FROM queued_session_input WHERE session_id='orch' AND state='queued' ORDER BY sequence")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// What the delivery path does with one worker result: hold it for an armed
    /// wait that covers it, otherwise queue it as today; then fire if due.
    fn deliver(db: &Connection, child: &str, status: &str, now: DateTime<Utc>) {
        report(db, child);
        let text = notice(child, status);
        match armed(db, "orch").unwrap().filter(|wait| wait.covers(child)) {
            Some(_) => hold(db, "orch", child, status, &text, now).unwrap(),
            None => {
                session_input::enqueue(db, "orch", &text, "result").unwrap();
            }
        }
        if let Some(wait) = armed(db, "orch").unwrap() {
            if let Some(reason) = due(db, &wait, now).unwrap() {
                fire(db, "orch", reason, None, json!([])).unwrap();
            }
        }
    }

    fn envelope(db: &Connection) -> Value {
        let rows = queued(db);
        assert_eq!(rows.len(), 1, "exactly one delivery: {rows:?}");
        serde_json::from_str(&rows[0].0).unwrap()
    }

    #[test]
    fn arming_resolves_all_and_refuses_what_is_not_yours() {
        let (_scratch, db) = store();
        let now = Utc::now();
        assert_eq!(
            arm(&db, "orch", &request("{}"), now).unwrap(),
            ArmOutcome::Armed { workers: vec!["w1".into(), "w2".into(), "w3".into()], adopted: 0 }
        );
        let wait = armed(&db, "orch").unwrap().unwrap();
        assert_eq!((wait.until, wait.break_on.clone()), (WaitUntil::AllDone, vec![WaitBreak::Failed, WaitBreak::NeedsYou]));
        assert_eq!(wait.deadline, now + chrono::Duration::seconds(900));
        assert!(matches!(arm(&db, "orch", &request(r#"{"workers":["w1","x1"]}"#), now).unwrap(), ArmOutcome::Refused(reason) if reason.contains("x1")));
        assert!(matches!(arm(&db, "other", &request(r#"{"workers":["w1"]}"#), now).unwrap(), ArmOutcome::Refused(_)), "a sibling's worker");
        report(&db, "x1");
        assert!(matches!(arm(&db, "other", &request("{}"), now).unwrap(), ArmOutcome::Refused(reason) if reason.contains("nothing to wait for")));
    }

    #[test]
    fn three_results_under_all_done_arrive_as_one_delivery() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request(r#"{"workers":"all","until":"all_done"}"#), now).unwrap();
        deliver(&db, "w1", "completed", now);
        deliver(&db, "w2", "completed", now);
        assert!(queued(&db).is_empty(), "held while workers still run");
        deliver(&db, "w3", "completed", now);
        let delivered = envelope(&db);
        assert_eq!(delivered["type"], "bridge-worker-results");
        assert_eq!(delivered["reason"], "all_done");
        let children: Vec<&str> = delivered["results"].as_array().unwrap().iter().map(|result| result["childSessionId"].as_str().unwrap()).collect();
        assert_eq!(children, ["w1", "w2", "w3"]);
        assert_eq!(delivered["stillRunning"], json!([]));
        assert_eq!(queued(&db)[0].1, "3 worker results");
        assert!(armed(&db, "orch").unwrap().is_none(), "a fired wait is gone");
    }

    #[test]
    fn a_failure_breaks_the_wait_with_the_results_so_far() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request("{}"), now).unwrap();
        deliver(&db, "w1", "completed", now);
        deliver(&db, "w2", "failed", now);
        let delivered = envelope(&db);
        assert_eq!(delivered["reason"], "failed");
        assert_eq!(delivered["results"].as_array().unwrap().len(), 2);
        assert_eq!(delivered["stillRunning"], json!(["w3"]));
        assert!(delivered["instruction"].as_str().unwrap().contains("Do not claim the task is done"));
    }

    #[test]
    fn a_failure_does_not_break_a_wait_that_does_not_ask() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request(r#"{"breakOn":[]}"#), now).unwrap();
        deliver(&db, "w1", "failed", now);
        assert!(queued(&db).is_empty());
    }

    #[test]
    fn any_done_fires_on_the_first_result() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request(r#"{"until":"any_done"}"#), now).unwrap();
        deliver(&db, "w2", "completed", now);
        let delivered = envelope(&db);
        assert_eq!(delivered["reason"], "any_done");
        assert_eq!(delivered["stillRunning"], json!(["w1", "w3"]));
    }

    #[test]
    fn the_timeout_delivers_what_arrived_and_names_the_rest() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request(r#"{"timeoutSeconds":60}"#), now).unwrap();
        deliver(&db, "w1", "completed", now);
        let wait = armed(&db, "orch").unwrap().unwrap();
        assert_eq!(due(&db, &wait, now + chrono::Duration::seconds(59)).unwrap(), None);
        let reason = due(&db, &wait, now + chrono::Duration::seconds(60)).unwrap().unwrap();
        assert_eq!(reason, FireReason::Timeout);
        fire(&db, "orch", reason, None, json!([])).unwrap();
        let delivered = envelope(&db);
        assert_eq!(delivered["reason"], "timeout");
        assert_eq!(delivered["stillRunning"], json!(["w2", "w3"]));
        assert!(delivered["instruction"].as_str().unwrap().contains("timed out"));
    }

    #[test]
    fn an_essential_notice_fires_with_the_notice_inside() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request("{}"), now).unwrap();
        deliver(&db, "w1", "completed", now);
        let stopped = json!({"type": "bridge-worker-stopped", "childSessionId": "w2"});
        fire(&db, "orch", FireReason::Notice, Some(stopped.clone()), json!([])).unwrap();
        let delivered = envelope(&db);
        assert_eq!((delivered["reason"].clone(), delivered["notice"].clone()), (json!("notice"), stopped));
    }

    #[test]
    fn results_outside_the_wait_are_queued_as_today() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request(r#"{"workers":["w1","w2"]}"#), now).unwrap();
        deliver(&db, "w3", "completed", now);
        let rows = queued(&db);
        assert_eq!(rows.len(), 1);
        assert_eq!(serde_json::from_str::<Value>(&rows[0].0).unwrap()["type"], "bridge-worker-result", "not held, not wrapped");
        assert!(armed(&db, "orch").unwrap().is_some());
    }

    #[test]
    fn a_result_queued_before_the_wait_armed_moves_into_it() {
        let (_scratch, db) = store();
        let now = Utc::now();
        // w1's result raced the orchestrator's own turn: it is queued, not yet
        // delivered, and w1 is still pending in the runtime table.
        session_input::enqueue(&db, "orch", &notice("w1", "completed"), "result").unwrap();
        session_input::enqueue(&db, "orch", "the person typed this", "the person typed this").unwrap();
        assert_eq!(
            arm(&db, "orch", &request(r#"{"workers":["w1","w2"]}"#), now).unwrap(),
            ArmOutcome::Armed { workers: vec!["w1".into(), "w2".into()], adopted: 1 }
        );
        assert_eq!(queued(&db).iter().map(|row| row.1.as_str()).collect::<Vec<_>>(), ["the person typed this"], "only the result moved");
        deliver(&db, "w2", "completed", now);
        let rows = queued(&db);
        let delivered: Value = serde_json::from_str(&rows[1].0).unwrap();
        assert_eq!(delivered["results"].as_array().unwrap().len(), 2, "the adopted result travels with w2's");
    }

    #[test]
    fn rearming_keeps_what_is_held() {
        let (_scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request("{}"), now).unwrap();
        deliver(&db, "w1", "completed", now);
        // A new block replaces the wait's terms, not what it holds.
        arm(&db, "orch", &request(r#"{"workers":["w2"],"until":"any_done"}"#), now).unwrap();
        let wait = armed(&db, "orch").unwrap().unwrap();
        assert_eq!((wait.workers.clone(), wait.until), (vec!["w2".to_owned()], WaitUntil::AnyDone));
        deliver(&db, "w2", "completed", now);
        let children: Vec<String> = envelope(&db)["results"].as_array().unwrap().iter().map(|result| result["childSessionId"].as_str().unwrap().to_owned()).collect();
        assert_eq!(children, ["w1", "w2"]);
    }

    #[test]
    fn an_armed_wait_survives_a_restart() {
        let (scratch, db) = store();
        let now = Utc::now();
        arm(&db, "orch", &request(r#"{"timeoutSeconds":120}"#), now).unwrap();
        deliver(&db, "w1", "completed", now);
        drop(db);
        let reopened = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
        let wait = armed(&reopened, "orch").unwrap().expect("still armed after a restart");
        assert_eq!(armed_orchestrators(&reopened).unwrap(), vec!["orch".to_owned()]);
        assert_eq!(due(&reopened, &wait, now).unwrap(), None);
        deliver(&reopened, "w2", "completed", now);
        deliver(&reopened, "w3", "completed", now);
        assert_eq!(envelope(&reopened)["results"].as_array().unwrap().len(), 3, "the held result came through the restart");
    }
}
