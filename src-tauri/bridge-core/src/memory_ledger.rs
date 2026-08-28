//! Explicit named-scope pins. Not session recall, not the learning router.
//!
//! The only writable scope in this release is `account:local`, set on save
//! (never taken from the client, never SQL NULL). Listing always filters
//! `scope_key = ?`. `task_knowledge` is dropped without copying rows.

use crate::secret_interception;
use crate::BridgeError;
use bridge_protocol::messages::{
    ListMemoryRecordsResult, MemoryRecord, ACCOUNT_MEMORY_SCOPE, MAX_MEMORY_BODY_CHARS,
    MAX_MEMORY_LIST_LIMIT,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

const KIND_PREFERENCE: &str = "preference";
const KIND_FACT: &str = "fact";
const KIND_DECISION: &str = "decision";
const KIND_CONSTRAINT: &str = "constraint";
const PROVENANCE_USER_EXPLICIT: &str = "user_explicit";
const STATUS_ACTIVE: &str = "active";
const STATUS_PROPOSED: &str = "proposed";
const STATUS_REJECTED: &str = "rejected";
const STATUS_SUPERSEDED: &str = "superseded";
const STATUS_DELETED: &str = "deleted";
/// Reached only by the sweep. Distinct from superseded, which names a
/// successor, and from deleted, which is the user saying to forget.
pub const STATUS_EXPIRED: &str = "expired";

pub fn account_memory_scope() -> &'static str {
    ACCOUNT_MEMORY_SCOPE
}

pub(crate) fn install_ledger(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_records (
            id TEXT PRIMARY KEY,
            scope_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            body TEXT NOT NULL,
            provenance TEXT NOT NULL,
            status TEXT NOT NULL,
            source_session_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_records_scope_status
            ON memory_records(scope_key, status, updated_at);
        -- task_knowledge never had a production reader (only a round-trip unit
        -- test). Dropping it does not SELECT/INSERT those rows into memory_records.
        DROP TABLE IF EXISTS task_knowledge;",
    )?;
    Ok(())
}

/// Schema 32: the lifecycle extraction and packets cannot land without.
/// Chain columns for supersession, and an FTS index over active bodies that
/// every removal path (tombstone, reject, supersede) purges by trigger.
pub(crate) fn install_lifecycle(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::store::add_column_if_missing(transaction, "memory_records", "supersedes", "TEXT")?;
    crate::store::add_column_if_missing(transaction, "memory_records", "superseded_by", "TEXT")?;
    transaction.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memory_record_fts USING fts5(
            record_id UNINDEXED,
            scope_key UNINDEXED,
            body,
            tokenize = 'unicode61 remove_diacritics 2'
        );
        DROP TRIGGER IF EXISTS memory_records_ai_fts;
        DROP TRIGGER IF EXISTS memory_records_ad_fts;
        DROP TRIGGER IF EXISTS memory_records_au_fts;
        CREATE TRIGGER memory_records_ai_fts AFTER INSERT ON memory_records
        WHEN NEW.status = 'active'
        BEGIN
          INSERT INTO memory_record_fts(record_id, scope_key, body)
          VALUES (NEW.id, NEW.scope_key, NEW.body);
        END;
        CREATE TRIGGER memory_records_ad_fts AFTER DELETE ON memory_records
        BEGIN
          DELETE FROM memory_record_fts WHERE record_id = OLD.id;
        END;
        CREATE TRIGGER memory_records_au_fts AFTER UPDATE OF status, body, scope_key
        ON memory_records
        BEGIN
          DELETE FROM memory_record_fts WHERE record_id = OLD.id;
          INSERT INTO memory_record_fts(record_id, scope_key, body)
          SELECT NEW.id, NEW.scope_key, NEW.body
          WHERE NEW.status = 'active';
        END;
        INSERT INTO memory_record_fts(record_id, scope_key, body)
        SELECT id, scope_key, body FROM memory_records
        WHERE status = 'active'
          AND id NOT IN (SELECT record_id FROM memory_record_fts);",
    )?;
    Ok(())
}

/// Schema 33 (record half): trust fields arrive with their first honest
/// producer, the extractor. Explicit saves keep both NULL.
pub(crate) fn install_trust_fields(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    crate::store::add_column_if_missing(transaction, "memory_records", "confidence_bps", "INTEGER")?;
    crate::store::add_column_if_missing(transaction, "memory_records", "rationale", "TEXT")?;
    Ok(())
}

/// Schema 42 (record half): validity stops being a flag.
///
/// `valid_from` is the instant a record's claim began to hold and `valid_to`
/// the instant it stopped; an active record's end is open. A record that never
/// reached active carries an empty interval, `valid_to = valid_from`, so the
/// as-of read is pure containment and needs no status list beside it.
///
/// The backfill opens every existing interval at creation, leaves every active
/// record open, and closes the rest at the last instant the row was touched —
/// which for a superseded or tombstoned record is when it stopped holding.
/// `max` guards the one case that would produce a backwards interval, a row
/// whose `updated_at` predates its `created_at`.
pub(crate) fn install_validity_intervals(
    transaction: &Transaction<'_>,
) -> Result<(), BridgeError> {
    crate::store::add_column_if_missing(transaction, "memory_records", "valid_from", "TEXT")?;
    crate::store::add_column_if_missing(transaction, "memory_records", "valid_to", "TEXT")?;
    crate::store::add_column_if_missing(transaction, "memory_records", "expires_at", "TEXT")?;
    crate::store::add_column_if_missing(transaction, "memory_records", "conflict_group", "TEXT")?;
    transaction.execute_batch(
        "UPDATE memory_records SET valid_from = created_at WHERE valid_from IS NULL;
         UPDATE memory_records
            SET valid_to = CASE
                WHEN status = 'active' THEN NULL
                WHEN status IN ('proposed','rejected') THEN valid_from
                ELSE max(valid_from, updated_at)
            END
          WHERE valid_to IS NULL;
         CREATE INDEX IF NOT EXISTS idx_memory_records_scope_interval
             ON memory_records(scope_key, valid_from, valid_to);
         CREATE INDEX IF NOT EXISTS idx_memory_records_expiry
             ON memory_records(status, expires_at);
         CREATE INDEX IF NOT EXISTS idx_memory_records_conflict
             ON memory_records(scope_key, conflict_group, status);",
    )?;
    Ok(())
}

