//! The attention ledger: what a person should know about, decided once, in core.
//!
//! The desktop webview used to work this out by diffing two session-list
//! snapshots, so no other client could see the decision, a worker finishing
//! pinged the person, and a worker approval (which never moves the worker's
//! status to `waiting`) pinged nobody. Here the decision is made where history
//! is written: [`store::append_session_entry_tx`](crate::store) calls
//! [`record_in_transaction`] for every forest entry, so a row commits or rolls
//! back with the entry that caused it. Imports and forks copy history with
//! direct inserts and never pass through it, so old history raises nothing.
//!
//! Clients learn that the ledger moved from the `attention/changed` refetch
//! hint and read it with [`list`]. The hint carries no rows, for the same
//! reason `memory-changed` carries no records: a client that rebuilt state
//! from a notification would keep showing what the ledger already closed.
//!
//! Only rows for the person exist so far (`audience = 'human'`). The column is
//! there for orchestrator wait conditions, which will route worker outcomes to
//! an agent instead.

use crate::BridgeError;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

const HUMAN: &str = "human";
pub const DEFAULT_LIST_LIMIT: u32 = 100;
pub const MAX_LIST_LIMIT: u32 = 500;

pub fn install_store(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    // AUTOINCREMENT, not a plain rowid: the sequence is every client's cursor,
    // and a reused number after a delete would hide a row from a client whose
    // cursor already passed it. Kinds are not CHECK-constrained because the
    // next slices add kinds, and SQLite cannot alter a CHECK in place.
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS attention_events(
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            id TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL,
            severity TEXT NOT NULL,
            audience TEXT NOT NULL DEFAULT 'human',
            session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
            approval_event_id INTEGER,
            title TEXT NOT NULL,
            body TEXT,
            created_at TEXT NOT NULL,
            resolved_at TEXT,
            seen_at TEXT,
            superseded_by INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_attention_events_session
            ON attention_events(session_id, audience, sequence);",
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionKind {
    /// A top-level chat finished a turn.
    Finished,
    /// A top-level chat's turn ended in an error.
    Failed,
    /// Any visible chat, workers included, is waiting on a person.
    NeedsYou,
    /// Input queued for a chat whose adapter is gone, waiting past the bound.
    /// Raised by [`escalate_stranded`], not by a forest entry.
    Escalation,
}

impl AttentionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::NeedsYou => "needs_you",
            Self::Escalation => "escalation",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "finished" => Some(Self::Finished),
            "failed" => Some(Self::Failed),
            "needs_you" => Some(Self::NeedsYou),
            "escalation" => Some(Self::Escalation),
            _ => None,
        }
    }

    pub const fn severity(self) -> Severity {
        match self {
            Self::Finished => Severity::Informational,
            Self::Failed | Self::NeedsYou | Self::Escalation => Severity::Actionable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Informational,
    Actionable,
}

impl Severity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Informational => "informational",
            Self::Actionable => "actionable",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "informational" => Some(Self::Informational),
            "actionable" => Some(Self::Actionable),
            _ => None,
        }
    }
}

/// What a forest entry does to the ledger, judged from its kind and status
/// alone. Everything that is not [`Effect::Ignore`] is a trigger kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Open(AttentionKind),
    /// A request was answered; close the row it opened.
    Close,
    Ignore,
}

/// Forest kinds that ask a person for something. `delegation.requested` is not
/// one: it means the launch was queued (`RouteDecision::Queue`), and a launch
/// that needs approval is written as `approval.requested`.
const REQUEST_KINDS: &[&str] = &["approval.requested", "permission.requested", "question.requested"];

/// Forest kinds that answer one of those requests, by a person or by policy.
const RESOLUTION_KINDS: &[&str] = &[
    "approval.resolved",
    "approval.settled",
    "permission.resolved",
    "permission.denied",
    "question.resolved",
    "question.rejected",
    "delegation.approved",
    "delegation.rejected",
];

/// Pure and allocation-free, because the forest append runs it for every
/// streamed tool event: anything that is not a trigger kind returns here,
/// before any SQL.
pub fn effect(kind: &str, status: Option<&str>) -> Effect {
    if kind == "turn.completed" {
        return match status {
            Some("completed" | "end_turn") => Effect::Open(AttentionKind::Finished),
            Some(status) if status == "failed" || status.starts_with("error") => {
                Effect::Open(AttentionKind::Failed)
            }
            // Cancelled and interrupted turns were stopped by the person, who
            // does not need telling.
            _ => Effect::Ignore,
        };
    }
    if REQUEST_KINDS.contains(&kind) {
        return Effect::Open(AttentionKind::NeedsYou);
    }
    if RESOLUTION_KINDS.contains(&kind) {
        return Effect::Close;
    }
    Effect::Ignore
}

