//! The Work board: what needs doing, read from SQLite and nothing else.
//!
//! [`board`] is the whole read path behind `work/get_work_board`, and it is
//! **store-only** by construction: it takes a `&Connection`, not a
//! `&Arc<BridgeCore>`, so it cannot reach an adapter map, a connector, or a
//! process spawner even by accident. Anything the board needs that SQLite does
//! not already hold is observed elsewhere and served from `work_fact_cache`
//! with the timestamp of that observation.
//!
//! The DTOs are `bridge_protocol::messages`' Work types used directly. A second
//! copy in this crate would only create something to drift.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension};

use bridge_protocol::messages as wire;

use crate::{BridgeError, WORKER_APPROVAL_TIMEOUT_SECONDS};

/// The defaults a board reports when Work has never been configured. Chosen to
/// match the epic's stated operational bounds: a 10-minute run deadline inside
/// a 15-minute lease, and a cooldown that cannot be shorter than the lease.
pub const DEFAULT_MAX_WALL_SECONDS: i64 = 600;
pub const DEFAULT_MAX_TURNS: i64 = 12;
pub const DEFAULT_MAX_TOOL_CALLS: i64 = 24;
pub const DEFAULT_COOLDOWN_MINUTES: i64 = 15;

/// Where Work's configuration lives in `configuration_entries`.
const SETTINGS_KIND: &str = "work";
const SETTINGS_ID: &str = "settings";

/// Work's configuration before anyone has configured it: facts only, no model,
/// no connector, no cadence.
pub fn default_settings() -> wire::WorkSettings {
    wire::WorkSettings {
        briefing: None,
        enabled_connector_instances: Vec::new(),
        refresh_on_focus: false,
        refresh_interval_minutes: None,
        cooldown_minutes: DEFAULT_COOLDOWN_MINUTES,
        limits: wire::WorkBriefLimits {
            max_wall_seconds: DEFAULT_MAX_WALL_SECONDS,
            max_turns: DEFAULT_MAX_TURNS,
            max_tool_calls: DEFAULT_MAX_TOOL_CALLS,
            max_output_tokens: None,
            cost_ceiling_microusd: None,
        },
    }
}

/// Read Work's settings. `Ok(None)` means never configured; `Err` means a row
/// exists that this contract cannot read, which the caller degrades rather than
/// failing the whole board over.
fn stored_settings(db: &Connection) -> Result<Option<wire::WorkSettings>, BridgeError> {
    // `optional()`, not `ok()`: an absent row means "never configured", while a
    // database error means the read failed and must not read as the same thing.
    let payload: Option<String> = db
        .query_row(
            "SELECT payload FROM configuration_entries WHERE kind=?1 AND id=?2",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID],
            |row| row.get(0),
        )
        .optional()?;
    let Some(payload) = payload else {
        return Ok(None);
    };
    serde_json::from_str(&payload)
        .map(Some)
        .map_err(|error| BridgeError::Invalid(format!("stored Work settings are invalid: {error}")))
}

// ---------------------------------------------------------------------------
// Fact projection
// ---------------------------------------------------------------------------

/// How deep to walk a delegation chain looking for the session a human is
/// actually being asked something on. `MAX_DEPTH` for delegation is 3, so this
/// is generous; it exists to bound the recursion, not to express a policy.
const MAX_ANCESTOR_WALK: i64 = 16;

/// A projected fact plus the instant its timestamp parsed to. The instant is a
/// sort key only — the fact carries the text so a board never invents precision
/// a stored timestamp did not have.
struct Projected {
    fact: wire::WorkFact,
    at: DateTime<Utc>,
}

fn instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

fn projected(fact: wire::WorkFact) -> Projected {
    // An unparseable timestamp sorts last within its severity rather than
    // first: it is unknown, and unknown is not "oldest".
    let at = instant(&fact.actionable_at).unwrap_or(DateTime::<Utc>::MAX_UTC);
    Projected { fact, at }
}