/// Reject empty, whitespace, and anything that is not a named scope.
pub fn parse_scope_key(raw: &str) -> Result<String, BridgeError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BridgeError::Invalid(
            "Memory scope is required; it cannot be empty or NULL".into(),
        ));
    }
    if trimmed == ACCOUNT_MEMORY_SCOPE {
        return Ok(ACCOUNT_MEMORY_SCOPE.to_string());
    }
    if let Some(workspace_id) = trimmed.strip_prefix("workspace:") {
        if workspace_id.is_empty() || workspace_id.chars().all(char::is_whitespace) {
            return Err(BridgeError::Invalid(
                "Workspace memory scope needs a workspace id after workspace:".into(),
            ));
        }
        return Ok(format!("workspace:{workspace_id}"));
    }
    Err(BridgeError::Invalid(format!(
        "Unknown memory scope '{trimmed}'. Account pins use {ACCOUNT_MEMORY_SCOPE}."
    )))
}

pub(crate) fn parse_kind(raw: Option<&str>) -> Result<&'static str, BridgeError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(KIND_PREFERENCE),
        Some(KIND_PREFERENCE) => Ok(KIND_PREFERENCE),
        Some(KIND_FACT) => Ok(KIND_FACT),
        Some(KIND_DECISION) => Ok(KIND_DECISION),
        Some(KIND_CONSTRAINT) => Ok(KIND_CONSTRAINT),
        Some(other) => Err(BridgeError::Invalid(format!(
            "Unknown memory kind '{other}'. Use preference, fact, decision, or constraint."
        ))),
    }
}

pub(crate) fn require_body(body: &str) -> Result<String, BridgeError> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err(BridgeError::Invalid(
            "A memory pin needs some text. Empty bodies are not stored.".into(),
        ));
    }
    if trimmed.chars().count() > MAX_MEMORY_BODY_CHARS {
        return Err(BridgeError::Invalid(format!(
            "A memory pin can be at most {MAX_MEMORY_BODY_CHARS} characters."
        )));
    }
    let intercepted = secret_interception::intercept(trimmed);
    if !intercepted.sanitized.interceptions.is_empty() {
        return Err(BridgeError::Invalid(
            "Memory pins cannot store credentials. Nothing was saved.".into(),
        ));
    }
    Ok(trimmed.to_string())
}

fn require_writable_account_scope() -> String {
    ACCOUNT_MEMORY_SCOPE.to_string()
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    Ok(MemoryRecord {
        id: row.get(0)?,
        scope_key: row.get(1)?,
        kind: row.get(2)?,
        body: row.get(3)?,
        provenance: row.get(4)?,
        status: row.get(5)?,
        source_session_id: row.get(6)?,
        confidence_bps: row.get::<_, Option<i64>>(7)?.map(|value| value as u32),
        rationale: row.get(8)?,
        supersedes: row.get(9)?,
        valid_from: row.get(10)?,
        valid_to: row.get(11)?,
        expires_at: row.get(12)?,
        conflict_group: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

const RECORD_COLUMNS: &str = "id, scope_key, kind, body, provenance, status, source_session_id, \
     confidence_bps, rationale, supersedes, valid_from, valid_to, expires_at, conflict_group, \
     created_at, updated_at";

fn session_exists(db: &Connection, session_id: &str) -> Result<bool, BridgeError> {
    let found: Option<i64> = db
        .query_row(
            "SELECT 1 FROM sessions WHERE id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

pub fn save(
    db: &Connection,
    body: &str,
    kind: Option<&str>,
    source_session_id: Option<&str>,
) -> Result<MemoryRecord, BridgeError> {
    let body = require_body(body)?;
    let kind = parse_kind(kind)?;
    let scope_key = require_writable_account_scope();
    let source_session_id = match source_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        None => None,
        Some(session_id) => {
            if !session_exists(db, session_id)? {
                return Err(BridgeError::Invalid(format!(
                    "Session '{session_id}' does not exist"
                )));
            }
            Some(session_id.to_string())
        }
    };
    crate::memory_consolidation::enforce_scope_budget(db, &scope_key)?;
    let now = Utc::now().to_rfc3339();
    let record = MemoryRecord {
        id: Uuid::new_v4().to_string(),
        scope_key,
        kind: kind.to_string(),
        body,
        provenance: PROVENANCE_USER_EXPLICIT.to_string(),
        status: STATUS_ACTIVE.to_string(),
        source_session_id,
        confidence_bps: None,
        rationale: None,
        supersedes: None,
        valid_from: now.clone(),
        valid_to: None,
        // An explicit save is the user saying this holds until they say
        // otherwise. Nothing the ledger derives puts an end on it.
        expires_at: None,
        conflict_group: None,
        created_at: now.clone(),
        updated_at: now,
    };
    db.execute(
        "INSERT INTO memory_records(
            id, scope_key, kind, body, provenance, status, source_session_id,
            valid_from, created_at, updated_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            record.id,
            record.scope_key,
            record.kind,
            record.body,
            record.provenance,
            record.status,
            record.source_session_id,
            record.valid_from,
            record.created_at,
            record.updated_at,
        ],
    )?;
    Ok(record)
}

pub fn list(
    db: &Connection,
    scope_key: &str,
    status: Option<&str>,
) -> Result<ListMemoryRecordsResult, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let status = match status.map(str::trim).filter(|value| !value.is_empty()) {
        None => STATUS_ACTIVE,
        Some(STATUS_ACTIVE) => STATUS_ACTIVE,
        Some(STATUS_PROPOSED) => STATUS_PROPOSED,
        Some(other) => {
            return Err(BridgeError::Invalid(format!(
                "Memory list can show active or proposed records, not '{other}'."
            )))
        }
    };
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS}
         FROM memory_records
         WHERE scope_key=?1 AND status=?2
         ORDER BY updated_at DESC, id DESC
         LIMIT ?3",
    ))?;
    let records = statement
        .query_map(
            params![scope_key, status, MAX_MEMORY_LIST_LIMIT as i64],
            map_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ListMemoryRecordsResult { scope_key, records })
}