/// Whether a forest kind can move the ledger at all, whatever its status. The
/// event bus uses it to decide when to send `attention/changed`.
pub fn is_trigger_kind(kind: &str) -> bool {
    kind == "turn.completed" || REQUEST_KINDS.contains(&kind) || RESOLUTION_KINDS.contains(&kind)
}

/// The session facts classification needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFacts {
    pub kind: Option<String>,
    pub parent_session_id: Option<String>,
    pub title: Option<String>,
    pub label: String,
    pub harness: String,
}

impl SessionFacts {
    fn load(db: &Connection, session_id: &str) -> Result<Option<Self>, BridgeError> {
        Ok(db
            .query_row(
                "SELECT kind, parent_session_id, title, label, harness FROM sessions WHERE id=?1",
                params![session_id],
                |row| {
                    Ok(Self {
                        kind: row.get(0)?,
                        parent_session_id: row.get(1)?,
                        title: row.get(2)?,
                        label: row.get(3)?,
                        harness: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    fn is_worker(&self) -> bool {
        self.parent_session_id.as_deref().is_some_and(|parent| !parent.is_empty())
    }
}

/// The effect after looking at who the session is. Hidden kinds (briefings,
/// suggestions, connector polls and the like) never reach a person, and a
/// worker's finished or failed turn belongs to its orchestrator, which already
/// receives a `bridge-worker-result`. A worker asking for approval still
/// reaches the person: nobody else can answer it.
pub fn classify(effect: Effect, session: Option<&SessionFacts>) -> Effect {
    let Some(session) = session else { return Effect::Ignore };
    if crate::work_briefing_config::is_hidden_session_kind(session.kind.as_deref()) {
        return Effect::Ignore;
    }
    match effect {
        Effect::Open(AttentionKind::Finished | AttentionKind::Failed) if session.is_worker() => Effect::Ignore,
        other => other,
    }
}

/// Mirrors `PLACEHOLDER_LABELS` and `chatName` in `src/components/sidebarChats.ts`.
fn chat_name(session: &SessionFacts) -> String {
    if let Some(title) = session.title.as_deref().map(str::trim).filter(|title| !title.is_empty()) {
        return title.to_owned();
    }
    let label = session.label.trim();
    if matches!(
        label.to_ascii_lowercase().as_str(),
        "orchestrator" | "bridge orchestrator" | "new chat" | "chat" | ""
    ) {
        "New chat".to_owned()
    } else {
        label.to_owned()
    }
}

/// Mirrors `harnessLabel` in `src/utils.ts`.
fn harness_label(harness: &str) -> String {
    match harness {
        "bridge" => "Bridge".into(),
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "cursor" => "Cursor".into(),
        "opencode" => "OpenCode".into(),
        "pi" => "Pi".into(),
        "shell" => "Shell".into(),
        "" => "Agent".into(),
        other => {
            let mut characters = other.chars();
            characters
                .next()
                .map(|first| first.to_uppercase().chain(characters).collect())
                .unwrap_or_default()
        }
    }
}

/// Server-side copy, matching `src/attentionCopy.ts`, so every client says the
/// same thing about the same row.
pub fn copy(kind: AttentionKind, session: &SessionFacts) -> (String, String) {
    let who = format!("{} · {}", chat_name(session), harness_label(&session.harness));
    match kind {
        AttentionKind::NeedsYou => ("Bridge needs you".into(), format!("{who} is waiting for your input")),
        AttentionKind::Failed => ("Turn failed".into(), format!("{who} ended this turn with an error")),
        AttentionKind::Finished => ("Turn completed".into(), format!("{who} finished its turn")),
        AttentionKind::Escalation => escalation_copy(session, 1),
    }
}

/// Copy for a stranded queue: who stopped, and how much is waiting for it.
pub fn escalation_copy(session: &SessionFacts, waiting: i64) -> (String, String) {
    let who = format!("{} · {}", chat_name(session), harness_label(&session.harness));
    let updates = if waiting == 1 { "1 update".to_owned() } else { format!("{waiting} updates") };
    ("A result is waiting".into(), format!("{who} stopped before it read {updates}"))
}

/// The request a resolution answers, when it says. Typed entries carry
/// `requestEventId` at the root; adapter events carry it under `data`.
fn answered_request(payload: &serde_json::Value) -> Option<i64> {
    payload
        .get("requestEventId")
        .or_else(|| payload.pointer("/data/requestEventId"))
        .and_then(serde_json::Value::as_i64)
        .filter(|sequence| *sequence > 0)
}

/// Apply one forest entry to the ledger, inside the caller's transaction.
/// Returns whether a row was opened or closed.
pub fn record_in_transaction(
    transaction: &Transaction<'_>,
    session_id: &str,
    kind: &str,
    payload: &serde_json::Value,
    entry_sequence: i64,
) -> Result<bool, BridgeError> {
    let status = payload.get("status").and_then(serde_json::Value::as_str);
    let effect = effect(kind, status);
    if effect == Effect::Ignore {
        return Ok(false);
    }
    let session = SessionFacts::load(transaction, session_id)?;
    match classify(effect, session.as_ref()) {
        Effect::Ignore => Ok(false),
        Effect::Open(attention) => {
            let session = session.expect("classify ignores unknown sessions");
            open(transaction, session_id, attention, &session, entry_sequence)?;
            Ok(true)
        }
        Effect::Close => Ok(close(transaction, session_id, answered_request(payload))? > 0),
    }
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn open(
    db: &Connection,
    session_id: &str,
    kind: AttentionKind,
    session: &SessionFacts,
    entry_sequence: i64,
) -> Result<i64, BridgeError> {
    let (title, body) = copy(kind, session);
    let approval_event_id = (kind == AttentionKind::NeedsYou).then_some(entry_sequence);
    insert_row(db, session_id, kind, &title, &body, approval_event_id)
}

fn insert_row(
    db: &Connection,
    session_id: &str,
    kind: AttentionKind,
    title: &str,
    body: &str,
    approval_event_id: Option<i64>,
) -> Result<i64, BridgeError> {
    db.execute(
        "INSERT INTO attention_events(id,kind,severity,audience,session_id,approval_event_id,title,body,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            Uuid::new_v4().to_string(),
            kind.as_str(),
            kind.severity().as_str(),
            HUMAN,
            session_id,
            approval_event_id,
            title,
            body,
            now(),
        ],
    )?;
    let sequence = db.last_insert_rowid();
    // Newest per session wins. A row the person already saw stays as it was.
    db.execute(
        "UPDATE attention_events SET superseded_by=?1
         WHERE session_id=?2 AND audience=?3 AND sequence<?1 AND seen_at IS NULL AND superseded_by IS NULL",
        params![sequence, session_id, HUMAN],
    )?;
    Ok(sequence)
}

fn close(db: &Connection, session_id: &str, request: Option<i64>) -> Result<usize, BridgeError> {
    let at = now();
    let changed = match request {
        Some(request) => db.execute(
            "UPDATE attention_events SET resolved_at=?1
             WHERE session_id=?2 AND audience=?3 AND kind='needs_you' AND resolved_at IS NULL
               AND approval_event_id=?4",
            params![at, session_id, HUMAN, request],
        )?,
        // No request id (Claude's `permission.denied`, Cursor's settle):
        // close whatever this session still has open.
        None => db.execute(
            "UPDATE attention_events SET resolved_at=?1
             WHERE session_id=?2 AND audience=?3 AND kind='needs_you' AND resolved_at IS NULL",
            params![at, session_id, HUMAN],
        )?,
    };
    Ok(changed)
}

/// How long queued input may wait for a chat with no live adapter before the
/// person is told.
pub const STRANDED_AFTER: chrono::Duration = chrono::Duration::seconds(30);
/// Older leftovers are left alone, so an upgrade does not raise a storm of
/// escalations for input that went stale long ago.
pub const STRANDED_LOOKBACK: chrono::Duration = chrono::Duration::hours(24);

/// Tell the person about queued input that is waiting on a chat whose adapter
/// is gone, and close what has since been delivered. Called by the queued-input
/// sweep; never starts a provider. Returns whether the ledger changed.
///
/// One open escalation per chat: a seen but unresolved one is not raised again,
/// because the person already knows. It closes once that chat's queue is empty,
/// whichever route emptied it.
pub fn escalate_stranded(
    db: &Connection,
    live_adapters: &std::collections::HashSet<String>,
    now: chrono::DateTime<Utc>,
) -> Result<bool, BridgeError> {
    let at = now.to_rfc3339();
    let stranded: Vec<(String, i64)> = {
        let mut statement = db.prepare(
            "SELECT q.session_id, COUNT(*) FROM queued_session_input q
             JOIN sessions s ON s.id=q.session_id
             WHERE q.state='queued' AND s.archived_at IS NULL
               AND julianday(q.created_at) <= julianday(?1)
               AND julianday(q.created_at) >= julianday(?2)
             GROUP BY q.session_id ORDER BY q.session_id",
        )?;
        let rows = statement.query_map(
            params![(now - STRANDED_AFTER).to_rfc3339(), (now - STRANDED_LOOKBACK).to_rfc3339()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        rows.collect::<Result<_, _>>()?
    };
    let mut changed = false;
    for (session_id, waiting) in stranded {
        if live_adapters.contains(&session_id) {
            continue;
        }
        let Some(session) = SessionFacts::load(db, &session_id)? else { continue };
        if crate::work_briefing_config::is_hidden_session_kind(session.kind.as_deref()) {
            continue;
        }
        let open: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM attention_events
             WHERE session_id=?1 AND audience=?2 AND kind='escalation' AND resolved_at IS NULL)",
            params![session_id, HUMAN],
            |row| row.get(0),
        )?;
        if open {
            continue;
        }
        let (title, body) = escalation_copy(&session, waiting);
        insert_row(db, &session_id, AttentionKind::Escalation, &title, &body, None)?;
        changed = true;
    }
    let closed = db.execute(
        "UPDATE attention_events SET resolved_at=?1
         WHERE audience=?2 AND kind='escalation' AND resolved_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM queued_session_input q
                           WHERE q.session_id=attention_events.session_id AND q.state='queued')",
        params![at, HUMAN],
    )?;
    Ok(changed || closed > 0)
}

/// One ledger row as clients see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionRow {
    pub sequence: i64,
    pub id: String,
    pub kind: AttentionKind,
    pub severity: Severity,
    pub session_id: Option<String>,
    pub approval_event_id: Option<i64>,
    pub title: String,
    pub body: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub seen_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionPage {
    pub items: Vec<AttentionRow>,
    pub latest_sequence: i64,
}

/// The person's rows after `after_sequence`, oldest first, without superseded
/// ones. `latest_sequence` lets a fresh client start its cursor without paging.
pub fn list(
    db: &Connection,
    after_sequence: Option<i64>,
    limit: Option<u32>,
    unseen_only: bool,
) -> Result<AttentionPage, BridgeError> {
    let limit = limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT);
    let mut statement = db.prepare(
        "SELECT sequence,id,kind,severity,session_id,approval_event_id,title,body,created_at,resolved_at,seen_at
         FROM attention_events
         WHERE audience=?1 AND sequence>?2 AND superseded_by IS NULL AND (?3=0 OR seen_at IS NULL)
         ORDER BY sequence ASC LIMIT ?4",
    )?;
    let rows = statement.query_map(
        params![HUMAN, after_sequence.unwrap_or(0), unseen_only, limit],
        |row| {
            let kind: String = row.get(2)?;
            let severity: String = row.get(3)?;
            Ok((
                kind,
                severity,
                AttentionRow {
                    sequence: row.get(0)?,
                    id: row.get(1)?,
                    kind: AttentionKind::Finished,
                    severity: Severity::Informational,
                    session_id: row.get(4)?,
                    approval_event_id: row.get(5)?,
                    title: row.get(6)?,
                    body: row.get(7)?,
                    created_at: row.get(8)?,
                    resolved_at: row.get(9)?,
                    seen_at: row.get(10)?,
                },
            ))
        },
    )?;
    let mut items = Vec::new();
    for row in rows {
        let (kind, severity, mut item) = row?;
        // A kind this build does not know came from a newer one sharing the
        // data directory; skipping it beats failing the whole read.
        let (Some(kind), Some(severity)) = (AttentionKind::parse(&kind), Severity::parse(&severity)) else {
            continue;
        };
        item.kind = kind;
        item.severity = severity;
        items.push(item);
    }
    Ok(AttentionPage { items, latest_sequence: latest_sequence(db)? })
}

pub fn latest_sequence(db: &Connection) -> Result<i64, BridgeError> {
    Ok(db.query_row("SELECT COALESCE(MAX(sequence),0) FROM attention_events", [], |row| row.get(0))?)
}

/// Stamp every unseen row of one session as seen. Returns how many it stamped.
pub fn mark_seen(db: &Connection, session_id: &str) -> Result<usize, BridgeError> {
    Ok(db.execute(
        "UPDATE attention_events SET seen_at=?1 WHERE session_id=?2 AND audience=?3 AND seen_at IS NULL",
        params![now(), session_id, HUMAN],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE sessions(id TEXT PRIMARY KEY, title TEXT, label TEXT NOT NULL, harness TEXT NOT NULL,
                kind TEXT, parent_session_id TEXT);",
        )
        .unwrap();
        let transaction = db.unchecked_transaction().unwrap();
        install_store(&transaction).unwrap();
        transaction.commit().unwrap();
        db
    }

    fn session(db: &Connection, id: &str, kind: &str, parent: Option<&str>, title: Option<&str>) {
        db.execute(
            "INSERT INTO sessions(id,title,label,harness,kind,parent_session_id) VALUES(?1,?2,'Orchestrator','codex',?3,?4)",
            params![id, title, kind, parent],
        )
        .unwrap();
    }

    fn record(db: &Connection, session_id: &str, kind: &str, payload: serde_json::Value, sequence: i64) -> bool {
        let transaction = db.unchecked_transaction().unwrap();
        let changed = record_in_transaction(&transaction, session_id, kind, &payload, sequence).unwrap();
        transaction.commit().unwrap();
        changed
    }

    fn rows(db: &Connection) -> Vec<(String, String, Option<i64>, bool, bool)> {
        db.prepare(
            "SELECT session_id,kind,approval_event_id,resolved_at IS NOT NULL,superseded_by IS NOT NULL
             FROM attention_events ORDER BY sequence",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
    }

    #[test]
    fn classify_table() {
        use AttentionKind::*;
        let cases: &[(&str, Option<&str>, Effect)] = &[
            ("turn.completed", Some("completed"), Effect::Open(Finished)),
            ("turn.completed", Some("end_turn"), Effect::Open(Finished)),
            ("turn.completed", Some("failed"), Effect::Open(Failed)),
            ("turn.completed", Some("error_during_execution"), Effect::Open(Failed)),
            ("turn.completed", Some("error_sidecar"), Effect::Open(Failed)),
            ("turn.completed", Some("cancelled"), Effect::Ignore),
            ("turn.completed", Some("interrupted"), Effect::Ignore),
            ("turn.completed", None, Effect::Ignore),
            ("approval.requested", None, Effect::Open(NeedsYou)),
            ("permission.requested", Some("settling"), Effect::Open(NeedsYou)),
            ("question.requested", None, Effect::Open(NeedsYou)),
            ("delegation.requested", None, Effect::Ignore),
            ("approval.resolved", None, Effect::Close),
            ("approval.settled", Some("selected"), Effect::Close),
            ("permission.resolved", Some("allowed_for_session"), Effect::Close),
            ("permission.denied", Some("denied"), Effect::Close),
            ("question.resolved", Some("answered"), Effect::Close),
            ("question.rejected", None, Effect::Close),
            ("delegation.approved", None, Effect::Close),
            ("delegation.rejected", None, Effect::Close),
            ("turn.started", None, Effect::Ignore),
            ("tool.completed", Some("completed"), Effect::Ignore),
            ("assistant.message", None, Effect::Ignore),
            ("session.status", Some("ready"), Effect::Ignore),
        ];
        let top = SessionFacts {
            kind: Some("orchestrator".into()),
            parent_session_id: None,
            title: None,
            label: "Orchestrator".into(),
            harness: "codex".into(),
        };
        let worker = SessionFacts { parent_session_id: Some("parent".into()), ..top.clone() };
        for (kind, status, expected) in cases {
            let raw = effect(kind, *status);
            assert_eq!(raw, *expected, "{kind} {status:?}");
            assert_eq!(is_trigger_kind(kind), *kind == "turn.completed" || *expected != Effect::Ignore, "{kind}");
            assert_eq!(classify(raw, Some(&top)), *expected, "top-level {kind} {status:?}");
            let for_worker = match expected {
                Effect::Open(Finished | Failed) => Effect::Ignore,
                other => *other,
            };
            assert_eq!(classify(raw, Some(&worker)), for_worker, "worker {kind} {status:?}");
        }
    }

    #[test]
    fn hidden_and_unknown_sessions_write_nothing() {
        let db = store();
        for kind in ["briefing", "suggestion", "extraction", "outcome_evaluation", "consolidation", "connector", "chat_search"] {
            session(&db, kind, kind, None, None);
            assert!(!record(&db, kind, "turn.completed", json!({"status": "completed"}), 3));
            assert!(!record(&db, kind, "permission.requested", json!({}), 4));
        }
        assert!(!record(&db, "missing", "turn.completed", json!({"status": "completed"}), 3));
        assert!(rows(&db).is_empty());
    }

    #[test]
    fn worker_finishing_writes_nothing_but_its_approval_does() {
        let db = store();
        session(&db, "parent", "orchestrator", None, None);
        session(&db, "child", "orchestrator", Some("parent"), None);
        assert!(!record(&db, "child", "turn.completed", json!({"status": "completed"}), 9));
        assert!(!record(&db, "child", "turn.completed", json!({"status": "failed"}), 10));
        assert!(record(&db, "child", "permission.requested", json!({}), 11));
        assert!(record(&db, "parent", "turn.completed", json!({"status": "completed"}), 4));
        assert_eq!(
            rows(&db),
            vec![
                ("child".into(), "needs_you".into(), Some(11), false, false),
                ("parent".into(), "finished".into(), None, false, false),
            ]
        );
    }

    #[test]
    fn newest_row_supersedes_older_unseen_rows() {
        let db = store();
        session(&db, "s", "direct", None, None);
        session(&db, "other", "direct", None, None);
        record(&db, "s", "permission.requested", json!({}), 2);
        record(&db, "other", "turn.completed", json!({"status": "completed"}), 2);
        record(&db, "s", "turn.completed", json!({"status": "completed"}), 5);
        assert_eq!(
            rows(&db),
            vec![
                ("s".into(), "needs_you".into(), Some(2), false, true),
                ("other".into(), "finished".into(), None, false, false),
                ("s".into(), "finished".into(), None, false, false),
            ]
        );
        let page = list(&db, None, None, false).unwrap();
        assert_eq!(page.items.iter().map(|row| row.kind).collect::<Vec<_>>(), vec![AttentionKind::Finished, AttentionKind::Finished]);
    }

    #[test]
    fn seen_rows_are_not_superseded() {
        let db = store();
        session(&db, "s", "direct", None, None);
        record(&db, "s", "turn.completed", json!({"status": "completed"}), 2);
        assert_eq!(mark_seen(&db, "s").unwrap(), 1);
        record(&db, "s", "turn.completed", json!({"status": "failed"}), 4);
        assert_eq!(
            rows(&db),
            vec![
                ("s".into(), "finished".into(), None, false, false),
                ("s".into(), "failed".into(), None, false, false),
            ]
        );
    }

    #[test]
    fn resolution_closes_open_needs_you_rows() {
        let db = store();
        session(&db, "a", "direct", None, None);
        session(&db, "b", "direct", None, None);
        // Typed entries carry the request at the root, adapter events under data.
        record(&db, "a", "approval.requested", json!({}), 7);
        assert!(record(&db, "a", "approval.resolved", json!({"requestEventId": 7}), 8));
        record(&db, "b", "permission.requested", json!({}), 3);
        assert!(!record(&db, "b", "permission.resolved", json!({"data": {"requestEventId": 99}}), 4));
        assert!(record(&db, "b", "permission.resolved", json!({"data": {"requestEventId": 3}}), 5));
        // No request id at all: close whatever the session still has open.
        record(&db, "b", "permission.requested", json!({}), 6);
        assert!(record(&db, "b", "permission.denied", json!({"data": {"tool_name": "Bash"}}), 7));
        assert!(!record(&db, "a", "question.resolved", json!({}), 9));
        let resolved: Vec<bool> = rows(&db).into_iter().map(|row| row.3).collect();
        assert_eq!(resolved, vec![true, true, true]);
    }

    #[test]
    fn copy_uses_title_label_and_harness() {
        let mut facts = SessionFacts {
            kind: Some("direct".into()),
            parent_session_id: None,
            title: Some("Fix login".into()),
            label: "Chat".into(),
            harness: "claude".into(),
        };
        assert_eq!(copy(AttentionKind::NeedsYou, &facts), ("Bridge needs you".into(), "Fix login · Claude is waiting for your input".into()));
        facts.title = Some("  ".into());
        assert_eq!(copy(AttentionKind::Finished, &facts).1, "New chat · Claude finished its turn");
        facts.label = "Release notes".into();
        facts.harness = "grok".into();
        assert_eq!(copy(AttentionKind::Failed, &facts), ("Turn failed".into(), "Release notes · Grok ended this turn with an error".into()));
        facts.harness = "opencode".into();
        assert_eq!(copy(AttentionKind::Finished, &facts).1, "Release notes · OpenCode finished its turn");
    }

    #[test]
    fn list_pages_ascending_and_filters_unseen() {
        let db = store();
        for id in ["a", "b", "c"] {
            session(&db, id, "direct", None, None);
            record(&db, id, "turn.completed", json!({"status": "completed"}), 2);
        }
        mark_seen(&db, "b").unwrap();
        let all = list(&db, None, None, false).unwrap();
        assert_eq!(all.latest_sequence, 3);
        assert_eq!(all.items.iter().map(|row| row.sequence).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(list(&db, Some(1), Some(1), false).unwrap().items[0].sequence, 2);
        let unseen = list(&db, None, None, true).unwrap();
        assert_eq!(unseen.items.iter().map(|row| row.session_id.clone().unwrap()).collect::<Vec<_>>(), vec!["a", "c"]);
        assert!(list(&db, Some(3), None, false).unwrap().items.is_empty());
        assert_eq!(list(&db, None, Some(0), false).unwrap().items.len(), 1, "limit clamps to at least one");
        let empty = store();
        assert_eq!(list(&empty, None, None, false).unwrap().latest_sequence, 0);
    }

    #[test]
    fn mark_seen_stamps_only_that_session() {
        let db = store();
        session(&db, "a", "direct", None, None);
        session(&db, "b", "direct", None, None);
        record(&db, "a", "turn.completed", json!({"status": "completed"}), 2);
        record(&db, "b", "turn.completed", json!({"status": "completed"}), 2);
        assert_eq!(mark_seen(&db, "a").unwrap(), 1);
        assert_eq!(mark_seen(&db, "a").unwrap(), 0);
        let page = list(&db, None, None, false).unwrap();
        assert!(page.items[0].seen_at.is_some());
        assert!(page.items[1].seen_at.is_none());
    }

    #[test]
    fn rolled_back_write_leaves_no_row() {
        let db = store();
        session(&db, "s", "direct", None, None);
        let transaction = db.unchecked_transaction().unwrap();
        assert!(record_in_transaction(&transaction, "s", "turn.completed", &json!({"status": "completed"}), 2).unwrap());
        drop(transaction);
        assert!(rows(&db).is_empty());
    }

    fn queue_store() -> Connection {
        let db = store();
        db.execute_batch(
            "ALTER TABLE sessions ADD COLUMN archived_at TEXT;
             CREATE TABLE queued_session_input(id TEXT PRIMARY KEY, sequence INTEGER, session_id TEXT NOT NULL,
                provider_text TEXT, display_text TEXT, state TEXT NOT NULL, created_at TEXT NOT NULL, delivered_at TEXT);",
        )
        .unwrap();
        db
    }

    fn queued(db: &Connection, id: &str, session_id: &str, state: &str, age: chrono::Duration, now: chrono::DateTime<Utc>) {
        db.execute(
            "INSERT INTO queued_session_input(id,sequence,session_id,provider_text,display_text,state,created_at) VALUES(?1,1,?2,'{}','worker report',?3,?4)",
            params![id, session_id, state, (now - age).to_rfc3339()],
        )
        .unwrap();
    }

    fn escalations(db: &Connection) -> Vec<(String, String, bool)> {
        db.prepare("SELECT session_id, body, resolved_at IS NOT NULL FROM attention_events WHERE kind='escalation' ORDER BY sequence")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn stranded_session_escalates_once() {
        let db = queue_store();
        let now = Utc::now();
        session(&db, "orch", "orchestrator", None, Some("Ship it"));
        queued(&db, "q1", "orch", "queued", chrono::Duration::seconds(45), now);
        queued(&db, "q2", "orch", "queued", chrono::Duration::seconds(40), now);
        let live = std::collections::HashSet::new();
        assert!(escalate_stranded(&db, &live, now).unwrap());
        assert!(!escalate_stranded(&db, &live, now + chrono::Duration::seconds(2)).unwrap(), "one open escalation per chat");
        assert_eq!(escalations(&db), vec![("orch".to_owned(), "Ship it · Codex stopped before it read 2 updates".to_owned(), false)]);
        let page = list(&db, None, None, false).unwrap();
        assert_eq!(page.items[0].kind, AttentionKind::Escalation);
        assert_eq!(page.items[0].severity, Severity::Actionable);
        assert_eq!(page.items[0].title, "A result is waiting");
    }

    #[test]
    fn live_adapter_archived_hidden_and_old_rows_do_not_escalate() {
        let db = queue_store();
        let now = Utc::now();
        session(&db, "live", "orchestrator", None, None);
        session(&db, "archived", "orchestrator", None, None);
        session(&db, "hidden", "briefing", None, None);
        session(&db, "fresh", "orchestrator", None, None);
        session(&db, "stale", "orchestrator", None, None);
        session(&db, "delivered", "orchestrator", None, None);
        db.execute("UPDATE sessions SET archived_at='2026-11-01T00:00:00+00:00' WHERE id='archived'", []).unwrap();
        for id in ["live", "archived", "hidden"] {
            queued(&db, &format!("q-{id}"), id, "queued", chrono::Duration::minutes(5), now);
        }
        queued(&db, "q-fresh", "fresh", "queued", chrono::Duration::seconds(10), now);
        queued(&db, "q-stale", "stale", "queued", chrono::Duration::hours(30), now);
        queued(&db, "q-delivered", "delivered", "delivered", chrono::Duration::minutes(5), now);
        queued(&db, "q-gone", "no-such-session", "queued", chrono::Duration::minutes(5), now);
        let live: std::collections::HashSet<String> = ["live".to_owned()].into();
        assert!(!escalate_stranded(&db, &live, now).unwrap());
        assert!(escalations(&db).is_empty());
    }

    #[test]
    fn seen_escalation_is_not_raised_again() {
        let db = queue_store();
        let now = Utc::now();
        session(&db, "orch", "orchestrator", None, None);
        queued(&db, "q1", "orch", "queued", chrono::Duration::minutes(1), now);
        let live = std::collections::HashSet::new();
        escalate_stranded(&db, &live, now).unwrap();
        mark_seen(&db, "orch").unwrap();
        queued(&db, "q2", "orch", "queued", chrono::Duration::seconds(50), now);
        assert!(!escalate_stranded(&db, &live, now).unwrap());
        assert_eq!(escalations(&db).len(), 1);
    }

    #[test]
    fn escalation_closes_when_the_queue_empties() {
        let db = queue_store();
        let now = Utc::now();
        session(&db, "orch", "orchestrator", None, None);
        queued(&db, "q1", "orch", "queued", chrono::Duration::minutes(1), now);
        let live = std::collections::HashSet::new();
        escalate_stranded(&db, &live, now).unwrap();
        db.execute("UPDATE queued_session_input SET state='delivered' WHERE id='q1'", []).unwrap();
        assert!(escalate_stranded(&db, &live, now).unwrap(), "closing is a change");
        assert_eq!(escalations(&db), vec![("orch".to_owned(), "New chat · Codex stopped before it read 1 update".to_owned(), true)]);
        // A later strand raises a new one.
        queued(&db, "q2", "orch", "queued", chrono::Duration::minutes(1), now);
        assert!(escalate_stranded(&db, &live, now).unwrap());
        assert_eq!(escalations(&db).iter().filter(|row| !row.2).count(), 1);
    }

    #[test]
    fn escalation_copy_counts_updates() {
        let facts = SessionFacts { kind: Some("orchestrator".into()), parent_session_id: None, title: Some("Ship it".into()), label: "Orchestrator".into(), harness: "claude".into() };
        assert_eq!(escalation_copy(&facts, 1), ("A result is waiting".into(), "Ship it · Claude stopped before it read 1 update".into()));
        assert_eq!(escalation_copy(&facts, 3).1, "Ship it · Claude stopped before it read 3 updates");
    }
}
