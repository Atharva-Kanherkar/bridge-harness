//! When a failed worker is worth paying for again — and when it is not.
//!
//! Bridge used to retry any typed `failed` result once, automatically, with the
//! instruction "Retry the same assigned task once." It never asked whether the
//! cause could have changed. A failing test retried into the same failing test;
//! a malformed envelope retried into another malformed envelope. Each one was a
//! full model turn the user did not ask for and could not see.
//!
//! So a retry now has to be earned. There are three things a failure can be, and
//! only one of them is worth another attempt:
//!
//! - **protocol invalid** — Bridge could not read the result. Nothing about the
//!   task changed, so nothing about the outcome would.
//! - **permanent** — the task itself did not work, and the worker proved it (a
//!   test it ran failed). The same attempt produces the same failure.
//! - **transient** — something outside the task went wrong: a timeout, a reset
//!   connection, a rate limit. That condition can change on its own.
//!
//! And even a transient failure is bounded, by objective rather than by session:
//! retrying the same objective through a fresh worker is the same spend, and
//! counting per session let an identical task be paid for twice under a new id.

use chrono::Utc;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::{
    delegation::{TestStatus, WorkerResult, WorkerResultStatus},
    BridgeError,
};

/// How many automatic attempts one objective is allowed, across every worker
/// that ever runs it.
pub const MAX_AUTOMATIC_ATTEMPTS_PER_OBJECTIVE: i64 = 1;

/// The kinds of turn Bridge spends on its own recovery. Counted separately from
/// the user's turns, and from each other.
pub const RECOVERY_CORRECTION: &str = "recovery.correction";
pub const RECOVERY_REPAIR: &str = "recovery.repair";
pub const RECOVERY_TASK_RETRY: &str = "recovery.task_retry";

/// Words that mean "this went wrong somewhere other than the work".
///
/// Deliberately narrow. A signal that is not on this list is treated as
/// permanent, so the failure mode of being wrong here is declining a retry that
/// might have helped — not spending turns on one that cannot.
const TRANSIENT_SIGNALS: &[&str] = &[
    "timeout",
    "timed out",
    "etimedout",
    "connection reset",
    "econnreset",
    "connection refused",
    "econnrefused",
    "socket hang up",
    "stream error",
    "rate limit",
    "rate-limited",
    "429",
    "502",
    "503",
    "504",
    "temporarily",
    "temporary failure",
    "try again later",
    "overloaded",
    "at capacity",
    "quota",
    "usage limit",
    "enotfound",
    "ehostunreach",
    "network error",
    "fetch failed",
];

/// The subset of [`TRANSIENT_SIGNALS`] that specifically means "this
/// provider account is out of usage", as opposed to a generic network hiccup
/// that has nothing to do with which harness answered. Retrying the very same
/// harness immediately would just hit the same wall, so a quota signal is
/// routed differently by its caller: see
/// `learning_router::mark_harness_quota_exhausted`.
const QUOTA_SIGNALS: &[&str] = &[
    "rate limit",
    "rate-limited",
    "429",
    "overloaded",
    "at capacity",
    "quota",
    "usage limit",
];

/// Whether a transient signal `classify` already identified means the
/// provider account ran out of quota, rather than a passing network fault.
pub fn is_quota_signal(signal: &str) -> bool {
    QUOTA_SIGNALS.contains(&signal)
}

/// What Bridge believes about a failure, from evidence rather than from the
/// worker's opinion of itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureClass {
    /// Bridge could not read the result. A transport fact, not a task outcome.
    ProtocolInvalid,
    /// Something outside the task failed and may not fail again.
    Transient { signal: String },
    /// The task did not work. Another identical attempt will not change that.
    Permanent { reason: String },
}

impl FailureClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ProtocolInvalid => "protocol_invalid",
            Self::Transient { .. } => "transient",
            Self::Permanent { .. } => "permanent",
        }
    }

    /// The cause in words, for the orchestrator and for the user. This is the
    /// thing the old generic retry instruction hid.
    pub fn cause(&self) -> String {
        match self {
            Self::ProtocolInvalid => {
                "the worker's result could not be read as a result".to_owned()
            }
            Self::Transient { signal } => format!("a transient failure ({signal})"),
            Self::Permanent { reason } => reason.clone(),
        }
    }
}