pub fn forget(db: &Connection, record_id: &str) -> Result<MemoryRecord, BridgeError> {
    let record_id = record_id.trim();
    if record_id.is_empty() {
        return Err(BridgeError::Invalid(
            "Unpin needs a memory record id.".into(),
        ));
    }
    let updated_at = Utc::now().to_rfc3339();
    // A tombstone is the claim ceasing to hold, so it closes the interval at
    // the same instant. History still answers what the user believed before.
    let changed = db.execute(
        "UPDATE memory_records SET status=?1, valid_to=?2, updated_at=?2 WHERE id=?3 AND status=?4",
        params![STATUS_DELETED, updated_at, record_id, STATUS_ACTIVE],
    )?;
    if changed == 0 {
        return Err(BridgeError::Invalid(
            "That memory pin is not active (unknown id or already forgotten).".into(),
        ));
    }
    load(db, record_id)?.ok_or_else(|| {
        BridgeError::Invalid(
            "That memory pin is not active (unknown id or already forgotten).".into(),
        )
    })
}

fn load(db: &Connection, record_id: &str) -> Result<Option<MemoryRecord>, BridgeError> {
    db.query_row(
        &format!("SELECT {RECORD_COLUMNS} FROM memory_records WHERE id=?1"),
        params![record_id],
        map_row,
    )
    .optional()
    .map_err(BridgeError::from)
}

/// The extractor's only write path. Whatever a model claimed, what lands is
/// `proposed` / `model_proposal` — the gate in memory_extraction has already
/// validated body, kind, confidence, and rationale before this runs.
pub(crate) fn insert_proposal(
    db: &Connection,
    scope_key: &str,
    body: &str,
    kind: &str,
    confidence_bps: Option<u32>,
    rationale: Option<&str>,
    source_session_id: &str,
) -> Result<MemoryRecord, BridgeError> {
    crate::memory_consolidation::enforce_scope_budget(db, scope_key)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    // A proposal has never held, so its interval is empty rather than open:
    // an as-of read of any instant must not return something nobody approved.
    db.execute(
        "INSERT INTO memory_records(
            id, scope_key, kind, body, provenance, status, source_session_id,
            confidence_bps, rationale, valid_from, valid_to, created_at, updated_at
         ) VALUES(?1,?2,?3,?4,'model_proposal','proposed',?5,?6,?7,?8,?8,?8,?8)",
        params![
            id,
            scope_key,
            kind,
            body,
            source_session_id,
            confidence_bps.map(|value| value as i64),
            rationale,
            now,
        ],
    )?;
    load(db, &id)?.ok_or_else(|| BridgeError::Invalid("The proposal was not written.".into()))
}