/// Sort into the board's documented order and drop repeats.
///
/// Severity, then oldest actionable first, then key. The key tie-break is what
/// makes the order total: two facts can never compare equal, so the board does
/// not depend on the order SQLite happened to return rows in.
fn finalize(mut projections: Vec<Projected>) -> Vec<wire::WorkFact> {
    projections.sort_by(|left, right| {
        left.fact
            .severity
            .cmp(&right.fact.severity)
            .then(left.at.cmp(&right.at))
            .then(left.fact.dedupe_key.cmp(&right.fact.dedupe_key))
    });
    let mut seen = std::collections::HashSet::new();
    projections
        .into_iter()
        .filter(|projection| seen.insert(projection.fact.dedupe_key.clone()))
        .map(|projection| projection.fact)
        .collect()
}

/// Failed completion checks on attempts nobody has resolved.
///
/// An attempt that is verified, waived, or superseded is settled; its old failed
/// check rows are history, not work. A required check blocks — the completion
/// gate will not pass without it — while an optional one only warrants a look.
fn failed_completion_checks(db: &Connection) -> Result<Vec<Projected>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT c.attempt_id,c.check_id,c.required,c.detail,
                COALESCE(c.completed_at,c.started_at,a.started_at),a.session_id
           FROM eval_check_runs c
           JOIN eval_attempts a ON a.id=c.attempt_id
          WHERE c.status='failed'
            AND a.status NOT IN ('verified','waived','superseded')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, bool>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (attempt_id, check_id, required, detail, at, session_id) = row?;
        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::FailedCompletionCheck,
            dedupe_key: format!("completion-check:{attempt_id}:{check_id}"),
            severity: if required {
                wire::WorkFactSeverity::Blocking
            } else {
                wire::WorkFactSeverity::Attention
            },
            title: format!("{check_id} failed"),
            detail,
            target: wire::WorkFactTarget::CompletionAttempt {
                session_id: session_id.clone(),
                attempt_id: attempt_id.clone(),
            },
            actionable_at: at.clone(),
            observed_at: at,
            // The row *is* the observation; there is nothing cached about it.
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::ReviewCompletionCheck {
                session_id,
                attempt_id,
                check_id,
            },
        }));
    }
    Ok(facts)
}

/// Queued workers parked because someone upstream is waiting on a human.
///
/// The action points at the session the approval is actually on, found by
/// walking `parent_session_id` up to the nearest `waiting` ancestor. Pointing it
/// at the queue row would be useless: a queue row is not something a person
/// answers.
fn blocked_queue_items(db: &Connection) -> Result<Vec<Projected>, BridgeError> {
    let mut chain = db.prepare(
        "WITH RECURSIVE chain(queue_id,session_id,depth) AS (
             SELECT id,parent_session_id,0 FROM worker_queue WHERE queue_status='blocked_on_human'
             UNION ALL
             SELECT c.queue_id,s.parent_session_id,c.depth+1
               FROM chain c JOIN sessions s ON s.id=c.session_id
              WHERE s.parent_session_id IS NOT NULL AND c.depth < ?1
         )
         SELECT c.queue_id,c.session_id
           FROM chain c JOIN sessions s ON s.id=c.session_id
          WHERE s.status='waiting'
          ORDER BY c.queue_id,c.depth",
    )?;
    let mut waiting: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let rows = chain.query_map([MAX_ANCESTOR_WALK], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (queue_id, session_id) = row?;
        // Ordered by depth, so the first row for a queue item is the nearest.
        waiting.entry(queue_id).or_insert(session_id);
    }

    let mut statement = db.prepare(
        "SELECT q.id,q.workspace_id,q.parent_session_id,COALESCE(q.blocked_at,q.updated_at),
                COALESCE(s.label,q.parent_session_id)
           FROM worker_queue q
           JOIN sessions s ON s.id=q.parent_session_id
          WHERE q.queue_status='blocked_on_human'",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (queue_id, workspace_id, parent_session_id, at, label) = row?;
        // No `waiting` ancestor recorded means the queue and the session tree
        // disagree. Send the human to the parent rather than nowhere.
        let session_id = waiting
            .get(&queue_id)
            .cloned()
            .unwrap_or_else(|| parent_session_id.clone());
        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::BlockedWorkerQueueItem,
            dedupe_key: format!("worker-queue:{queue_id}"),
            // Queued work is stopped, and only a person restarts it.
            severity: wire::WorkFactSeverity::Blocking,
            title: format!("A queued worker for {label} is waiting on you"),
            detail: Some(
                "the delegation cannot start until the approval above it is answered".to_owned(),
            ),
            target: wire::WorkFactTarget::WorkerQueueItem {
                queue_id,
                workspace_id,
            },
            actionable_at: at.clone(),
            observed_at: at,
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::AnswerApproval {
                session_id,
                approval_sequence: None,
            },
        }));
    }
    Ok(facts)
}