/// Classify a failed worker result.
///
/// Order matters: a failed test outranks a transient-looking word in the prose,
/// because a worker that ran a test and watched it fail has produced evidence
/// about the task, and evidence outranks phrasing.
pub fn classify(result: &WorkerResult) -> FailureClass {
    if result.status == WorkerResultStatus::ProtocolInvalid {
        return FailureClass::ProtocolInvalid;
    }
    if let Some(failed) = result
        .tests
        .iter()
        .find(|test| test.status == TestStatus::Failed)
    {
        return FailureClass::Permanent {
            reason: format!("a check the worker ran failed: {}", failed.command),
        };
    }
    let haystack = [result.summary.as_str()]
        .into_iter()
        .chain(result.risks.iter().map(String::as_str))
        .chain(result.remaining_work.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n")
        .to_ascii_lowercase();
    if let Some(signal) = TRANSIENT_SIGNALS
        .iter()
        .find(|signal| haystack.contains(**signal))
    {
        return FailureClass::Transient {
            signal: (*signal).to_owned(),
        };
    }
    FailureClass::Permanent {
        reason: "no transient cause was identified in the worker's own report".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Worth another attempt, and here is the condition expected to have moved.
    Retry { signal: String },
    /// Not worth another attempt, and here is why — which is what reaches the
    /// orchestrator and the user instead of a silent turn.
    Decline { reason: String },
}

impl RetryDecision {
    pub fn is_retry(&self) -> bool {
        matches!(self, Self::Retry { .. })
    }
}

/// Decide whether Bridge pays for another automatic attempt.
///
/// Pure, so the policy is readable in one place and testable without a live
/// worker. Every argument is evidence: what came back, whether this worker has
/// already been retried, whether its process is still warm, and what this
/// objective has already spent.
pub fn decide(
    result: &WorkerResult,
    retry_count: i64,
    has_hot_process: bool,
    attempts_spent: i64,
) -> RetryDecision {
    if !matches!(
        result.status,
        WorkerResultStatus::Failed | WorkerResultStatus::ProtocolInvalid
    ) {
        return RetryDecision::Decline {
            reason: format!("{} is not a failure", result.status.as_str()),
        };
    }
    let class = classify(result);
    let signal = match &class {
        FailureClass::ProtocolInvalid => {
            return RetryDecision::Decline {
                reason: "a formatting failure is not a task failure, and retrying it would repeat it".to_owned(),
            }
        }
        FailureClass::Permanent { reason } => {
            return RetryDecision::Decline {
                reason: format!("permanent: {reason}"),
            }
        }
        FailureClass::Transient { signal } => signal.clone(),
    };
    if retry_count > 0 {
        return RetryDecision::Decline {
            reason: "this worker has already been retried once".to_owned(),
        };
    }
    if attempts_spent >= MAX_AUTOMATIC_ATTEMPTS_PER_OBJECTIVE {
        return RetryDecision::Decline {
            reason: format!(
                "this objective has already used its {MAX_AUTOMATIC_ATTEMPTS_PER_OBJECTIVE} automatic attempt"
            ),
        };
    }
    if !has_hot_process {
        // Without a live process a "retry" is a fresh worker: a new spawn, new
        // context, new spend. That is a decision for the parent or the user.
        return RetryDecision::Decline {
            reason: "the worker's process is gone, so a retry would be a new worker".to_owned(),
        };
    }
    RetryDecision::Retry { signal }
}

/// A stable name for "this piece of work", independent of which worker ran it.
///
/// Hashed rather than stored verbatim so an objective's text does not end up as
/// a primary key, and truncated because a collision here costs one declined
/// retry, not correctness.
pub fn objective_key(parent_session_id: &str, role: &str, objective: &str) -> String {
    let digest = Sha256::digest(
        format!(
            "{parent_session_id}\u{1f}{role}\u{1f}{}",
            objective.trim().to_ascii_lowercase()
        )
        .as_bytes(),
    );
    format!("{digest:x}")[..32].to_owned()
}

pub fn attempts_spent(db: &Connection, objective_key: &str) -> Result<i64, BridgeError> {
    Ok(db
        .query_row(
            "SELECT attempts FROM worker_retry_budget WHERE objective_key=?1",
            params![objective_key],
            |row| row.get(0),
        )
        .unwrap_or(0))
}

/// Spend one attempt against an objective, recording the condition that was
/// expected to have changed. The signal is stored so a later reader can check
/// the claim rather than take it on trust.
pub fn consume_attempt(
    db: &Connection,
    objective_key: &str,
    parent_session_id: &str,
    signal: &str,
) -> Result<i64, BridgeError> {
    db.execute(
        "INSERT INTO worker_retry_budget(objective_key,parent_session_id,attempts,last_signal,updated_at)
         VALUES(?1,?2,1,?3,?4)
         ON CONFLICT(objective_key) DO UPDATE SET
            attempts=attempts+1,
            last_signal=excluded.last_signal,
            updated_at=excluded.updated_at",
        params![objective_key, parent_session_id, signal, Utc::now().to_rfc3339()],
    )?;
    attempts_spent(db, objective_key)
}

/// Record one turn Bridge spent recovering from its own coordination failure.
///
/// Two writes on purpose. `recovery_turns` is the ledger these counts are
/// queried from; the event row is what makes them visible, because a turn the
/// user paid for and cannot see is exactly the problem this whole path is
/// about. The event kind is the recovery kind, so the feed separates a
/// correction from a repair from a task retry without anyone having to parse a
/// message.
pub fn record_recovery_turn(
    db: &Connection,
    session_id: &str,
    kind: &str,
    detail: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO recovery_turns(session_id,kind,detail,created_at) VALUES(?1,?2,?3,?4)",
        params![session_id, kind, detail, Utc::now().to_rfc3339()],
    )?;
    crate::store::event(db, "recovery", kind, session_id, detail)?;
    Ok(())
}

/// How many recovery turns of each kind a session has cost, most-spent first.
pub fn recovery_turn_counts(
    db: &Connection,
    session_id: &str,
) -> Result<Vec<(String, i64)>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT kind,COUNT(*) FROM recovery_turns WHERE session_id=?1 GROUP BY kind ORDER BY kind",
    )?;
    let rows = statement
        .query_map(params![session_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delegation::{SuggestedNextAction, WorkerTestResult, SCHEMA_VERSION};

    fn failure(summary: &str) -> WorkerResult {
        WorkerResult {
            schema_version: SCHEMA_VERSION,
            status: WorkerResultStatus::Failed,
            summary: summary.into(),
            files_changed: Vec::new(),
            tests: Vec::new(),
            decisions: Vec::new(),
            risks: Vec::new(),
            remaining_work: Vec::new(),
            suggested_next_action: SuggestedNextAction::FollowUp,
            suggested_role: None,
            suggested_task: None,
        }
    }

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE worker_retry_budget (
                objective_key TEXT PRIMARY KEY,
                parent_session_id TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                last_signal TEXT,
                updated_at TEXT NOT NULL
            );
            CREATE TABLE recovery_turns (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                detail TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                kind TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();
        db
    }

    #[test]
    fn quota_signals_are_told_apart_from_generic_transients() {
        let result = failure("Request failed: 429 rate limit exceeded, please retry later");
        assert_eq!(
            classify(&result),
            FailureClass::Transient { signal: "rate limit".into() }
        );
        assert!(is_quota_signal("rate limit"));
        assert!(is_quota_signal("429"));
        assert!(is_quota_signal("quota"));
        assert!(
            !is_quota_signal("connection reset"),
            "a network hiccup is not an account-quota problem"
        );
        assert!(!is_quota_signal("timeout"));
    }

    #[test]
    fn a_transient_cause_is_retried_once_and_names_what_changed() {
        let result = failure("Connection reset by peer while streaming from the provider");
        assert_eq!(
            classify(&result),
            FailureClass::Transient {
                signal: "connection reset".into()
            }
        );
        let decision = decide(&result, 0, true, 0);
        assert_eq!(
            decision,
            RetryDecision::Retry {
                signal: "connection reset".into()
            },
            "the retry has to name the condition expected to have moved"
        );
        // Once. The second time, the same objective is out of budget.
        assert_eq!(
            decide(&result, 0, true, 1),
            RetryDecision::Decline {
                reason: "this objective has already used its 1 automatic attempt".into()
            }
        );
        assert!(!decide(&result, 1, true, 0).is_retry(), "already retried");
    }

    #[test]
    fn a_failed_check_is_permanent_however_the_prose_reads() {
        // The trap: a worker whose test failed *and* who mentions a timeout in
        // passing. Evidence outranks phrasing, so this does not buy a retry.
        let mut result = failure("The suite timed out on one case and another assertion failed");
        result.tests = vec![WorkerTestResult {
            command: "cargo test auth".into(),
            status: TestStatus::Failed,
            detail: None,
        }];
        assert_eq!(
            classify(&result),
            FailureClass::Permanent {
                reason: "a check the worker ran failed: cargo test auth".into()
            }
        );
        assert!(!decide(&result, 0, true, 0).is_retry());
    }

    #[test]
    fn an_unexplained_failure_is_permanent_rather_than_hopeful() {
        let result = failure("Could not complete the objective");
        assert!(matches!(classify(&result), FailureClass::Permanent { .. }));
        let RetryDecision::Decline { reason } = decide(&result, 0, true, 0) else {
            panic!("an unexplained failure must not be retried");
        };
        assert!(reason.contains("permanent"), "{reason}");
        assert!(reason.contains("no transient cause"), "{reason}");
    }

    #[test]
    fn a_formatting_failure_never_buys_a_turn() {
        let mut result = failure("the worker's result could not be read");
        result.status = WorkerResultStatus::ProtocolInvalid;
        assert_eq!(classify(&result), FailureClass::ProtocolInvalid);
        let RetryDecision::Decline { reason } = decide(&result, 0, true, 0) else {
            panic!("a formatting failure must not be retried");
        };
        assert!(reason.contains("not a task failure"), "{reason}");
    }

    #[test]
    fn a_success_is_not_a_retry_candidate() {
        for status in [
            WorkerResultStatus::Completed,
            WorkerResultStatus::Cancelled,
            WorkerResultStatus::Blocked,
            WorkerResultStatus::NeedsDelegation,
        ] {
            let mut result = failure("whatever");
            result.status = status;
            assert!(!decide(&result, 0, true, 0).is_retry(), "{status:?}");
        }
    }

    #[test]
    fn a_cold_worker_is_a_decision_not_an_automatic_respawn() {
        let result = failure("request timed out talking to the provider");
        assert!(decide(&result, 0, true, 0).is_retry());
        let RetryDecision::Decline { reason } = decide(&result, 0, false, 0) else {
            panic!("a dead process must not be respawned automatically");
        };
        assert!(reason.contains("new worker"), "{reason}");
    }

    #[test]
    fn the_budget_is_per_objective_not_per_worker() {
        let db = db();
        let key = objective_key("parent", "implementation", "Add refresh-token rotation");
        // Same work, described with different whitespace and case, is the same
        // work — otherwise re-asking would buy a fresh budget every time.
        assert_eq!(
            key,
            objective_key("parent", "implementation", "  add REFRESH-token rotation  ")
        );
        assert_ne!(key, objective_key("parent", "verification", "Add refresh-token rotation"));
        assert_ne!(key, objective_key("other", "implementation", "Add refresh-token rotation"));

        assert_eq!(attempts_spent(&db, &key).unwrap(), 0);
        assert_eq!(consume_attempt(&db, &key, "parent", "timeout").unwrap(), 1);
        assert_eq!(attempts_spent(&db, &key).unwrap(), 1);
        // A fresh worker on the same objective inherits the spend.
        let result = failure("connection reset by peer");
        assert!(!decide(&result, 0, true, attempts_spent(&db, &key).unwrap()).is_retry());
        assert_eq!(
            db.query_row(
                "SELECT last_signal FROM worker_retry_budget WHERE objective_key=?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
            "timeout",
            "the condition claimed to have changed is recorded, not taken on trust"
        );
    }

    #[test]
    fn recovery_turns_are_counted_apart_from_each_other() {
        let db = db();
        record_recovery_turn(&db, "chat", RECOVERY_CORRECTION, "bad role").unwrap();
        record_recovery_turn(&db, "chat", RECOVERY_CORRECTION, "bad role again").unwrap();
        record_recovery_turn(&db, "chat", RECOVERY_REPAIR, "missing fence").unwrap();
        record_recovery_turn(&db, "worker", RECOVERY_TASK_RETRY, "timeout").unwrap();

        assert_eq!(
            recovery_turn_counts(&db, "chat").unwrap(),
            vec![
                (RECOVERY_CORRECTION.to_owned(), 2),
                (RECOVERY_REPAIR.to_owned(), 1)
            ],
            "a correction and a repair are different bills"
        );
        assert_eq!(
            recovery_turn_counts(&db, "worker").unwrap(),
            vec![(RECOVERY_TASK_RETRY.to_owned(), 1)]
        );

        // And they reach the feed the user can actually read, one row per turn,
        // kept apart by kind. A turn nobody can see is the problem, not the fix.
        let visible: Vec<(String, String)> = {
            let mut statement = db
                .prepare("SELECT kind,entity_id FROM events WHERE source='recovery' ORDER BY id")
                .unwrap();
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        };
        assert_eq!(
            visible,
            vec![
                (RECOVERY_CORRECTION.to_owned(), "chat".to_owned()),
                (RECOVERY_CORRECTION.to_owned(), "chat".to_owned()),
                (RECOVERY_REPAIR.to_owned(), "chat".to_owned()),
                (RECOVERY_TASK_RETRY.to_owned(), "worker".to_owned()),
            ]
        );
    }
}