/// Case-insensitive body match against every non-deleted record in scope, so
/// the extractor cannot re-propose what already exists in any state but gone.
pub(crate) fn body_already_known(
    db: &Connection,
    scope_key: &str,
    body: &str,
) -> Result<bool, BridgeError> {
    let found: Option<i64> = db
        .query_row(
            "SELECT 1 FROM memory_records
             WHERE scope_key=?1 AND status<>'deleted' AND lower(trim(body))=lower(trim(?2))
             LIMIT 1",
            params![scope_key, body],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// `proposed -> active`. The only path to active a proposal has, and the
/// instant the record's claim starts holding: approval opens the interval that
/// the proposal was written with closed.
pub fn approve(db: &Connection, record_id: &str) -> Result<MemoryRecord, BridgeError> {
    let record = transition(db, record_id, STATUS_PROPOSED, STATUS_ACTIVE, "approve")?;
    db.execute(
        "UPDATE memory_records SET valid_from=?2, valid_to=NULL WHERE id=?1",
        params![record.id, record.updated_at],
    )?;
    load(db, &record.id)?
        .ok_or_else(|| BridgeError::Invalid("The approved record was not written.".into()))
}

/// `proposed -> rejected`. Terminal short of a fresh proposal.
pub fn reject(db: &Connection, record_id: &str) -> Result<MemoryRecord, BridgeError> {
    transition(db, record_id, STATUS_PROPOSED, STATUS_REJECTED, "reject")
}

fn transition(
    db: &Connection,
    record_id: &str,
    from: &str,
    to: &str,
    verb: &str,
) -> Result<MemoryRecord, BridgeError> {
    let record_id = record_id.trim();
    if record_id.is_empty() {
        return Err(BridgeError::Invalid(format!(
            "Memory {verb} needs a record id."
        )));
    }
    let updated_at = Utc::now().to_rfc3339();
    let changed = db.execute(
        "UPDATE memory_records SET status=?1, updated_at=?2 WHERE id=?3 AND status=?4",
        params![to, updated_at, record_id, from],
    )?;
    if changed == 0 {
        return Err(BridgeError::Invalid(format!(
            "Only a {from} memory record can be {to}; '{record_id}' is not one."
        )));
    }
    load(db, record_id)?.ok_or_else(|| {
        BridgeError::Invalid(format!("Memory record '{record_id}' does not exist."))
    })
}

/// Edit. A new active record carries `supersedes`; the old row is stamped
/// `superseded` with `superseded_by` — never an UPDATE of a body in place.
/// Both rows stay readable history; only the new one lists and indexes.
pub fn supersede(
    db: &Connection,
    record_id: &str,
    body: &str,
    kind: Option<&str>,
) -> Result<MemoryRecord, BridgeError> {
    let old = load(db, record_id.trim())?.ok_or_else(|| {
        BridgeError::Invalid(format!("Memory record '{record_id}' does not exist."))
    })?;
    if old.status != STATUS_ACTIVE {
        return Err(BridgeError::Invalid(
            "Only an active memory record can be superseded.".into(),
        ));
    }
    let body = require_body(body)?;
    let kind = match kind {
        None => old.kind.clone(),
        Some(raw) => parse_kind(Some(raw))?.to_string(),
    };
    let now = Utc::now().to_rfc3339();
    let transaction = db.unchecked_transaction()?;
    let replacement_id = write_supersession(
        &transaction,
        &[old.clone()],
        &old.scope_key,
        &body,
        &kind,
        PROVENANCE_USER_EXPLICIT,
        old.source_session_id.as_deref(),
        old.conflict_group.as_deref(),
        &now,
    )?;
    transaction.commit()?;
    load(db, &replacement_id)?.ok_or_else(|| {
        BridgeError::Invalid("The superseding record was not written.".into())
    })
}

/// One successor, any number of predecessors, one instant.
///
/// This is the whole temporal contract in one place: the successor's interval
/// opens exactly where every predecessor's closes, so there is no gap in which
/// nothing held and no overlap in which two things did. A merge is expressed
/// as a supersession of every record it replaces, which is why the predecessor
/// list is a slice — provenance and the chain survive it, and each source stays
/// reachable through its own `superseded_by`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_supersession(
    transaction: &Transaction<'_>,
    predecessors: &[MemoryRecord],
    scope_key: &str,
    body: &str,
    kind: &str,
    provenance: &str,
    source_session_id: Option<&str>,
    conflict_group: Option<&str>,
    at: &str,
) -> Result<String, BridgeError> {
    let replacement_id = Uuid::new_v4().to_string();
    transaction.execute(
        "INSERT INTO memory_records(
            id, scope_key, kind, body, provenance, status, source_session_id,
            valid_from, created_at, updated_at, supersedes, conflict_group
         ) VALUES(?1,?2,?3,?4,?5,'active',?6,?7,?7,?7,?8,?9)",
        params![
            replacement_id,
            scope_key,
            kind,
            body,
            provenance,
            source_session_id,
            at,
            predecessors.first().map(|record| record.id.clone()),
            conflict_group,
        ],
    )?;
    for predecessor in predecessors {
        transaction.execute(
            "UPDATE memory_records
             SET status=?1, superseded_by=?2, valid_to=?3, updated_at=?3
             WHERE id=?4 AND status=?5",
            params![
                STATUS_SUPERSEDED,
                replacement_id,
                at,
                predecessor.id,
                STATUS_ACTIVE
            ],
        )?;
    }
    Ok(replacement_id)
}

/// The scope as it stood at one instant: exactly the records whose validity
/// interval contains it.
///
/// The interval is half-open, `[valid_from, valid_to)`, so a record replaced at
/// an instant is not returned for that instant and its successor is — the
/// boundary belongs to whichever claim held afterwards. Read as of now this
/// returns the active set, which is why nothing that reads the ledger today
/// changes behaviour.
pub fn list_as_of(
    db: &Connection,
    scope_key: &str,
    at: DateTime<Utc>,
) -> Result<ListMemoryRecordsResult, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let at = at.to_rfc3339();
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS}
         FROM memory_records
         WHERE scope_key=?1 AND valid_from <= ?2 AND (valid_to IS NULL OR valid_to > ?2)
         ORDER BY updated_at DESC, id DESC
         LIMIT ?3",
    ))?;
    let records = statement
        .query_map(params![scope_key, at, MAX_MEMORY_LIST_LIMIT as i64], map_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ListMemoryRecordsResult { scope_key, records })
}