/// Approval requests nobody has answered.
///
/// Two resolution shapes reach the store: adapter approvals carry
/// `requestEventId` (nested under the event envelope's `data`, or top-level),
/// and policy approvals carry `approvalId`. An entry is unresolved when neither
/// match exists.
///
/// Sessions in a terminal state are excluded. Their adapter is gone, so the
/// approval genuinely cannot be answered any more, and showing it would put a
/// button on the board that cannot work.
fn actionable_approvals(db: &Connection, now: DateTime<Utc>) -> Result<Vec<Projected>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT e.session_id,e.sequence,e.created_at,
                COALESCE(json_extract(e.payload,'$.title'),
                         json_extract(e.payload,'$.data.title'),
                         'Approval required')
           FROM session_entries e
           JOIN sessions s ON s.id=e.session_id
          WHERE e.kind='approval.requested'
            AND s.status NOT IN ('stopped','failed','completed','cancelled')
            AND NOT EXISTS (
                SELECT 1 FROM session_entries r
                 WHERE r.session_id=e.session_id AND r.kind='approval.resolved'
                   AND (COALESCE(json_extract(r.payload,'$.data.requestEventId'),
                                 json_extract(r.payload,'$.requestEventId')) = e.sequence
                     OR (COALESCE(json_extract(e.payload,'$.approvalId'),
                                  json_extract(e.payload,'$.data.approvalId')) IS NOT NULL
                         AND COALESCE(json_extract(r.payload,'$.approvalId'),
                                      json_extract(r.payload,'$.data.approvalId'))
                           = COALESCE(json_extract(e.payload,'$.approvalId'),
                                      json_extract(e.payload,'$.data.approvalId'))))
            )",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut facts = Vec::new();
    for row in rows {
        let (session_id, sequence, at, title) = row?;
        // Past the approval deadline the thing it was holding has already been
        // given up on, so it is not merely waiting — it is overdue.
        let expired = instant(&at).is_some_and(|requested| {
            now.signed_duration_since(requested).num_seconds() >= WORKER_APPROVAL_TIMEOUT_SECONDS
        });
        facts.push(projected(wire::WorkFact {
            kind: wire::WorkFactKind::ActionableApproval,
            dedupe_key: format!("approval:{session_id}:{sequence}"),
            severity: if expired {
                wire::WorkFactSeverity::Blocking
            } else {
                wire::WorkFactSeverity::Attention
            },
            title,
            detail: expired.then(|| {
                format!(
                    "unanswered for more than {} minutes",
                    WORKER_APPROVAL_TIMEOUT_SECONDS / 60
                )
            }),
            target: wire::WorkFactTarget::Session {
                session_id: session_id.clone(),
            },
            actionable_at: at.clone(),
            observed_at: at,
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::AnswerApproval {
                session_id,
                approval_sequence: Some(sequence),
            },
        }));
    }
    Ok(facts)
}

/// Every fact, ordered and deduplicated. Store-only.
pub fn facts(db: &Connection, now: DateTime<Utc>) -> Result<Vec<wire::WorkFact>, BridgeError> {
    let mut projections = failed_completion_checks(db)?;
    projections.extend(blocked_queue_items(db)?);
    projections.extend(actionable_approvals(db, now)?);
    Ok(finalize(projections))
}

/// The board. Deterministic, offline, and useful with no model configured.
pub fn board(db: &Connection) -> Result<wire::WorkBoard, BridgeError> {
    let (settings, settings_error) = match stored_settings(db) {
        Ok(Some(settings)) => (settings, None),
        Ok(None) => (default_settings(), None),
        // A configuration row we cannot read is worth saying out loud, but it
        // is not worth withholding every fact over: the facts do not depend on
        // it. Fall back to defaults and let `suggestions` carry the reason.
        Err(_) => (
            default_settings(),
            Some("stored Work settings could not be read; using defaults".to_owned()),
        ),
    };

    // Suggested work is contracted but not yet produced by anything. Until the
    // briefing runner lands, the honest state is "no model is configured" —
    // never an empty list that looks like "nothing to suggest".
    let suggestions = match settings_error {
        Some(detail) => wire::WorkSuggestions {
            state: wire::WorkSuggestionsState::Degraded,
            detail: Some(detail),
        },
        None => wire::WorkSuggestions {
            state: wire::WorkSuggestionsState::NotConfigured,
            detail: Some(
                "no briefing model is configured, so Work is showing facts only".to_owned(),
            ),
        },
    };

    let now = Utc::now();
    Ok(wire::WorkBoard {
        generated_at: now.to_rfc3339(),
        facts: facts(db, now)?,
        tasks: Vec::new(),
        latest_run: None,
        sources: Vec::new(),
        usage: None,
        settings,
        suggestions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn memory_db() -> Connection {
        store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn store_settings(db: &Connection, payload: &str) {
        db.execute(
            "INSERT INTO configuration_entries(kind,id,payload,created_at,updated_at)
             VALUES(?1,?2,?3,'now','now')",
            rusqlite::params![SETTINGS_KIND, SETTINGS_ID, payload],
        )
        .unwrap();
    }

    /// A project, workspace, and session, so the foreign keys the projections
    /// join through are satisfiable.
    fn seed(db: &Connection) {
        db.execute_batch(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Bridge','/tmp/p','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
                 VALUES('w','p','Kyoto','Task','bridge/task','/tmp/w','idle','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source)
                 VALUES('parent','w','codex','Orchestrator','working','reported');",
        )
        .unwrap();
    }

    fn add_session(db: &Connection, id: &str, parent: Option<&str>, status: &str) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id)
             VALUES(?1,'w','codex',?1,?2,'reported',?3)",
            rusqlite::params![id, status, parent],
        )
        .unwrap();
    }

    /// One completion attempt with one check in the given state.
    fn add_check(
        db: &Connection,
        attempt: &str,
        check: &str,
        required: bool,
        status: &str,
        attempt_status: &str,
        completed_at: &str,
    ) {
        db.execute(
            "INSERT OR IGNORE INTO completion_contracts(id,workspace_id,session_id,schema_version,
                 acceptance_criteria,markdown_committed,status,created_at,updated_at)
             VALUES('c','w','parent',1,'[]',0,'open','now','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR IGNORE INTO eval_plans(id,contract_id,schema_version,risk,plan,created_at)
             VALUES('plan','c',1,'low','{}','now')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT OR IGNORE INTO eval_attempts(id,plan_id,session_id,repository_head,dirty_digest,
                 repository_path,status,started_at)
             VALUES(?1,'plan','parent','head','digest','/tmp/w',?2,'2026-08-19T08:00:00+00:00')",
            rusqlite::params![attempt, attempt_status],
        )
        .unwrap();
        db.execute(
            "INSERT INTO eval_check_runs(id,attempt_id,check_id,kind,required,status,executor,
                 detail,artifact_refs,completed_at)
             VALUES(?1,?2,?3,'deterministic',?4,?5,'shell','2 failing tests','[]',?6)",
            rusqlite::params![
                format!("{attempt}:{check}"),
                attempt,
                check,
                required,
                status,
                completed_at
            ],
        )
        .unwrap();
    }

    fn add_blocked_queue_item(db: &Connection, id: &str, parent: &str, blocked_at: &str) {
        db.execute(
            "INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,
                 queue_status,blocked_at,created_at,updated_at)
             VALUES(?1,?2,'w','turn','{}','model','blocked_on_human',?3,'now','now')",
            rusqlite::params![id, parent, blocked_at],
        )
        .unwrap();
    }

    fn add_entry(db: &Connection, session: &str, sequence: i64, kind: &str, payload: &str) {
        db.execute(
            "INSERT INTO session_entries(id,session_id,sequence,kind,payload,created_at)
             VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                format!("{session}-{sequence}"),
                session,
                sequence,
                kind,
                payload,
                "2026-08-19T09:00:00+00:00"
            ],
        )
        .unwrap();
    }

    fn now() -> DateTime<Utc> {
        instant("2026-08-19T09:10:00+00:00").unwrap()
    }

    fn keys(facts: &[wire::WorkFact]) -> Vec<&str> {
        facts.iter().map(|fact| fact.dedupe_key.as_str()).collect()
    }

    #[test]
    fn an_empty_database_projects_no_facts() {
        let db = memory_db();
        assert!(facts(&db, now()).unwrap().is_empty());
        assert!(board(&db).unwrap().facts.is_empty());
    }

    #[test]
    fn a_required_failed_check_blocks_and_an_optional_one_asks_for_attention() {
        let db = memory_db();
        seed(&db);
        add_check(&db, "a-1", "cargo-test", true, "failed", "running", "2026-08-19T08:30:00+00:00");
        add_check(&db, "a-1", "clippy", false, "failed", "running", "2026-08-19T08:31:00+00:00");
        let facts = facts(&db, now()).unwrap();
        assert_eq!(
            keys(&facts),
            vec!["completion-check:a-1:cargo-test", "completion-check:a-1:clippy"]
        );
        assert_eq!(facts[0].severity, wire::WorkFactSeverity::Blocking);
        assert_eq!(facts[1].severity, wire::WorkFactSeverity::Attention);
        assert_eq!(facts[0].kind, wire::WorkFactKind::FailedCompletionCheck);
        assert_eq!(facts[0].actionable_at, "2026-08-19T08:30:00+00:00");
        assert_eq!(facts[0].freshness, wire::WorkFactFreshness::Live);
        assert_eq!(
            facts[0].target,
            wire::WorkFactTarget::CompletionAttempt {
                session_id: "parent".into(),
                attempt_id: "a-1".into()
            }
        );
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::ReviewCompletionCheck {
                session_id: "parent".into(),
                attempt_id: "a-1".into(),
                check_id: "cargo-test".into(),
            },
            "a failed check is reviewed, never cleared"
        );
    }

    #[test]
    fn a_passing_check_and_a_settled_attempt_project_nothing() {
        let db = memory_db();
        seed(&db);
        add_check(&db, "a-1", "cargo-test", true, "passed", "verified", "2026-08-19T08:30:00+00:00");
        assert!(facts(&db, now()).unwrap().is_empty());

        for settled in ["verified", "waived", "superseded"] {
            let db = memory_db();
            seed(&db);
            add_check(
                &db,
                "a-1",
                "cargo-test",
                true,
                "failed",
                settled,
                "2026-08-19T08:30:00+00:00",
            );
            assert!(
                facts(&db, now()).unwrap().is_empty(),
                "a {settled} attempt's failed checks are history, not work"
            );
        }
    }

    #[test]
    fn a_blocked_queue_item_points_at_the_nearest_waiting_ancestor() {
        let db = memory_db();
        seed(&db);
        // parent (working) -> middle (waiting) -> leaf (working); the queue item
        // hangs off the leaf, so the human is being asked on `middle`.
        add_session(&db, "middle", Some("parent"), "waiting");
        add_session(&db, "leaf", Some("middle"), "working");
        add_blocked_queue_item(&db, "q-1", "leaf", "2026-08-19T08:00:00+00:00");
        let facts = facts(&db, now()).unwrap();
        assert_eq!(keys(&facts), vec!["worker-queue:q-1"]);
        assert_eq!(facts[0].severity, wire::WorkFactSeverity::Blocking);
        assert_eq!(
            facts[0].target,
            wire::WorkFactTarget::WorkerQueueItem {
                queue_id: "q-1".into(),
                workspace_id: "w".into()
            }
        );
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::AnswerApproval {
                session_id: "middle".into(),
                approval_sequence: None,
            },
            "the action must land on the session the approval is on, not the queue row"
        );
    }

    #[test]
    fn a_blocked_queue_item_with_no_waiting_ancestor_falls_back_to_its_parent() {
        let db = memory_db();
        seed(&db);
        add_blocked_queue_item(&db, "q-1", "parent", "2026-08-19T08:00:00+00:00");
        let facts = facts(&db, now()).unwrap();
        assert_eq!(
            facts[0].action,
            wire::WorkFactAction::AnswerApproval {
                session_id: "parent".into(),
                approval_sequence: None,
            },
            "the queue and the session tree disagreeing must not produce an action to nowhere"
        );
    }

    #[test]
    fn a_queue_item_that_is_merely_queued_is_not_a_fact() {
        let db = memory_db();
        seed(&db);
        db.execute(
            "INSERT INTO worker_queue(id,parent_session_id,workspace_id,turn_id,request,actual_model,
                 queue_status,created_at,updated_at)
             VALUES('q-1','parent','w','turn','{}','model','queued','now','now')",
            [],
        )
        .unwrap();
        assert!(facts(&db, now()).unwrap().is_empty());
    }

    #[test]
    fn a_young_approval_asks_for_attention_and_an_overdue_one_blocks() {
        let db = memory_db();
        seed(&db);
        add_entry(
            &db,
            "parent",
            4,
            "approval.requested",
            r#"{"title":"Approve command","status":"pending"}"#,
        );
        let young = facts(&db, now()).unwrap();
        assert_eq!(keys(&young), vec!["approval:parent:4"]);
        assert_eq!(young[0].severity, wire::WorkFactSeverity::Attention);
        assert_eq!(young[0].title, "Approve command");
        assert!(young[0].detail.is_none());
        assert_eq!(
            young[0].action,
            wire::WorkFactAction::AnswerApproval {
                session_id: "parent".into(),
                approval_sequence: Some(4),
            }
        );

        // The same request, read an hour after it arrived — past the 30-minute
        // approval deadline.
        let overdue = facts(&db, instant("2026-08-19T10:00:00+00:00").unwrap()).unwrap();
        assert_eq!(overdue[0].severity, wire::WorkFactSeverity::Blocking);
        assert!(overdue[0].detail.as_deref().is_some_and(|d| d.contains("30 minutes")));
    }

    #[test]
    fn a_resolved_approval_projects_nothing_in_either_resolution_shape() {
        // Adapter shape: the resolution names the request's sequence, nested in
        // the normalized event envelope.
        let db = memory_db();
        seed(&db);
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"Approve command"}"#);
        add_entry(
            &db,
            "parent",
            5,
            "approval.resolved",
            r#"{"data":{"requestEventId":4,"decision":"accept"}}"#,
        );
        assert!(facts(&db, now()).unwrap().is_empty());

        // Policy shape: the resolution names the approval id instead.
        let db = memory_db();
        seed(&db);
        add_entry(
            &db,
            "parent",
            6,
            "approval.requested",
            r#"{"title":"Approve delegation write scope","approvalId":"scope-1"}"#,
        );
        add_entry(&db, "parent", 7, "approval.resolved", r#"{"approvalId":"scope-1"}"#);
        assert!(facts(&db, now()).unwrap().is_empty());
    }

    #[test]
    fn a_resolution_for_a_different_approval_does_not_settle_this_one() {
        let db = memory_db();
        seed(&db);
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"First"}"#);
        add_entry(&db, "parent", 6, "approval.requested", r#"{"title":"Second"}"#);
        add_entry(
            &db,
            "parent",
            7,
            "approval.resolved",
            r#"{"data":{"requestEventId":4,"decision":"accept"}}"#,
        );
        assert_eq!(keys(&facts(&db, now()).unwrap()), vec!["approval:parent:6"]);
    }

    #[test]
    fn an_approval_on_a_finished_session_is_not_offered() {
        for terminal in ["stopped", "failed", "completed", "cancelled"] {
            let db = memory_db();
            seed(&db);
            add_session(&db, "child", Some("parent"), terminal);
            add_entry(&db, "child", 4, "approval.requested", r#"{"title":"Approve command"}"#);
            assert!(
                facts(&db, now()).unwrap().is_empty(),
                "a {terminal} session has no adapter left to answer to"
            );
        }
    }

    #[test]
    fn facts_are_ordered_by_severity_then_age_then_key() {
        let db = memory_db();
        seed(&db);
        // An overdue approval and a required failed check are both blocking; the
        // check is older, so it comes first. The optional check is attention.
        add_entry(&db, "parent", 4, "approval.requested", r#"{"title":"Approve command"}"#);
        add_check(&db, "a-1", "cargo-test", true, "failed", "running", "2026-08-19T07:00:00+00:00");
        add_check(&db, "a-1", "clippy", false, "failed", "running", "2026-08-19T06:00:00+00:00");
        let later = instant("2026-08-19T11:00:00+00:00").unwrap();
        let facts = facts(&db, later).unwrap();
        assert_eq!(
            keys(&facts),
            vec![
                "completion-check:a-1:cargo-test",
                "approval:parent:4",
                "completion-check:a-1:clippy",
            ],
            "blocking before attention, and oldest first inside a severity"
        );
    }

    #[test]
    fn the_key_tie_break_makes_the_order_total() {
        // Same severity and same instant: only the key can separate them, and it
        // always does, so the board never depends on SQLite's row order.
        let facts = finalize(vec![
            projected(sample_fact("b", wire::WorkFactSeverity::Attention, "2026-08-19T08:00:00+00:00")),
            projected(sample_fact("a", wire::WorkFactSeverity::Attention, "2026-08-19T08:00:00+00:00")),
        ]);
        assert_eq!(keys(&facts), vec!["a", "b"]);
    }

    #[test]
    fn an_unparseable_timestamp_sorts_last_rather_than_first() {
        let facts = finalize(vec![
            projected(sample_fact("unknown", wire::WorkFactSeverity::Attention, "whenever")),
            projected(sample_fact("known", wire::WorkFactSeverity::Attention, "2026-08-19T08:00:00+00:00")),
        ]);
        assert_eq!(
            keys(&facts),
            vec!["known", "unknown"],
            "an unreadable timestamp is unknown, which is not the same as oldest"
        );
    }

    #[test]
    fn two_projections_of_one_condition_collapse_to_a_single_fact() {
        let facts = finalize(vec![
            projected(sample_fact("same", wire::WorkFactSeverity::Blocking, "2026-08-19T08:00:00+00:00")),
            projected(sample_fact("same", wire::WorkFactSeverity::Blocking, "2026-08-19T09:00:00+00:00")),
            projected(sample_fact("other", wire::WorkFactSeverity::Blocking, "2026-08-19T08:30:00+00:00")),
        ]);
        assert_eq!(keys(&facts), vec!["same", "other"]);
        assert_eq!(
            facts[0].actionable_at, "2026-08-19T08:00:00+00:00",
            "the oldest of a duplicated pair survives, because it sorted first"
        );
    }

    fn sample_fact(key: &str, severity: wire::WorkFactSeverity, at: &str) -> wire::WorkFact {
        wire::WorkFact {
            kind: wire::WorkFactKind::ActionableApproval,
            dedupe_key: key.into(),
            severity,
            title: key.into(),
            detail: None,
            target: wire::WorkFactTarget::Session { session_id: "parent".into() },
            actionable_at: at.into(),
            observed_at: at.into(),
            freshness: wire::WorkFactFreshness::Live,
            action: wire::WorkFactAction::AnswerApproval {
                session_id: "parent".into(),
                approval_sequence: None,
            },
        }
    }

    #[test]
    fn absent_settings_read_as_the_documented_defaults() {
        let db = memory_db();
        let board = board(&db).unwrap();
        assert_eq!(board.settings, default_settings());
        assert!(board.settings.briefing.is_none());
        assert_eq!(board.settings.limits.max_wall_seconds, DEFAULT_MAX_WALL_SECONDS);
        assert_eq!(board.suggestions.state, wire::WorkSuggestionsState::NotConfigured);
    }

    #[test]
    fn a_board_with_no_model_or_connector_is_still_a_board() {
        let db = memory_db();
        let board = board(&db).unwrap();
        assert!(board.tasks.is_empty(), "no briefing runner has produced tasks");
        assert!(board.latest_run.is_none());
        assert!(board.sources.is_empty());
        assert!(board.usage.is_none());
        assert!(!board.generated_at.is_empty());
    }

    #[test]
    fn configured_settings_are_read_back_verbatim() {
        let db = memory_db();
        store_settings(
            &db,
            r#"{"briefing":{"harness":"claude","model":"claude-opus-5","effort":"high"},
                "enabledConnectorInstances":["github:acme"],"refreshOnFocus":true,
                "refreshIntervalMinutes":30,"cooldownMinutes":20,
                "limits":{"maxWallSeconds":300,"maxTurns":6,"maxToolCalls":12,
                          "maxOutputTokens":null,"costCeilingMicrousd":null}}"#,
        );
        let board = board(&db).unwrap();
        let briefing = board.settings.briefing.as_ref().expect("briefing profile");
        assert_eq!(briefing.harness.as_str(), "claude");
        assert_eq!(briefing.model, "claude-opus-5");
        assert_eq!(board.settings.enabled_connector_instances, vec!["github:acme".to_owned()]);
        assert_eq!(board.settings.cooldown_minutes, 20);
        assert_eq!(board.settings.limits.max_turns, 6);
    }

    #[test]
    fn settings_that_cannot_be_read_degrade_instead_of_failing_the_board() {
        let db = memory_db();
        // An unknown field is exactly what a rolled-back binary would meet.
        store_settings(
            &db,
            r#"{"briefing":null,"enabledConnectorInstances":[],"refreshOnFocus":false,
                "refreshIntervalMinutes":null,"cooldownMinutes":15,
                "limits":{"maxWallSeconds":600,"maxTurns":12,"maxToolCalls":24,
                          "maxOutputTokens":null,"costCeilingMicrousd":null},
                "writeConnectorTools":true}"#,
        );
        let board = board(&db).unwrap();
        assert_eq!(board.settings, default_settings(), "defaults, not a half-read payload");
        assert_eq!(board.suggestions.state, wire::WorkSuggestionsState::Degraded);
        assert!(board
            .suggestions
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("could not be read")));
    }
}