/// Move every active record whose expiry has arrived to `expired`, closing its
/// interval at the expiry rather than at the moment the sweep ran.
///
/// The instant is an argument and never a clock read inside the query, so a
/// record expiring exactly at the sweep instant is expired and one expiring
/// after it is not — a boundary a test can pin. Passing an expiry is a
/// lifecycle transition, not a deletion: the body, the provenance and the
/// interval all survive, so the user can still see why it stopped applying.
/// Leaving the index needs no second mechanism, because the FTS triggers
/// already key on active status.
pub fn sweep_expired(db: &Connection, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let now = now.to_rfc3339();
    let mut statement = db.prepare(
        "SELECT id FROM memory_records
         WHERE status=?1 AND expires_at IS NOT NULL AND expires_at <= ?2
         ORDER BY expires_at, id",
    )?;
    let expiring: Vec<String> = statement
        .query_map(params![STATUS_ACTIVE, now], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if expiring.is_empty() {
        return Ok(expiring);
    }
    db.execute(
        "UPDATE memory_records
         SET status=?1, valid_to=expires_at, updated_at=?2
         WHERE status=?3 AND expires_at IS NOT NULL AND expires_at <= ?2",
        params![STATUS_EXPIRED, now, STATUS_ACTIVE],
    )?;
    Ok(expiring)
}

/// Put an end on an active record without closing it now. The sweep is what
/// applies it, so setting one is reversible by the user until it arrives.
pub(crate) fn set_expiry(
    db: &Connection,
    record_id: &str,
    expires_at: &str,
    at: &str,
) -> Result<bool, BridgeError> {
    let changed = db.execute(
        "UPDATE memory_records SET expires_at=?2, updated_at=?3 WHERE id=?1 AND status=?4",
        params![record_id, expires_at, at, STATUS_ACTIVE],
    )?;
    Ok(changed > 0)
}

/// Put competing claims about one subject into a group and leave one of them
/// answering for it.
///
/// At most one member of a group is active at a time, so activating a member
/// closes whichever member was active exactly where the survivor's claim
/// continues — the same boundary rule supersession follows, and the reason a
/// packet can never carry two contradictory facts. The survivor is the most
/// recently touched active member: it is the scope's latest word on the
/// subject. A group whose members all expire or are rejected simply has no
/// active member, which is a legible state and not an error.
pub(crate) fn assign_conflict_group(
    db: &Connection,
    scope_key: &str,
    conflict_group: &str,
    targets: &[MemoryRecord],
    at: &str,
) -> Result<String, BridgeError> {
    let mut members: Vec<MemoryRecord> = targets.to_vec();
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM memory_records
         WHERE scope_key=?1 AND conflict_group=?2 AND status=?3",
    ))?;
    let existing = statement
        .query_map(params![scope_key, conflict_group, STATUS_ACTIVE], map_row)?
        .collect::<Result<Vec<_>, _>>()?;
    for record in existing {
        if !members.iter().any(|known| known.id == record.id) {
            members.push(record);
        }
    }
    // Chosen before anything is stamped: assigning the group touches every
    // member's `updated_at`, so deciding afterwards would be deciding by
    // whichever row the loop happened to write last.
    let survivor = members
        .iter()
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then(left.created_at.cmp(&right.created_at))
                .then(left.id.cmp(&right.id))
        })
        .map(|record| record.id.clone())
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Conflict group '{conflict_group}' has no active member to answer for it."
            ))
        })?;
    let transaction = db.unchecked_transaction()?;
    for member in &members {
        transaction.execute(
            "UPDATE memory_records SET conflict_group=?2, updated_at=?3
             WHERE id=?1 AND scope_key=?4",
            params![member.id, conflict_group, at, scope_key],
        )?;
    }
    transaction.execute(
        "UPDATE memory_records
         SET status=?1, superseded_by=?2, valid_to=?3, updated_at=?3
         WHERE scope_key=?4 AND conflict_group=?5 AND status=?6 AND id<>?2",
        params![
            STATUS_SUPERSEDED,
            survivor,
            at,
            scope_key,
            conflict_group,
            STATUS_ACTIVE
        ],
    )?;
    transaction.commit()?;
    Ok(survivor)
}

/// Every active record in one scope, oldest first. The consolidation job's
/// entire view of the world.
pub(crate) fn active_records(
    db: &Connection,
    scope_key: &str,
    limit: i64,
) -> Result<Vec<MemoryRecord>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM memory_records
         WHERE scope_key=?1 AND status=?2
         ORDER BY created_at, id
         LIMIT ?3",
    ))?;
    let records = statement
        .query_map(params![scope_key, STATUS_ACTIVE, limit], map_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

pub(crate) fn load_record(
    db: &Connection,
    record_id: &str,
) -> Result<Option<MemoryRecord>, BridgeError> {
    load(db, record_id)
}

/// FTS over active pins in one scope. Same guarded query shape as session
/// recall: phrase-quoted tokens, scope bound in SQL, bounded limit.
pub fn search(
    db: &Connection,
    scope_key: &str,
    query: &str,
    limit: Option<u32>,
) -> Result<Vec<MemoryRecord>, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let match_query = crate::session_recall::fts_match_query(query)
        .map_err(|_| BridgeError::Invalid("Memory search needs a word to look for.".into()))?;
    let limit = match limit {
        None => 20,
        Some(0) | Some(51..) => {
            return Err(BridgeError::Invalid(format!(
                "Memory search limit must be between 1 and {MAX_MEMORY_LIST_LIMIT}."
            )))
        }
        Some(value) => value as i64,
    };
    let mut statement = db.prepare(
        "SELECT m.id, m.scope_key, m.kind, m.body, m.provenance, m.status,
                m.source_session_id, m.confidence_bps, m.rationale,
                m.supersedes, m.valid_from, m.valid_to, m.expires_at, m.conflict_group,
                m.created_at, m.updated_at
         FROM memory_record_fts f
         JOIN memory_records m ON m.id = f.record_id
         WHERE f.scope_key = ?1 AND memory_record_fts MATCH ?2 AND m.status = ?3
         ORDER BY rank, m.updated_at DESC
         LIMIT ?4",
    )?;
    let records = statement
        .query_map(
            params![scope_key, match_query, STATUS_ACTIVE, limit],
            map_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(records)
}

/// Slash `/unpin` may pass a unique prefix of an active `account:local` id.
pub fn forget_by_selector(db: &Connection, selector: &str) -> Result<MemoryRecord, BridgeError> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(BridgeError::Invalid(
            "Usage: /unpin <id>. `/pins` lists ids.".into(),
        ));
    }
    if let Some(record) = load(db, selector)? {
        if record.status == STATUS_ACTIVE {
            return forget(db, &record.id);
        }
    }
    let mut statement = db.prepare(
        "SELECT id FROM memory_records
         WHERE scope_key=?1 AND status=?2 AND id LIKE ?3
         ORDER BY id",
    )?;
    let pattern = format!("{selector}%");
    let ids: Vec<String> = statement
        .query_map(
            params![ACCOUNT_MEMORY_SCOPE, STATUS_ACTIVE, pattern],
            |row| row.get(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    match ids.as_slice() {
        [id] => forget(db, id),
        [] => Err(BridgeError::Invalid(
            "No active account pin matches that id.".into(),
        )),
        _ => Err(BridgeError::Invalid(
            "That id prefix matches more than one pin. Use more of the id from `/pins`.".into(),
        )),
    }
}

pub fn format_saved(record: &MemoryRecord) -> String {
    format!(
        "Saved to account memory on this machine (`{}`, id {}). This is not this chat's history and not the helper picker.",
        record.scope_key,
        short_id(&record.id)
    )
}

pub fn format_list(result: &ListMemoryRecordsResult) -> String {
    if result.records.is_empty() {
        return format!(
            "No active pins in `{}`. `/pin <text>` saves one. This is not session recall.",
            result.scope_key
        );
    }
    let mut out = format!(
        "Account memory in `{}` ({}):\n",
        result.scope_key,
        result.records.len()
    );
    for record in &result.records {
        let body = record.body.replace('\n', " ");
        out.push_str(&format!(
            "\n- {} · {}\n  {}\n",
            short_id(&record.id),
            record.kind,
            body
        ));
    }
    out.push_str("\n`/unpin <id>` forgets a pin. Session history is unchanged.");
    out
}

pub fn format_forgotten(record: &MemoryRecord) -> String {
    format!(
        "Forgot pin {} from `{}`. Session history is unchanged.",
        short_id(&record.id),
        record.scope_key
    )
}

fn short_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;
    use rusqlite::params;

    fn ledger_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn insert_proposed(db: &Connection, id: &str, body: &str) {
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                 valid_from, valid_to, created_at, updated_at)
             VALUES(?1, ?2, 'preference', ?3, 'model_proposal', 'proposed', 'now', 'now', 'now', 'now')",
            params![id, ACCOUNT_MEMORY_SCOPE, body],
        )
        .unwrap();
    }

    fn chain_columns(db: &Connection, id: &str) -> (Option<String>, Option<String>) {
        db.query_row(
            "SELECT supersedes, superseded_by FROM memory_records WHERE id=?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn proposed_reaches_active_only_through_approve() {
        let (_dir, db) = ledger_db();
        insert_proposed(&db, "p1", "Proposed convention");
        assert!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.is_empty());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "convention", None)
            .unwrap()
            .is_empty());
        let approved = approve(&db, "p1").unwrap();
        assert_eq!(approved.status, "active");
        assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.len(), 1);
        assert_eq!(
            search(&db, ACCOUNT_MEMORY_SCOPE, "convention", None)
                .unwrap()
                .len(),
            1,
            "approval is what makes a proposal searchable"
        );
        assert!(approve(&db, "p1").is_err(), "approve is not repeatable");
    }

    #[test]
    fn rejected_is_terminal_and_never_indexed() {
        let (_dir, db) = ledger_db();
        insert_proposed(&db, "p2", "Rejected idea");
        let rejected = reject(&db, "p2").unwrap();
        assert_eq!(rejected.status, "rejected");
        assert!(approve(&db, "p2").is_err(), "rejected is terminal");
        assert!(reject(&db, "p2").is_err());
        assert!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.is_empty());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "idea", None)
            .unwrap()
            .is_empty());
        assert!(load(&db, "p2").unwrap().is_some(), "history stays readable");
    }

    #[test]
    fn active_records_cannot_be_approved_or_rejected() {
        let (_dir, db) = ledger_db();
        let saved = save(&db, "Already active", None, None).unwrap();
        assert!(approve(&db, &saved.id).is_err());
        assert!(reject(&db, &saved.id).is_err());
    }

    #[test]
    fn supersede_retires_the_old_row_into_walkable_history() {
        let (_dir, db) = ledger_db();
        let original = save(&db, "Prefers yarn", None, None).unwrap();
        let replacement = supersede(&db, &original.id, "Prefers bun", None).unwrap();
        assert_ne!(replacement.id, original.id, "edit is never an in-place UPDATE");
        assert_eq!(replacement.status, "active");
        assert_eq!(replacement.kind, original.kind, "kind is inherited unless given");
        let (sup, sup_by) = chain_columns(&db, &replacement.id);
        assert_eq!(sup.as_deref(), Some(original.id.as_str()));
        assert_eq!(sup_by, None);
        let (old_sup, old_sup_by) = chain_columns(&db, &original.id);
        assert_eq!(old_sup, None);
        assert_eq!(old_sup_by.as_deref(), Some(replacement.id.as_str()));
        let old = load(&db, &original.id).unwrap().unwrap();
        assert_eq!(old.status, "superseded");
        assert_eq!(old.body, "Prefers yarn", "history keeps the original body");
        let listed = list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, replacement.id);
    }

    #[test]
    fn only_active_records_supersede() {
        let (_dir, db) = ledger_db();
        let saved = save(&db, "Short lived", None, None).unwrap();
        forget(&db, &saved.id).unwrap();
        assert!(supersede(&db, &saved.id, "Replacement", None).is_err());
        insert_proposed(&db, "p3", "Still proposed");
        assert!(supersede(&db, "p3", "Replacement", None).is_err());
    }

    #[test]
    fn every_removal_path_purges_the_search_index() {
        let (_dir, db) = ledger_db();
        let kept = save(&db, "Keep tabs config", None, None).unwrap();
        let dropped = save(&db, "Forget zsh detail", None, None).unwrap();
        supersede(&db, &kept.id, "Keep spaces config", None).unwrap();
        forget(&db, &dropped.id).unwrap();
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "tabs", None).unwrap().is_empty());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "zsh", None).unwrap().is_empty());
        assert_eq!(
            search(&db, ACCOUNT_MEMORY_SCOPE, "spaces", None).unwrap().len(),
            1
        );
    }

    #[test]
    fn fts_operators_cannot_widen_memory_search() {
        let (_dir, db) = ledger_db();
        save(&db, "Account pin about deploys", None, None).unwrap();
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status, valid_from, created_at, updated_at)
             VALUES('w1', 'workspace:other', 'fact', 'Workspace deploys secret detail', 'user_explicit', 'active', 'now', 'now', 'now')",
            [],
        )
        .unwrap();
        let scoped = search(&db, ACCOUNT_MEMORY_SCOPE, "deploys", None).unwrap();
        assert_eq!(scoped.len(), 1, "the other scope's match stays invisible");
        assert_eq!(scoped[0].scope_key, ACCOUNT_MEMORY_SCOPE);
        assert!(
            search(&db, ACCOUNT_MEMORY_SCOPE, "deploys OR secret", None)
                .unwrap()
                .is_empty(),
            "OR is a stripped word, not an operator that widens the query"
        );
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "\"", None).is_err());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "deploys", Some(0)).is_err());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "deploys", Some(51)).is_err());
    }

    #[test]
    fn kind_vocabulary_matches_the_wire_contract() {
        for kind in bridge_protocol::messages::MEMORY_KINDS {
            assert!(parse_kind(Some(kind)).is_ok(), "contract kind {kind} parses");
        }
        assert!(parse_kind(Some("runbook")).is_err());
    }

    fn table_exists(db: &Connection, name: &str) -> bool {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            params![name],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    }

    fn insert_chat(db: &Connection, id: &str) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES(?1,NULL,'codex','Chat','idle','estimated','direct')",
            params![id],
        )
        .unwrap();
    }

    #[test]
    fn account_scope_is_named_never_null() {
        assert_eq!(account_memory_scope(), "account:local");
        assert!(parse_scope_key("").is_err());
        assert!(parse_scope_key("   ").is_err());
        assert!(parse_scope_key("legacy:global").is_err());
        assert!(parse_scope_key("workspace:").is_err());
        assert_eq!(parse_scope_key("account:local").unwrap(), "account:local");
        assert_eq!(
            parse_scope_key(" workspace:other ").unwrap(),
            "workspace:other"
        );
    }

    #[test]
    fn save_always_writes_account_local_with_user_explicit_provenance() {
        let (_dir, db) = ledger_db();
        insert_chat(&db, "s");
        let record = save(
            &db,
            " I prefer Conventional Commits ",
            Some("preference"),
            Some("s"),
        )
        .unwrap();
        assert_eq!(record.scope_key, "account:local");
        assert_eq!(record.provenance, "user_explicit");
        assert_eq!(record.status, "active");
        assert_eq!(record.kind, "preference");
        assert_eq!(record.body, "I prefer Conventional Commits");
        assert_eq!(record.source_session_id.as_deref(), Some("s"));
    }

    #[test]
    fn empty_body_and_secrets_are_rejected() {
        let (_dir, db) = ledger_db();
        assert!(save(&db, "  ", None, None).is_err());
        assert!(save(
            &db,
            "token sk-ant-abcdefghijklmnopqrstuvwxyz123456",
            None,
            None
        )
        .is_err());
        assert!(list(&db, "account:local", None).unwrap().records.is_empty());
    }

    #[test]
    fn list_account_local_does_not_return_another_named_scope() {
        let (_dir, db) = ledger_db();
        save(&db, "about me pin", None, None).unwrap();
        db.execute(
            "INSERT INTO memory_records(
                id, scope_key, kind, body, provenance, status, source_session_id,
                valid_from, created_at, updated_at
             ) VALUES('other','workspace:other','fact','other desk','user_explicit','active',NULL,'now','now','now')",
            [],
        )
        .unwrap();
        let account = list(&db, "account:local", None).unwrap();
        assert_eq!(account.records.len(), 1);
        assert_eq!(account.records[0].body, "about me pin");
        let other = list(&db, "workspace:other", None).unwrap();
        assert_eq!(other.records.len(), 1);
        assert_eq!(other.records[0].body, "other desk");
        assert!(list(&db, "", None).is_err());
    }

    #[test]
    fn forget_tombstones_and_drops_from_list() {
        let (_dir, db) = ledger_db();
        let record = save(&db, "forget me", Some("fact"), None).unwrap();
        let forgotten = forget(&db, &record.id).unwrap();
        assert_eq!(forgotten.status, "deleted");
        assert_eq!(forgotten.body, "forget me");
        assert!(list(&db, "account:local", None).unwrap().records.is_empty());
        assert!(forget(&db, &record.id).is_err());
    }

    #[test]
    fn forget_by_unique_prefix() {
        let (_dir, db) = ledger_db();
        let record = save(&db, "prefix pin", None, None).unwrap();
        let forgotten = forget_by_selector(&db, &record.id[..8]).unwrap();
        assert_eq!(forgotten.id, record.id);
        assert_eq!(forgotten.status, "deleted");
    }

    #[test]
    fn unknown_session_is_rejected_not_stored_as_null_scope() {
        let (_dir, db) = ledger_db();
        assert!(save(&db, "hello", None, Some("missing")).is_err());
        assert!(list(&db, "account:local", None).unwrap().records.is_empty());
    }

    #[test]
    fn drops_task_knowledge_without_copying_bodies() {
        let (_dir, db) = ledger_db();
        assert!(table_exists(&db, "memory_records"));
        assert!(!table_exists(&db, "task_knowledge"));
        db.execute_batch(
            "CREATE TABLE task_knowledge (id TEXT PRIMARY KEY, body TEXT NOT NULL);
             INSERT INTO task_knowledge(id, body) VALUES('k', 'stolen from task_knowledge');",
        )
        .unwrap();
        {
            let tx = db.unchecked_transaction().unwrap();
            install_ledger(&tx).unwrap();
            tx.commit().unwrap();
        }
        assert!(!table_exists(&db, "task_knowledge"));
        let count: i64 = db
            .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn the_backfill_opens_every_interval_at_creation_and_leaves_active_records_open() {
        let (_dir, db) = ledger_db();
        db.execute_batch(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status, created_at, updated_at)
             VALUES('legacy-active','account:local','fact','Still true','user_explicit','active','2026-01-01T00:00:00+00:00','2026-01-01T00:00:00+00:00'),
                    ('legacy-old','account:local','fact','Was true','user_explicit','superseded','2026-01-01T00:00:00+00:00','2026-02-01T00:00:00+00:00'),
                    ('legacy-gone','account:local','fact','Forgotten','user_explicit','deleted','2026-01-01T00:00:00+00:00','2026-03-01T00:00:00+00:00'),
                    ('legacy-proposed','account:local','fact','Never approved','model_proposal','proposed','2026-01-01T00:00:00+00:00','2026-01-01T00:00:00+00:00'),
                    ('legacy-rejected','account:local','fact','Turned down','model_proposal','rejected','2026-01-01T00:00:00+00:00','2026-04-01T00:00:00+00:00'),
                    ('legacy-backwards','account:local','fact','Odd clock','user_explicit','superseded','2026-05-01T00:00:00+00:00','2026-01-01T00:00:00+00:00');
             UPDATE memory_records SET valid_from=NULL, valid_to=NULL;",
        )
        .unwrap();
        {
            let transaction = db.unchecked_transaction().unwrap();
            install_validity_intervals(&transaction).unwrap();
            transaction.commit().unwrap();
        }
        let interval = |id: &str| -> (String, Option<String>) {
            db.query_row(
                "SELECT valid_from, valid_to FROM memory_records WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        let born = "2026-01-01T00:00:00+00:00".to_string();
        assert_eq!(interval("legacy-active"), (born.clone(), None), "active stays open");
        assert_eq!(
            interval("legacy-old"),
            (born.clone(), Some("2026-02-01T00:00:00+00:00".into()))
        );
        assert_eq!(
            interval("legacy-gone"),
            (born.clone(), Some("2026-03-01T00:00:00+00:00".into()))
        );
        for never_held in ["legacy-proposed", "legacy-rejected"] {
            assert_eq!(
                interval(never_held),
                (born.clone(), Some(born.clone())),
                "{never_held}: a claim nobody approved carries an empty interval"
            );
        }
        assert_eq!(
            interval("legacy-backwards"),
            ("2026-05-01T00:00:00+00:00".into(), Some("2026-05-01T00:00:00+00:00".into())),
            "no backfilled interval ends before it begins"
        );

        let listed = list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "legacy-active");
        let as_of = list_as_of(&db, ACCOUNT_MEMORY_SCOPE, Utc::now()).unwrap().records;
        assert_eq!(
            as_of.iter().map(|record| record.id.clone()).collect::<Vec<_>>(),
            vec!["legacy-active".to_string()],
            "reading now returns exactly what the ledger already returned"
        );

        {
            let transaction = db.unchecked_transaction().unwrap();
            install_validity_intervals(&transaction).unwrap();
            transaction.commit().unwrap();
        }
        assert_eq!(
            interval("legacy-active"),
            (born.clone(), None),
            "the backfill is idempotent and never reopens a closed interval"
        );
        assert_eq!(interval("legacy-old"), (born, Some("2026-02-01T00:00:00+00:00".into())));
    }

    #[test]
    fn save_does_not_go_through_the_router_or_extract() {
        let (_dir, db) = ledger_db();
        let record = save(&db, "typed by the user", None, None).unwrap();
        assert_eq!(record.provenance, "user_explicit");
        assert_eq!(record.scope_key, account_memory_scope());
    }

    #[test]
    fn account_pins_are_not_session_recall() {
        let (_dir, db) = ledger_db();
        insert_chat(&db, "s");
        crate::store::append_session_entry(
            &db,
            "s",
            None,
            "user.message",
            &serde_json::json!({"text": "we talked about the forest"}),
            None,
            "eligible",
            None,
        )
        .unwrap();
        save(&db, "unique-account-pin-token-xyz", None, Some("s")).unwrap();
        let hits =
            crate::session_recall::search(&db, "s", "unique-account-pin-token-xyz", None).unwrap();
        assert!(hits.hits.is_empty());
    }
}
