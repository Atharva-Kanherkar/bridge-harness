//! Explicit named-scope pins. Not session recall, not the learning router.
//!
//! The only writable scope in this release is `account:local`, set on save
//! (never taken from the client, never SQL NULL). Listing always filters
//! `scope_key = ?`. `task_knowledge` is dropped without copying rows.

use crate::secret_interception;
use crate::BridgeError;
use bridge_protocol::messages::{
    ListMemoryRecordsResult, MemoryRecord, MemoryReviewRefusal, ReviewMemoryRecordsResult,
    ACCOUNT_MEMORY_SCOPE, MAX_MEMORY_BODY_CHARS, MAX_MEMORY_LIST_LIMIT, MAX_MEMORY_REVIEW_BATCH,
    MEMORY_LAYER, TASTE_LAYER,
};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

const KIND_PREFERENCE: &str = "preference";
const KIND_FACT: &str = "fact";
const KIND_DECISION: &str = "decision";
const KIND_CONSTRAINT: &str = "constraint";
const PROVENANCE_USER_EXPLICIT: &str = "user_explicit";
pub const PROVENANCE_MODEL_PROPOSAL: &str = "model_proposal";
/// A proposal an agent made with `memory_propose`. Never activated without a
/// person's approval.
pub const PROVENANCE_AGENT_PROPOSAL: &str = "agent_proposal";
/// How long a proposal waits for review before it is swept to `expired`.
/// Long enough to survive a holiday, short enough that a review queue nobody
/// opens drains instead of holding the scope at its budget forever.
pub const PROPOSAL_TTL_DAYS: i64 = crate::memory_retention::DEFAULT_PROPOSAL_TTL_DAYS;
const STATUS_ACTIVE: &str = "active";
const STATUS_PROPOSED: &str = "proposed";
pub(crate) const STATUS_REJECTED: &str = "rejected";
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
/// The memory index with Porter stemming, so a search for "restaurant"
/// finds a memory about "restaurants" and "moved" finds "moving". Rebuilt from
/// the active records; the triggers name the table, so they carry over.
pub(crate) fn install_stemmed_search(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "DROP TABLE IF EXISTS memory_record_fts;
        CREATE VIRTUAL TABLE memory_record_fts USING fts5(
            record_id UNINDEXED,
            scope_key UNINDEXED,
            body,
            tokenize = 'porter unicode61 remove_diacritics 2'
        );
        INSERT INTO memory_record_fts(record_id, scope_key, body)
        SELECT id, scope_key, body FROM memory_records WHERE status = 'active';",
    )?;
    Ok(())
}

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

/// Schema 105: the taste layer's fields, on the same ledger as memory.
///
/// `layer` says what a record is: a `memory` (what a person or agent said) or
/// a `taste` rule (learned from what a person kept, edited and dropped). Every
/// existing row is a memory, and every memory surface reads only that layer,
/// so nothing a person sees today changes. `path_globs`, `outcome_score` and
/// `last_confirmed_at` are for taste rules; `visibility` has one value until
/// records can be shared. `memory_evidence` links a record to the signals
/// that justify it, and a successor inherits its sources' links.
pub(crate) fn install_taste_fields(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    use crate::store::add_column_if_missing;
    add_column_if_missing(
        transaction,
        "memory_records",
        "layer",
        "TEXT NOT NULL DEFAULT 'memory' CHECK(layer IN ('memory','taste'))",
    )?;
    add_column_if_missing(transaction, "memory_records", "path_globs", "TEXT")?;
    add_column_if_missing(transaction, "memory_records", "outcome_score", "REAL NOT NULL DEFAULT 0")?;
    add_column_if_missing(transaction, "memory_records", "last_confirmed_at", "TEXT")?;
    add_column_if_missing(
        transaction,
        "memory_records",
        "visibility",
        "TEXT NOT NULL DEFAULT 'private' CHECK(visibility IN ('private'))",
    )?;
    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memory_records_scope_layer_status
             ON memory_records(scope_key, layer, status, updated_at);
         CREATE TABLE IF NOT EXISTS memory_evidence(
             record_id TEXT NOT NULL,
             signal_id TEXT NOT NULL,
             weight REAL NOT NULL DEFAULT 1,
             added_at TEXT NOT NULL,
             PRIMARY KEY(record_id, signal_id)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_memory_evidence_signal ON memory_evidence(signal_id);",
    )?;
    Ok(())
}

/// `memory` or `taste`; anything else is refused.
pub fn parse_layer(raw: Option<&str>) -> Result<&'static str, BridgeError> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some(MEMORY_LAYER) => Ok(MEMORY_LAYER),
        Some(TASTE_LAYER) => Ok(TASTE_LAYER),
        Some(other) => Err(BridgeError::Invalid(format!(
            "Unknown memory layer '{other}'. Use {MEMORY_LAYER} or {TASTE_LAYER}."
        ))),
    }
}

/// The scope of one repository, `project:<owner>/<repo>`, from a GitHub slug.
/// Lowercased, since GitHub treats names case-insensitively.
pub fn project_scope(owner: &str, repo: &str) -> Result<String, BridgeError> {
    parse_scope_key(&format!("project:{owner}/{repo}"))
}

/// A GitHub owner or repository name: letters, digits, `-`, `_` and `.`, and
/// never `.` or `..` on its own.
fn valid_slug_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && part != "."
        && part != ".."
        && part.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
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
    if let Some(slug) = trimmed.strip_prefix("project:") {
        let slug = slug.to_ascii_lowercase();
        return match slug.split_once('/') {
            Some((owner, repo)) if valid_slug_part(owner) && valid_slug_part(repo) => {
                Ok(format!("project:{owner}/{repo}"))
            }
            _ => Err(BridgeError::Invalid(format!(
                "A project memory scope is project:<owner>/<repo>, not '{trimmed}'."
            ))),
        };
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
        layer: row.get(16)?,
        path_globs: row
            .get::<_, Option<String>>(17)?
            .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok()),
        outcome_score: row.get(18)?,
        evidence_count: row.get::<_, i64>(19)?.clamp(0, i64::from(u32::MAX)) as u32,
    })
}

const RECORD_COLUMNS: &str = "id, scope_key, kind, body, provenance, status, source_session_id, \
     confidence_bps, rationale, supersedes, valid_from, valid_to, expires_at, conflict_group, \
     created_at, updated_at, layer, path_globs, outcome_score, \
     (SELECT COUNT(*) FROM memory_evidence WHERE memory_evidence.record_id = memory_records.id)";

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
    // Saving what is already pinned is not growth. A double click, or the same
    // pin typed again in different case, answers with the record that exists,
    // and a full scope does not refuse a pin it already holds.
    if let Some(existing) = active_with_same_text(db, &scope_key, &body)? {
        return Ok(existing);
    }
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
        layer: MEMORY_LAYER.to_string(),
        path_globs: None,
        outcome_score: 0.0,
        evidence_count: 0,
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
    list_page(db, scope_key, status, None, None)
}

/// One page of a scope's memories. [`list_layer_page`] for another layer.
pub fn list_page(
    db: &Connection,
    scope_key: &str,
    status: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<ListMemoryRecordsResult, BridgeError> {
    list_layer_page(db, scope_key, None, status, cursor, limit)
}

/// Where a page ended, so the next page starts strictly after it. Keyset
/// rather than offset: approving, rejecting or editing a record between two
/// reads cannot shift a later page and skip or repeat a record.
#[derive(serde::Serialize, serde::Deserialize)]
struct PageCursor {
    status: String,
    /// Absent in cursors issued before layers existed, which listed memories.
    #[serde(default = "memory_layer_name")]
    layer: String,
    updated_at: String,
    id: String,
}

fn memory_layer_name() -> String {
    MEMORY_LAYER.to_string()
}

fn encode_cursor(status: &str, layer: &str, record: &MemoryRecord) -> Result<String, BridgeError> {
    use base64::Engine;
    let cursor = PageCursor {
        layer: layer.to_string(),
        status: status.to_string(),
        updated_at: record.updated_at.clone(),
        id: record.id.clone(),
    };
    let json = serde_json::to_vec(&cursor)
        .map_err(|error| BridgeError::Invalid(format!("Memory page cursor: {error}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

fn decode_cursor(raw: &str, status: &str, layer: &str) -> Result<PageCursor, BridgeError> {
    use base64::Engine;
    let refused = || {
        BridgeError::Invalid(
            "That memory page cursor is not valid. Reload the list from its first page.".into(),
        )
    };
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw.trim())
        .map_err(|_| refused())?;
    let cursor: PageCursor = serde_json::from_slice(&bytes).map_err(|_| refused())?;
    if cursor.status != status {
        return Err(BridgeError::Invalid(format!(
            "That memory page cursor lists {} records, not {status} ones.",
            cursor.status
        )));
    }
    if cursor.layer != layer {
        return Err(BridgeError::Invalid(format!(
            "That memory page cursor lists the {} layer, not {layer}.",
            cursor.layer
        )));
    }
    Ok(cursor)
}

/// One page of a scope's `active`, `proposed` or `expired` records in one
/// layer (`memory` when absent), newest change first, with the status's total
/// and a cursor for the next page.
pub fn list_layer_page(
    db: &Connection,
    scope_key: &str,
    layer: Option<&str>,
    status: Option<&str>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<ListMemoryRecordsResult, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let layer = parse_layer(layer)?;
    let status = match status.map(str::trim).filter(|value| !value.is_empty()) {
        None => STATUS_ACTIVE,
        Some(STATUS_ACTIVE) => STATUS_ACTIVE,
        Some(STATUS_PROPOSED) => STATUS_PROPOSED,
        Some(STATUS_EXPIRED) => STATUS_EXPIRED,
        Some(other) => {
            return Err(BridgeError::Invalid(format!(
                "Memory list can show active, proposed or expired records, not '{other}'."
            )))
        }
    };
    let limit = match limit {
        None => MAX_MEMORY_LIST_LIMIT,
        Some(value) if (1..=MAX_MEMORY_LIST_LIMIT).contains(&value) => value,
        Some(_) => {
            return Err(BridgeError::Invalid(format!(
                "A memory page holds 1 to {MAX_MEMORY_LIST_LIMIT} records."
            )))
        }
    };
    let after = cursor
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|raw| decode_cursor(raw, status, layer))
        .transpose()?;
    // One row past the page tells whether another page exists without a
    // second count query.
    let fetch = i64::from(limit) + 1;
    let mut records = match &after {
        None => {
            let mut statement = db.prepare(&format!(
                "SELECT {RECORD_COLUMNS}
                 FROM memory_records
                 WHERE scope_key=?1 AND status=?2 AND layer=?4
                 ORDER BY updated_at DESC, id DESC
                 LIMIT ?3",
            ))?;
            let rows = statement
                .query_map(params![scope_key, status, fetch, layer], map_row)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        }
        Some(after) => {
            let mut statement = db.prepare(&format!(
                "SELECT {RECORD_COLUMNS}
                 FROM memory_records
                 WHERE scope_key=?1 AND status=?2 AND layer=?6
                   AND (updated_at < ?3 OR (updated_at = ?3 AND id < ?4))
                 ORDER BY updated_at DESC, id DESC
                 LIMIT ?5",
            ))?;
            let rows = statement
                .query_map(
                    params![scope_key, status, after.updated_at, after.id, fetch, layer],
                    map_row,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        }
    };
    let next_cursor = if records.len() > limit as usize {
        records.truncate(limit as usize);
        records
            .last()
            .map(|record| encode_cursor(status, layer, record))
            .transpose()?
    } else {
        None
    };
    let total: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_records WHERE scope_key=?1 AND status=?2 AND layer=?3",
        params![scope_key, status, layer],
        |row| row.get(0),
    )?;
    Ok(ListMemoryRecordsResult {
        scope_key,
        records,
        total,
        next_cursor,
    })
}

/// Approve and reject many proposals in one transaction. Each id goes through
/// the same transition as its single-record method, so a rule added there
/// (such as a budget check on approval) applies here too. An id that is not a
/// proposal in this scope is reported, not fatal: the rest of the batch
/// applies. Malformed input (too many ids, an id in both lists) changes
/// nothing.
pub fn review_batch(
    db: &Connection,
    scope_key: &str,
    approve_ids: &[String],
    reject_ids: &[String],
) -> Result<ReviewMemoryRecordsResult, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let clean = |ids: &[String]| -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        ids.iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty() && seen.insert(id.clone()))
            .collect()
    };
    let approve_ids = clean(approve_ids);
    let reject_ids = clean(reject_ids);
    if approve_ids.len() + reject_ids.len() > MAX_MEMORY_REVIEW_BATCH {
        return Err(BridgeError::Invalid(format!(
            "One review can decide at most {MAX_MEMORY_REVIEW_BATCH} memory records."
        )));
    }
    if let Some(both) = approve_ids.iter().find(|id| reject_ids.contains(id)) {
        return Err(BridgeError::Invalid(format!(
            "Memory record '{both}' cannot be approved and rejected in one review."
        )));
    }
    let transaction = db.unchecked_transaction()?;
    let mut result = ReviewMemoryRecordsResult {
        scope_key: scope_key.clone(),
        approved: Vec::new(),
        rejected: Vec::new(),
        refused: Vec::new(),
    };
    let decisions = approve_ids
        .iter()
        .map(|id| (id, true))
        .chain(reject_ids.iter().map(|id| (id, false)));
    for (id, approving) in decisions {
        let current: Option<(String, String)> = transaction
            .query_row(
                "SELECT scope_key, status FROM memory_records WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let reason = match current {
            None => Some("This memory record does not exist.".to_string()),
            Some((record_scope, _)) if record_scope != scope_key => {
                Some("This memory record belongs to another scope.".to_string())
            }
            Some((_, status)) if status != STATUS_PROPOSED => {
                Some(format!("This record is already {status}, not waiting for review."))
            }
            Some(_) => None,
        };
        if let Some(reason) = reason {
            result.refused.push(MemoryReviewRefusal { id: id.clone(), reason });
            continue;
        }
        if approving {
            // A full active budget is this record's refusal, not the batch's:
            // the approvals before it stand and the rest are reported. A
            // proposal that replaces an active memory does not grow the scope.
            if active_target_of(&transaction, id)?.is_none()
                && crate::memory_consolidation::scope_full(&transaction, &scope_key)?
            {
                let held = crate::memory_consolidation::held_records(&transaction, &scope_key)?;
                result.refused.push(MemoryReviewRefusal {
                    id: id.clone(),
                    reason: format!("Memory is full ({held} memories). Forget one or let unused ones retire first."),
                });
                continue;
            }
            approve(&transaction, id)?;
            result.approved.push(id.clone());
        } else {
            reject(&transaction, id)?;
            result.rejected.push(id.clone());
        }
    }
    transaction.commit()?;
    Ok(result)
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
    insert_proposal_as(
        db,
        scope_key,
        body,
        kind,
        PROVENANCE_MODEL_PROPOSAL,
        confidence_bps,
        rationale,
        source_session_id,
    )
}

/// The agent tools' only write path: the same proposal, under the provenance
/// that says an agent asked for it.
pub(crate) fn insert_agent_proposal(
    db: &Connection,
    scope_key: &str,
    body: &str,
    kind: &str,
    rationale: &str,
    source_session_id: &str,
) -> Result<MemoryRecord, BridgeError> {
    insert_proposal_as(
        db,
        scope_key,
        body,
        kind,
        PROVENANCE_AGENT_PROPOSAL,
        None,
        Some(rationale),
        source_session_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_proposal_as(
    db: &Connection,
    scope_key: &str,
    body: &str,
    kind: &str,
    provenance: &str,
    confidence_bps: Option<u32>,
    rationale: Option<&str>,
    source_session_id: &str,
) -> Result<MemoryRecord, BridgeError> {
    debug_assert!(provenance == PROVENANCE_MODEL_PROPOSAL || provenance == PROVENANCE_AGENT_PROPOSAL);
    crate::memory_retention::enforce_proposal_cap(db, scope_key)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    // A proposal has never held, so its interval is empty rather than open:
    // an as-of read of any instant must not return something nobody approved.
    db.execute(
        "INSERT INTO memory_records(
            id, scope_key, kind, body, provenance, status, source_session_id,
            confidence_bps, rationale, valid_from, valid_to, created_at, updated_at
         ) VALUES(?1,?2,?3,?4,?9,'proposed',?5,?6,?7,?8,?8,?8,?8)",
        params![
            id,
            scope_key,
            kind,
            body,
            source_session_id,
            confidence_bps.map(|value| value as i64),
            rationale,
            now,
            provenance,
        ],
    )?;
    load(db, &id)?.ok_or_else(|| BridgeError::Invalid("The proposal was not written.".into()))
}

/// A model proposal that would replace `target` once approved: the reconcile
/// answer "update that one". It waits for review like any proposal, with
/// `supersedes` naming the record it replaces.
///
/// A target that is itself a waiting proposal is replaced at once rather than
/// on approval, since it never held: the queue gets the corrected card instead
/// of a second one, and the card inherits whatever that proposal would have
/// replaced. An active target stays active until a person approves.
pub(crate) fn insert_update_proposal(
    db: &Connection,
    scope_key: &str,
    target: &MemoryRecord,
    body: &str,
    kind: &str,
    confidence_bps: Option<u32>,
    rationale: Option<&str>,
    source_session_id: &str,
) -> Result<MemoryRecord, BridgeError> {
    let replaces_proposal = target.status == STATUS_PROPOSED;
    if !replaces_proposal {
        crate::memory_retention::enforce_proposal_cap(db, scope_key)?;
    }
    // Replacing a waiting proposal takes over what it would have replaced: an
    // update of a pending update of A still supersedes A on approval, so A
    // never stays active beside its successor.
    let supersedes = if replaces_proposal { target.supersedes.clone() } else { Some(target.id.clone()) };
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    with_savepoint(db, || {
        db.execute(
            "INSERT INTO memory_records(
                id, scope_key, kind, body, provenance, status, source_session_id,
                confidence_bps, rationale, valid_from, valid_to, created_at, updated_at, supersedes
             ) VALUES(?1,?2,?3,?4,?5,'proposed',?6,?7,?8,?9,?9,?9,?9,?10)",
            params![
                id,
                scope_key,
                kind,
                body,
                PROVENANCE_MODEL_PROPOSAL,
                source_session_id,
                confidence_bps.map(|value| value as i64),
                rationale,
                now,
                supersedes,
            ],
        )?;
        inherit_from(db, &id, &[target.id.as_str()])?;
        if replaces_proposal {
            db.execute(
                "UPDATE memory_records SET status=?1, superseded_by=?2, updated_at=?3
                 WHERE id=?4 AND status=?5",
                params![STATUS_SUPERSEDED, id, now, target.id, STATUS_PROPOSED],
            )?;
        }
        Ok(())
    })?;
    load(db, &id)?.ok_or_else(|| BridgeError::Invalid("The proposal was not written.".into()))
}

/// Run `work` atomically whether or not the caller already holds a
/// transaction: a savepoint nests inside one and opens one otherwise.
pub(crate) fn with_savepoint(db: &Connection, work: impl FnOnce() -> Result<(), BridgeError>) -> Result<(), BridgeError> {
    db.execute_batch("SAVEPOINT memory_ledger_step")?;
    match work() {
        Ok(()) => {
            db.execute_batch("RELEASE memory_ledger_step")?;
            Ok(())
        }
        Err(error) => {
            db.execute_batch("ROLLBACK TO memory_ledger_step; RELEASE memory_ledger_step")?;
            Err(error)
        }
    }
}

/// What a successor carries over besides its text: its sources' layer, path
/// globs, trust and visibility, and the union of their evidence. An edit or a
/// merge of a taste rule stays a rule with its history; for a memory, which has
/// none of these, nothing changes. The widest paths win (a source with none
/// means every path), and so does the highest trust. Sources from different
/// layers are refused: merging them would turn a memory into a rule.
pub(crate) fn inherit_from(db: &Connection, successor: &str, sources: &[&str]) -> Result<(), BridgeError> {
    type Carried = (String, Option<String>, f64, Option<String>, String);
    let mut carried: Vec<Carried> = Vec::new();
    for source in sources {
        let row = db
            .query_row(
                "SELECT layer, path_globs, outcome_score, last_confirmed_at, visibility
                 FROM memory_records WHERE id=?1",
                params![source],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .optional()?;
        carried.extend(row);
        db.execute(
            "INSERT OR IGNORE INTO memory_evidence(record_id, signal_id, weight, added_at)
             SELECT ?1, signal_id, weight, added_at FROM memory_evidence WHERE record_id=?2",
            params![successor, source],
        )?;
    }
    let Some((layer, _, _, _, visibility)) = carried.first().cloned() else {
        return Ok(());
    };
    if carried.iter().any(|(other, ..)| *other != layer) {
        return Err(BridgeError::Invalid(
            "A memory and a taste rule cannot be merged into one record.".into(),
        ));
    }
    if layer == MEMORY_LAYER {
        return Ok(());
    }
    let mut globs: Option<Vec<String>> = Some(Vec::new());
    for (_, raw, ..) in &carried {
        match raw.as_deref().and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok()) {
            Some(more) => {
                if let Some(globs) = globs.as_mut() {
                    globs.extend(more);
                }
            }
            None => globs = None,
        }
    }
    let globs = globs.map(|mut globs| {
        globs.sort();
        globs.dedup();
        globs
    });
    let score = carried.iter().map(|(_, _, score, ..)| *score).fold(f64::NEG_INFINITY, f64::max);
    let confirmed = carried.iter().filter_map(|(_, _, _, at, _)| at.clone()).max();
    db.execute(
        "UPDATE memory_records
         SET layer=?2, path_globs=?3, outcome_score=?4, last_confirmed_at=?5, visibility=?6
         WHERE id=?1",
        params![
            successor,
            layer,
            globs.map(|globs| serde_json::to_string(&globs).unwrap_or_else(|_| "[]".into())),
            score,
            confirmed,
            visibility,
        ],
    )?;
    Ok(())
}

/// Every taste rule in `status`, in every repository, newest change first.
pub fn list_taste(db: &Connection, status: Option<&str>) -> Result<Vec<MemoryRecord>, BridgeError> {
    let status = match status.map(str::trim).filter(|value| !value.is_empty()) {
        None => STATUS_ACTIVE,
        Some(STATUS_ACTIVE) => STATUS_ACTIVE,
        Some(STATUS_PROPOSED) => STATUS_PROPOSED,
        Some(STATUS_REJECTED) => STATUS_REJECTED,
        Some(STATUS_EXPIRED) => STATUS_EXPIRED,
        Some(other) => {
            return Err(BridgeError::Invalid(format!(
                "Taste rules list as active, proposed, rejected or expired, not '{other}'."
            )))
        }
    };
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM memory_records
         WHERE layer='taste' AND status=?1
         ORDER BY updated_at DESC, id DESC LIMIT 500"
    ))?;
    let rows = statement.query_map(params![status], map_row)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// A rule the person writes: a `taste` record, active at once, theirs.
/// Writing the same rule again in the same scope answers with the one there.
pub fn save_taste_rule(
    db: &Connection,
    scope_key: &str,
    kind: &str,
    body: &str,
    path_globs: Option<&[String]>,
) -> Result<MemoryRecord, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    if !(scope_key == ACCOUNT_MEMORY_SCOPE || scope_key.starts_with("project:")) {
        return Err(BridgeError::Invalid("A taste rule belongs to one repository (project:<owner>/<repo>) or to every one (account:local).".into()));
    }
    if !bridge_protocol::messages::TASTE_RULE_KINDS.contains(&kind) {
        return Err(BridgeError::Invalid(format!(
            "A taste rule is one of {}, not '{kind}'.",
            bridge_protocol::messages::TASTE_RULE_KINDS.join(", ")
        )));
    }
    let body = require_body(body)?;
    let globs: Option<Vec<String>> = path_globs
        .map(|globs| globs.iter().map(|glob| glob.trim().to_string()).filter(|glob| !glob.is_empty()).collect::<Vec<_>>())
        .filter(|globs| !globs.is_empty());
    let same: Option<String> = db
        .query_row(
            "SELECT id FROM memory_records WHERE scope_key=?1 AND layer='taste' AND status='active'
             AND lower(trim(body))=lower(trim(?2))",
            params![scope_key, body],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = same {
        return load(db, &id)?.ok_or_else(|| BridgeError::Invalid("The rule vanished.".into()));
    }
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    db.execute(
        "INSERT INTO memory_records(
            id, scope_key, kind, body, provenance, status, valid_from, created_at, updated_at,
            layer, path_globs, last_confirmed_at
         ) VALUES(?1,?2,?3,?4,?5,'active',?6,?6,?6,'taste',?7,?6)",
        params![
            id,
            scope_key,
            kind,
            body,
            PROVENANCE_USER_EXPLICIT,
            now,
            globs.map(|globs| serde_json::to_string(&globs).unwrap_or_else(|_| "[]".into())),
        ],
    )?;
    load(db, &id)?.ok_or_else(|| BridgeError::Invalid("The rule was not written.".into()))
}

/// The decisions behind a record, oldest first, and how many links point at
/// decisions the log has since swept.
pub fn taste_evidence(db: &Connection, record_id: &str) -> Result<bridge_protocol::messages::TasteEvidence, BridgeError> {
    let mut statement = db.prepare(
        "SELECT s.id, s.at, s.source, s.verdict, s.title, s.path, s.severity, s.pr_number, s.harness, e.weight
         FROM memory_evidence e JOIN taste_signals s ON s.id = e.signal_id
         WHERE e.record_id = ?1
         ORDER BY s.at, s.id",
    )?;
    let signals = statement
        .query_map(params![record_id], |row| {
            Ok(bridge_protocol::messages::TasteSignal {
                id: row.get(0)?,
                at: row.get(1)?,
                source: row.get(2)?,
                verdict: row.get(3)?,
                title: row.get(4)?,
                path: row.get(5)?,
                severity: row.get(6)?,
                pr_number: row.get::<_, Option<i64>>(7)?.map(|number| number.max(0) as u64),
                harness: row.get(8)?,
                weight: row.get(9)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let linked: i64 = db.query_row("SELECT COUNT(*) FROM memory_evidence WHERE record_id=?1", params![record_id], |row| row.get(0))?;
    Ok(bridge_protocol::messages::TasteEvidence {
        swept: (linked.max(0) as usize).saturating_sub(signals.len()) as u32,
        signals,
    })
}

/// A rule the taste learner drafted, waiting for the person: a `taste` record
/// in `scope`, proposed, with an empty validity interval like every proposal,
/// and its evidence linked. Returns its id.
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_taste_proposal(
    db: &Connection,
    scope_key: &str,
    kind: &str,
    body: &str,
    rationale: Option<&str>,
    path_globs: Option<&[String]>,
    evidence: &[(String, f64)],
    at: &str,
) -> Result<String, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let body = require_body(body)?;
    let id = Uuid::new_v4().to_string();
    let globs = path_globs
        .map(|globs| serde_json::to_string(globs).map_err(|error| BridgeError::Invalid(error.to_string())))
        .transpose()?;
    with_savepoint(db, || {
        db.execute(
            "INSERT INTO memory_records(
                id, scope_key, kind, body, provenance, status, rationale,
                valid_from, valid_to, created_at, updated_at, layer, path_globs
             ) VALUES(?1,?2,?3,?4,?5,'proposed',?6,?7,?7,?7,?7,'taste',?8)",
            params![id, scope_key, kind, body, PROVENANCE_MODEL_PROPOSAL, rationale, at, globs],
        )?;
        for (signal, weight) in evidence {
            add_evidence(db, &id, signal, *weight, at)?;
        }
        Ok(())
    })?;
    Ok(id)
}

/// Link a record to a signal that justifies it. Linking the same signal again
/// keeps the first link. Its producers arrive with the signal log (#1198) and
/// the learner (#1201).
pub(crate) fn add_evidence(
    db: &Connection,
    record_id: &str,
    signal_id: &str,
    weight: f64,
    at: &str,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT OR IGNORE INTO memory_evidence(record_id, signal_id, weight, added_at) VALUES(?1,?2,?3,?4)",
        params![record_id, signal_id, weight, at],
    )?;
    Ok(())
}

/// Retire an active memory: it stops holding at `at` and lands in Retired,
/// where it can be restored ([`crate::memory_retention::restore`]). Unlike a
/// tombstone the body stays, because a dream may be wrong and the person
/// must be able to take it back. `false` when the record is no longer active.
pub(crate) fn retire(db: &Connection, record_id: &str, at: &str) -> Result<bool, BridgeError> {
    let changed = db.execute(
        "UPDATE memory_records SET status=?1, valid_to=?2, updated_at=?2 WHERE id=?3 AND status=?4",
        params![STATUS_EXPIRED, at, record_id, STATUS_ACTIVE],
    )?;
    Ok(changed > 0)
}

/// Several waiting proposals that say the same thing become one waiting
/// proposal, so the queue shrinks without anything being approved. The
/// successor is as confident as the least confident source, keeps the first
/// source's rationale and chat, and waits as long as the newest source had
/// left; every source is closed as superseded by it.
pub(crate) fn merge_proposals(
    db: &Connection,
    sources: &[MemoryRecord],
    body: &str,
    kind: &str,
    at: &str,
) -> Result<String, BridgeError> {
    let first = sources
        .first()
        .ok_or_else(|| BridgeError::Invalid("A merge needs at least one source.".into()))?;
    let confidence = sources.iter().filter_map(|source| source.confidence_bps).min();
    let waits_from = sources
        .iter()
        .map(|source| source.created_at.as_str())
        .max()
        .unwrap_or(at)
        .to_string();
    let id = Uuid::new_v4().to_string();
    with_savepoint(db, || {
        db.execute(
            "INSERT INTO memory_records(
                id, scope_key, kind, body, provenance, status, source_session_id,
                confidence_bps, rationale, valid_from, valid_to, created_at, updated_at
             ) VALUES(?1,?2,?3,?4,?5,'proposed',?6,?7,?8,?9,?9,?10,?9)",
            params![
                id,
                first.scope_key,
                kind,
                body,
                PROVENANCE_MODEL_PROPOSAL,
                first.source_session_id,
                confidence.map(i64::from),
                first.rationale,
                at,
                waits_from,
            ],
        )?;
        let source_ids: Vec<&str> = sources.iter().map(|source| source.id.as_str()).collect();
        inherit_from(db, &id, &source_ids)?;
        for source in sources {
            let closed = db.execute(
                "UPDATE memory_records SET status=?1, superseded_by=?2, updated_at=?3
                 WHERE id=?4 AND status=?5",
                params![STATUS_SUPERSEDED, id, at, source.id, STATUS_PROPOSED],
            )?;
            if closed == 0 {
                return Err(BridgeError::Invalid(format!("'{}' is no longer waiting.", source.id)));
            }
        }
        Ok(())
    })?;
    Ok(id)
}

/// The active record a waiting proposal would replace on approval, if any.
/// Such an approval swaps one active memory for another, so it never grows
/// the scope and the budget does not apply to it.
pub(crate) fn active_target_of(db: &Connection, record_id: &str) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT target.id FROM memory_records AS proposal
             JOIN memory_records AS target ON target.id = proposal.supersedes
             WHERE proposal.id=?1 AND proposal.status=?2 AND target.status=?3",
            params![record_id, STATUS_PROPOSED, STATUS_ACTIVE],
            |row| row.get(0),
        )
        .optional()?)
}

/// Case-insensitive body match against every non-deleted record in scope, so
/// the extractor cannot re-propose what already exists in any state but gone.
///
/// A proposal the TTL swept unreviewed is the exception: it is `expired` with
/// an empty interval, so it never held, nobody decided against it, and it can
/// no longer be approved. Vetoing a repeat would leave the claim with no way
/// back into review. A rejection still vetoes, and so does a record that held
/// and then expired.
pub(crate) fn body_already_known(
    db: &Connection,
    scope_key: &str,
    body: &str,
) -> Result<bool, BridgeError> {
    let found: Option<i64> = db
        .query_row(
            "SELECT 1 FROM memory_records
             WHERE scope_key=?1 AND layer='memory' AND status<>'deleted' AND lower(trim(body))=lower(trim(?2))
               AND NOT (status='expired' AND valid_to IS valid_from)
             LIMIT 1",
            params![scope_key, body],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
}

/// The active record in scope whose text is the same as `body` once case,
/// whitespace and edge punctuation are folded. An explicit save's idempotency
/// check: exact text only, because a person typing a similar pin means it.
fn active_with_same_text(
    db: &Connection,
    scope_key: &str,
    body: &str,
) -> Result<Option<MemoryRecord>, BridgeError> {
    let wanted = crate::memory_similarity::normalize(body);
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM memory_records
         WHERE scope_key=?1 AND status=?2 AND layer='memory'
         ORDER BY created_at, id",
    ))?;
    let records = statement.query_map(params![scope_key, STATUS_ACTIVE], map_row)?;
    for record in records {
        let record = record?;
        if crate::memory_similarity::normalize(&record.body) == wanted {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

/// The record a new proposal would repeat: an active, proposed or rejected
/// record in scope that makes the same claim by
/// [`crate::memory_similarity::near_duplicate`]. Rejected counts, so a claim
/// the person turned down does not come back with a word changed.
pub(crate) fn near_duplicate_of(
    db: &Connection,
    scope_key: &str,
    body: &str,
) -> Result<Option<MemoryRecord>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM memory_records
         WHERE scope_key=?1 AND layer='memory' AND status IN ('active','proposed','rejected')
         ORDER BY status='active' DESC, created_at, id",
    ))?;
    let records = statement.query_map(params![scope_key], map_row)?;
    for record in records {
        let record = record?;
        if crate::memory_similarity::near_duplicate(&record.body, body) {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

/// `proposed -> active`. The only path to active a proposal has, and the
/// instant the record's claim starts holding: approval opens the interval that
/// the proposal was written with closed.
///
/// One statement, like every other transition here: a status flip and an
/// interval open that could land separately would leave a crash window in
/// which a record is active with an empty interval — held by the ledger but
/// invisible to every as-of read, forever.
pub fn approve(db: &Connection, record_id: &str) -> Result<MemoryRecord, BridgeError> {
    let record_id = record_id.trim();
    if record_id.is_empty() {
        return Err(BridgeError::Invalid("Memory approve needs a record id.".into()));
    }
    // Proposals live outside the active budget, so approval is what grows it,
    // unless the proposal replaces an active memory: that swaps one for one.
    let replaces = active_target_of(db, record_id)?;
    if replaces.is_none() {
        if let Some(record) = load(db, record_id)? {
            if record.status == STATUS_PROPOSED {
                crate::memory_consolidation::enforce_scope_budget(db, &record.scope_key)?;
            }
        }
    }
    let updated_at = Utc::now().to_rfc3339();
    with_savepoint(db, || {
        let changed = db.execute(
            "UPDATE memory_records SET status=?1, updated_at=?2, valid_from=?2, valid_to=NULL
             WHERE id=?3 AND status=?4",
            params![STATUS_ACTIVE, updated_at, record_id, STATUS_PROPOSED],
        )?;
        if changed == 0 {
            return Err(BridgeError::Invalid(format!(
                "Only a {STATUS_PROPOSED} memory record can be {STATUS_ACTIVE}; '{record_id}' is not one."
            )));
        }
        // The replaced memory stops holding the instant its successor starts.
        if let Some(target) = &replaces {
            db.execute(
                "UPDATE memory_records SET status=?1, superseded_by=?2, valid_to=?3, updated_at=?3
                 WHERE id=?4 AND status=?5",
                params![STATUS_SUPERSEDED, record_id, updated_at, target, STATUS_ACTIVE],
            )?;
        }
        Ok(())
    })?;
    load(db, record_id)?
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
    let predecessor_ids: Vec<&str> = predecessors.iter().map(|record| record.id.as_str()).collect();
    inherit_from(transaction, &replacement_id, &predecessor_ids)?;
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
    // A tombstone is the one closure a user asked for by name. Its interval
    // still closes at the forget instant so nothing overlaps, but the body a
    // user said to forget does not come back through the history read — the
    // FTS purge already removes it from search, and this read honours the
    // same request. Superseded and expired records stay queryable: those
    // closures are lifecycle, not a deletion.
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS}
         FROM memory_records
         WHERE scope_key=?1 AND layer='memory' AND status<>?4
           AND valid_from <= ?2 AND (valid_to IS NULL OR valid_to > ?2)
         ORDER BY updated_at DESC, id DESC
         LIMIT ?3",
    ))?;
    let records = statement
        .query_map(
            params![scope_key, at, MAX_MEMORY_LIST_LIMIT as i64, STATUS_DELETED],
            map_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    // History reads stay one page; the total still counts everything that
    // held at the instant, so a surface can say what it is not showing.
    let total: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_records
         WHERE scope_key=?1 AND layer='memory' AND status<>?3
           AND valid_from <= ?2 AND (valid_to IS NULL OR valid_to > ?2)",
        params![scope_key, at, STATUS_DELETED],
        |row| row.get(0),
    )?;
    Ok(ListMemoryRecordsResult {
        scope_key,
        records,
        total,
        next_cursor: None,
    })
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

/// Expire proposals that waited longer than the retention setting's wait
/// (default [`PROPOSAL_TTL_DAYS`]) for review, and log the sweep.
///
/// A proposal never held, so its interval stays empty: `valid_to` is left at
/// `valid_from`, and no as-of read of any instant returns it. Only the status
/// moves, which takes it out of the review queue and out of the budget. Like
/// [`sweep_expired`], the instant is an argument so the boundary is exact: a
/// proposal created exactly `PROPOSAL_TTL_DAYS` before `now` is stale.
pub fn sweep_stale_proposals(
    db: &Connection,
    now: DateTime<Utc>,
) -> Result<Vec<String>, BridgeError> {
    crate::memory_retention::sweep_stale_proposals(db, now)
}

/// Expire proposals created at least `wait` before `now`. The retention
/// setting decides `wait`; this is only the transition.
pub(crate) fn sweep_proposals_older_than(
    db: &Connection,
    now: DateTime<Utc>,
    wait: chrono::Duration,
) -> Result<Vec<String>, BridgeError> {
    let cutoff = (now - wait).to_rfc3339();
    let mut statement = db.prepare(
        "SELECT id FROM memory_records
         WHERE status=?1 AND created_at <= ?2
         ORDER BY created_at, id",
    )?;
    let stale: Vec<String> = statement
        .query_map(params![STATUS_PROPOSED, cutoff], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if stale.is_empty() {
        return Ok(stale);
    }
    db.execute(
        "UPDATE memory_records SET status=?1, updated_at=?2
         WHERE status=?3 AND created_at <= ?4",
        params![STATUS_EXPIRED, now.to_rfc3339(), STATUS_PROPOSED, cutoff],
    )?;
    Ok(stale)
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

/// Everything extraction may reconcile a candidate against: the scope's active
/// memories and the proposals waiting for review, newest first.
pub(crate) fn reconcilable_records(
    db: &Connection,
    scope_key: &str,
    limit: i64,
) -> Result<Vec<MemoryRecord>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {RECORD_COLUMNS} FROM memory_records
         WHERE scope_key=?1 AND layer='memory' AND status IN (?2, ?3)
         ORDER BY updated_at DESC, id
         LIMIT ?4",
    ))?;
    let records = statement
        .query_map(params![scope_key, STATUS_ACTIVE, STATUS_PROPOSED, limit], map_row)?
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
/// recall: phrase-quoted tokens, scope bound in SQL, bounded limit. The order
/// is total, so `offset` pages through it with no gap and no repeat.
pub fn search(
    db: &Connection,
    scope_key: &str,
    query: &str,
    limit: Option<u32>,
    offset: u64,
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
    let offset = i64::try_from(offset).unwrap_or(i64::MAX);
    let wanted = offset.saturating_add(limit);
    // Every word first. An agent asks in a sentence ("what car does the
    // person drive"), and a record that holds the answer rarely holds every
    // word of the question, so a short AND result is topped up with records
    // that share any content word, best match first, after the AND hits. The
    // AND hits keep their place, so paging stays stable.
    let mut records = search_matching(db, &scope_key, &match_query, wanted)?;
    if (records.len() as i64) < wanted {
        if let Some(any_word) = any_word_query(query) {
            let seen: std::collections::HashSet<String> = records.iter().map(|record| record.id.clone()).collect();
            let relaxed = search_matching(db, &scope_key, &any_word, wanted.saturating_add(seen.len() as i64))?;
            records.extend(relaxed.into_iter().filter(|record| !seen.contains(&record.id)));
        }
    }
    Ok(records
        .into_iter()
        .skip(usize::try_from(offset).unwrap_or(usize::MAX))
        .take(limit as usize)
        .collect())
}

/// Words a relaxed search leaves out: they match nearly every record, so they
/// would rank noise as highly as the word that names the subject.
const SEARCH_STOP_WORDS: &[&str] = &[
    "a", "about", "am", "an", "and", "any", "are", "as", "at", "be", "been", "but", "by", "can",
    "could", "did", "do", "does", "for", "from", "had", "has", "have", "how", "i", "if", "in",
    "is", "it", "its", "me", "my", "of", "on", "or", "our", "person", "should", "so", "that",
    "the", "their", "them", "they", "this", "to", "user", "was", "we", "were", "what", "when",
    "where", "which", "who", "why", "will", "with", "would", "you", "your",
];

/// The OR of a query's content words, or `None` when fewer than two remain
/// (one word is already what the AND searched).
fn any_word_query(query: &str) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    for word in query.split(|character: char| !character.is_alphanumeric()).filter(|word| !word.is_empty()) {
        let lowered = word.to_lowercase();
        if SEARCH_STOP_WORDS.contains(&lowered.as_str())
            || matches!(lowered.as_str(), "and" | "or" | "not" | "near")
            || words.contains(&lowered)
        {
            continue;
        }
        words.push(lowered);
    }
    (words.len() > 1).then(|| words.iter().map(|word| format!("\"{word}\"")).collect::<Vec<_>>().join(" OR "))
}

fn search_matching(db: &Connection, scope_key: &str, match_query: &str, limit: i64) -> Result<Vec<MemoryRecord>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT m.id, m.scope_key, m.kind, m.body, m.provenance, m.status,
                m.source_session_id, m.confidence_bps, m.rationale,
                m.supersedes, m.valid_from, m.valid_to, m.expires_at, m.conflict_group,
                m.created_at, m.updated_at, m.layer, m.path_globs, m.outcome_score,
                (SELECT COUNT(*) FROM memory_evidence e WHERE e.record_id = m.id)
         FROM memory_record_fts f
         JOIN memory_records m ON m.id = f.record_id
         WHERE f.scope_key = ?1 AND memory_record_fts MATCH ?2 AND m.status = ?3
           AND m.layer = 'memory'
         ORDER BY rank, m.updated_at DESC, m.id
         LIMIT ?4",
    )?;
    let records = statement
        .query_map(params![scope_key, match_query, STATUS_ACTIVE, limit], map_row)?
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
    let shown = result.records.len() as i64;
    let count = if result.total > shown {
        format!("{shown} of {}", result.total)
    } else {
        shown.to_string()
    };
    let mut out = format!("Account memory in `{}` ({count}):\n", result.scope_key);
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
#[path = "memory_taste_fields_tests.rs"]
mod taste_fields_tests;

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

    fn insert_at(db: &Connection, id: &str, status: &str, updated_at: &str) {
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                 valid_from, valid_to, created_at, updated_at)
             VALUES(?1, ?2, 'fact', ?1, 'model_proposal', ?3, ?4, ?4, ?4, ?4)",
            params![id, ACCOUNT_MEMORY_SCOPE, status, updated_at],
        )
        .unwrap();
    }

    fn page_ids(result: &ListMemoryRecordsResult) -> Vec<String> {
        result.records.iter().map(|record| record.id.clone()).collect()
    }

    fn read_all_pages(db: &Connection, status: &str) -> Vec<String> {
        let mut seen = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = list_page(db, ACCOUNT_MEMORY_SCOPE, Some(status), cursor.as_deref(), None).unwrap();
            seen.extend(page_ids(&page));
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return seen,
            }
        }
    }

    #[test]
    fn proposals_page_as_fifty_fifty_thirty_eight_with_a_true_total() {
        let (_dir, db) = ledger_db();
        for index in 0..138 {
            insert_at(&db, &format!("p{index:03}"), "proposed", &format!("2026-11-{:02}T00:00:00+00:00", 1 + index % 20));
        }
        let first = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"), None, None).unwrap();
        assert_eq!((first.records.len(), first.total), (50, 138));
        let second = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"), first.next_cursor.as_deref(), None).unwrap();
        assert_eq!((second.records.len(), second.total), (50, 138));
        let third = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"), second.next_cursor.as_deref(), None).unwrap();
        assert_eq!((third.records.len(), third.total), (38, 138));
        assert!(third.next_cursor.is_none(), "the last page has no cursor");

        let all = read_all_pages(&db, "proposed");
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!((all.len(), unique.len()), (138, 138), "no duplicate and no gap across ties");
        let legacy = list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap();
        assert_eq!(page_ids(&legacy), page_ids(&first), "callers without a cursor get the same first page");
    }

    #[test]
    fn reviewing_a_record_between_pages_neither_skips_nor_repeats() {
        let (_dir, db) = ledger_db();
        for index in 0..60 {
            insert_at(&db, &format!("p{index:02}"), "proposed", "2026-11-20T00:00:00+00:00");
        }
        let first = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"), None, Some(20)).unwrap();
        approve(&db, &first.records[3].id).unwrap();
        reject(&db, &first.records[7].id).unwrap();
        let mut seen = page_ids(&first);
        let mut cursor = first.next_cursor;
        while let Some(next) = cursor {
            let page = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"), Some(&next), Some(20)).unwrap();
            seen.extend(page_ids(&page));
            cursor = page.next_cursor;
        }
        let unique: std::collections::HashSet<_> = seen.iter().collect();
        assert_eq!((seen.len(), unique.len()), (60, 60));
    }

    #[test]
    fn a_bad_limit_or_cursor_is_refused_not_read_as_the_first_page() {
        let (_dir, db) = ledger_db();
        for index in 0..3 {
            insert_at(&db, &format!("a{index}"), "active", "2026-11-20T00:00:00+00:00");
            insert_at(&db, &format!("p{index}"), "proposed", "2026-11-20T00:00:00+00:00");
        }
        for limit in [0, 51] {
            assert!(list_page(&db, ACCOUNT_MEMORY_SCOPE, None, None, Some(limit)).is_err(), "limit {limit}");
        }
        assert!(list_page(&db, ACCOUNT_MEMORY_SCOPE, None, Some("not a cursor"), None).is_err());
        let proposed = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed"), None, Some(1)).unwrap();
        let foreign = proposed.next_cursor.expect("more proposals");
        let error = list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("active"), Some(&foreign), None).unwrap_err();
        assert!(error.to_string().contains("proposed"), "{error}");
    }

    #[test]
    fn a_review_batch_applies_what_it_can_and_reports_the_rest() {
        let (_dir, db) = ledger_db();
        for index in 0..6 {
            insert_proposed(&db, &format!("p{index}"), &format!("proposal {index}"));
        }
        reject(&db, "p5").unwrap();
        let ids = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        let result = review_batch(&db, ACCOUNT_MEMORY_SCOPE, &ids(&["p0", "p1", "p2", "p5"]), &ids(&["p3", "p4", "missing"])).unwrap();
        assert_eq!(result.approved, ids(&["p0", "p1", "p2"]));
        assert_eq!(result.rejected, ids(&["p3", "p4"]));
        let refused: Vec<_> = result.refused.iter().map(|refusal| refusal.id.as_str()).collect();
        assert_eq!(refused, vec!["p5", "missing"]);
        assert!(result.refused[0].reason.contains("rejected"), "{}", result.refused[0].reason);
        assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().total, 3);
        assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap().total, 0);
    }

    #[test]
    fn a_malformed_review_batch_changes_nothing() {
        let (_dir, db) = ledger_db();
        insert_proposed(&db, "p0", "proposal");
        insert_proposed(&db, "p1", "proposal");
        let both = review_batch(&db, ACCOUNT_MEMORY_SCOPE, &["p0".into(), "p1".into()], &["p1".into()]);
        assert!(both.is_err(), "an id in both lists is refused");
        let too_many: Vec<String> = (0..=MAX_MEMORY_REVIEW_BATCH).map(|index| format!("x{index}")).collect();
        assert!(review_batch(&db, ACCOUNT_MEMORY_SCOPE, &too_many, &[]).is_err());
        assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, Some("proposed")).unwrap().total, 2, "nothing moved");
    }

    fn chain_columns(db: &Connection, id: &str) -> (Option<String>, Option<String>) {
        db.query_row(
            "SELECT supersedes, superseded_by FROM memory_records WHERE id=?1",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    }

    fn set_budget(db: &Connection, max_records: i64) {
        crate::memory_consolidation::update_settings(
            db,
            ACCOUNT_MEMORY_SCOPE,
            "off",
            Some(max_records),
            None,
            None,
        )
        .unwrap();
    }

    fn count(db: &Connection, status: &str) -> i64 {
        db.query_row(
            "SELECT COUNT(*) FROM memory_records WHERE status=?1",
            params![status],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn saving_the_same_pin_twice_is_one_record() {
        let (_dir, db) = ledger_db();
        let first = save(&db, "Prefer vitest -t filters while triaging.", None, None).unwrap();
        let again = save(&db, "  prefer VITEST -t filters while triaging ", Some("constraint"), None).unwrap();
        assert_eq!(again.id, first.id, "the existing record answers");
        assert_eq!(again.kind, "preference", "a repeat changes nothing");
        assert_eq!(count(&db, "active"), 1);
        let different = save(&db, "Prefer vitest -t filters while debugging.", None, None).unwrap();
        assert_ne!(different.id, first.id, "a different pin is a new record");
    }

    #[test]
    fn pins_that_differ_only_in_identifier_punctuation_are_two_records() {
        let (_dir, db) = ledger_db();
        let cpp = save(&db, "Use C++", None, None).unwrap();
        let csharp = save(&db, "Use C#", None, None).unwrap();
        assert_ne!(csharp.id, cpp.id);
        assert_eq!(csharp.body, "Use C#", "the second pin is kept as typed");
        assert_eq!(count(&db, "active"), 2);
    }

    #[test]
    fn only_an_expired_record_that_never_held_drops_out_of_the_known_bodies() {
        let (_dir, db) = ledger_db();
        let insert = |id: &str, valid_to: &str| {
            db.execute(
                "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                     valid_from, valid_to, created_at, updated_at)
                 VALUES(?1, ?2, 'fact', ?1, 'model_proposal', 'expired', '2026-09-01T00:00:00+00:00', ?3, 'now', 'now')",
                params![id, ACCOUNT_MEMORY_SCOPE, valid_to],
            )
            .unwrap();
        };
        insert("swept proposal", "2026-09-01T00:00:00+00:00");
        insert("held then expired", "2026-10-01T00:00:00+00:00");
        assert!(!body_already_known(&db, ACCOUNT_MEMORY_SCOPE, "swept proposal").unwrap());
        assert!(body_already_known(&db, ACCOUNT_MEMORY_SCOPE, "held then expired").unwrap());
    }

    #[test]
    fn a_full_scope_still_answers_a_pin_it_already_holds() {
        let (_dir, db) = ledger_db();
        set_budget(&db, 1);
        let first = save(&db, "Keep answers short", None, None).unwrap();
        assert_eq!(save(&db, "keep answers short.", None, None).unwrap().id, first.id);
        assert!(save(&db, "Something new", None, None).is_err(), "growth is still refused");
    }

    #[test]
    fn a_forgotten_pin_saved_again_is_a_new_record() {
        let (_dir, db) = ledger_db();
        let first = save(&db, "Uses zed", None, None).unwrap();
        forget(&db, &first.id).unwrap();
        let again = save(&db, "Uses zed", None, None).unwrap();
        assert_ne!(again.id, first.id, "only active records answer for a repeat");
        assert_eq!(again.status, "active");
    }

    #[test]
    fn near_duplicates_are_found_among_active_proposed_and_rejected() {
        let (_dir, db) = ledger_db();
        insert_proposed(&db, "p1", "CI is rerunning on 6a77f0c1 after the daemon test fix landed");
        assert_eq!(
            near_duplicate_of(&db, ACCOUNT_MEMORY_SCOPE, "CI is rerunning on 6a77f0c1 after the daemon test fix has landed")
                .unwrap()
                .map(|record| record.id),
            Some("p1".to_string())
        );
        reject(&db, "p1").unwrap();
        assert!(
            near_duplicate_of(&db, ACCOUNT_MEMORY_SCOPE, "ci is rerunning on 6a77f0c1 after the daemon test fix landed").unwrap().is_some(),
            "a rejected claim does not come back"
        );
        db.execute("UPDATE memory_records SET status='deleted' WHERE id='p1'", []).unwrap();
        assert!(
            near_duplicate_of(&db, ACCOUNT_MEMORY_SCOPE, "CI is rerunning on 6a77f0c1 after the daemon test fix landed").unwrap().is_none(),
            "a tombstone is gone"
        );
    }

    #[test]
    fn agent_proposals_carry_their_own_provenance_and_stay_proposed() {
        let (_dir, db) = ledger_db();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES('chat',NULL,'opencode','chat','idle','reported','direct')",
            [],
        )
        .unwrap();
        let record = insert_agent_proposal(&db, ACCOUNT_MEMORY_SCOPE, "Check upstream trackers before hunting", "constraint", "the person said so", "chat").unwrap();
        assert_eq!(record.provenance, PROVENANCE_AGENT_PROPOSAL);
        assert_eq!(record.status, "proposed");
        assert_eq!(record.valid_to.as_deref(), Some(record.valid_from.as_str()), "an empty interval");
        assert_eq!(record.source_session_id.as_deref(), Some("chat"));
        assert_eq!(record.confidence_bps, None, "an agent claims no confidence");
    }

    #[test]
    fn stale_proposals_expire_at_the_boundary_and_keep_an_empty_interval() {
        let (_dir, db) = ledger_db();
        let now = Utc::now();
        let created = |db: &Connection, id: &str, at: DateTime<Utc>| {
            db.execute(
                "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                     valid_from, valid_to, created_at, updated_at)
                 VALUES(?1, ?2, 'fact', ?1, 'model_proposal', 'proposed', ?3, ?3, ?3, ?3)",
                params![id, ACCOUNT_MEMORY_SCOPE, at.to_rfc3339()],
            )
            .unwrap();
        };
        let ttl = chrono::Duration::days(PROPOSAL_TTL_DAYS);
        created(&db, "exactly-stale", now - ttl);
        created(&db, "older", now - ttl - chrono::Duration::seconds(1));
        created(&db, "fresh", now - ttl + chrono::Duration::seconds(1));
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                 valid_from, created_at, updated_at)
             VALUES('old-active', ?1, 'fact', 'old active', 'user_explicit', 'active', ?2, ?2, ?2)",
            params![ACCOUNT_MEMORY_SCOPE, (now - ttl * 3).to_rfc3339()],
        )
        .unwrap();
        let held_before = crate::memory_retention::held_proposals(&db, ACCOUNT_MEMORY_SCOPE).unwrap();

        let swept = sweep_stale_proposals(&db, now).unwrap();
        assert_eq!(swept, vec!["older".to_string(), "exactly-stale".to_string()]);
        for id in ["older", "exactly-stale"] {
            let record = load(&db, id).unwrap().unwrap();
            assert_eq!(record.status, STATUS_EXPIRED);
            assert_eq!(record.valid_to.as_deref(), Some(record.valid_from.as_str()), "never held, so never true");
        }
        assert_eq!(load(&db, "fresh").unwrap().unwrap().status, "proposed");
        assert_eq!(load(&db, "old-active").unwrap().unwrap().status, "active", "the TTL is for proposals only");
        assert_eq!(
            crate::memory_retention::held_proposals(&db, ACCOUNT_MEMORY_SCOPE).unwrap(),
            held_before - 2,
            "expired proposals leave the review queue's allowance"
        );
        assert!(list_as_of(&db, ACCOUNT_MEMORY_SCOPE, now - ttl).unwrap().records.iter().all(|record| record.id == "old-active"));
        assert!(sweep_stale_proposals(&db, now).unwrap().is_empty(), "a second sweep finds nothing");
    }

    #[test]
    fn proposed_reaches_active_only_through_approve() {
        let (_dir, db) = ledger_db();
        insert_proposed(&db, "p1", "Proposed convention");
        assert!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.is_empty());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "convention", None, 0)
            .unwrap()
            .is_empty());
        let approved = approve(&db, "p1").unwrap();
        assert_eq!(approved.status, "active");
        assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.len(), 1);
        assert_eq!(
            search(&db, ACCOUNT_MEMORY_SCOPE, "convention", None, 0)
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
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "idea", None, 0)
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
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "tabs", None, 0).unwrap().is_empty());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "zsh", None, 0).unwrap().is_empty());
        assert_eq!(
            search(&db, ACCOUNT_MEMORY_SCOPE, "spaces", None, 0).unwrap().len(),
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
        let scoped = search(&db, ACCOUNT_MEMORY_SCOPE, "deploys", None, 0).unwrap();
        assert_eq!(scoped.len(), 1, "the other scope's match stays invisible");
        assert_eq!(scoped[0].scope_key, ACCOUNT_MEMORY_SCOPE);
        let widened = search(&db, ACCOUNT_MEMORY_SCOPE, "deploys OR secret", None, 0).unwrap();
        assert_eq!(
            widened.iter().map(|record| record.scope_key.as_str()).collect::<Vec<_>>(),
            vec![ACCOUNT_MEMORY_SCOPE],
            "relaxing the words never relaxes the scope"
        );
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "\"", None, 0).is_err());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "deploys", Some(0), 0).is_err());
        assert!(search(&db, ACCOUNT_MEMORY_SCOPE, "deploys", Some(51), 0).is_err());
    }

    #[test]
    fn a_question_finds_the_record_that_answers_it() {
        let (_dir, db) = ledger_db();
        save(&db, "Drives a 2019 Subaru Outback", Some("fact"), None).unwrap();
        save(&db, "Prefers tabs over spaces", Some("preference"), None).unwrap();
        save(&db, "Has tried three Korean restaurants in town", Some("fact"), None).unwrap();
        let bodies = |query: &str| -> Vec<String> {
            search(&db, ACCOUNT_MEMORY_SCOPE, query, None, 0).unwrap().into_iter().map(|record| record.body).collect()
        };
        assert_eq!(bodies("what car does the person drive")[0], "Drives a 2019 Subaru Outback", "a sentence finds its subject");
        assert_eq!(bodies("Subaru Outback"), vec!["Drives a 2019 Subaru Outback"], "every word still wins outright");
        assert_eq!(bodies("restaurant")[0], "Has tried three Korean restaurants in town", "a singular finds a plural");
        assert!(bodies("what is the").is_empty(), "stop words alone match nothing");
        let first = search(&db, ACCOUNT_MEMORY_SCOPE, "car drive tabs korean", Some(1), 0).unwrap();
        let rest = search(&db, ACCOUNT_MEMORY_SCOPE, "car drive tabs korean", Some(5), 1).unwrap();
        assert_eq!(first.len() + rest.len(), 3, "relaxed pages neither skip nor repeat");
        assert!(!rest.iter().any(|record| record.id == first[0].id));
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

    fn update_of(db: &Connection, target: &MemoryRecord, body: &str) -> MemoryRecord {
        db.execute(
            "INSERT OR IGNORE INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES('s1',NULL,'claude','s1','idle','reported','direct')",
            [],
        )
        .unwrap();
        insert_update_proposal(db, ACCOUNT_MEMORY_SCOPE, target, body, &target.kind, Some(8_000), None, "s1").unwrap()
    }

    #[test]
    fn approving_an_update_supersedes_its_active_target_in_one_step() {
        let (_dir, db) = ledger_db();
        let target = save(&db, "Prefers tabs", Some("preference"), None).unwrap();
        let update = update_of(&db, &target, "Prefers spaces");
        let approved = approve(&db, &update.id).unwrap();
        assert_eq!(approved.status, STATUS_ACTIVE);
        let old = load_record(&db, &target.id).unwrap().unwrap();
        assert_eq!(old.status, STATUS_SUPERSEDED);
        assert_eq!(old.valid_to.as_deref(), Some(approved.valid_from.as_str()), "the old claim ends as the new one starts");
        let active: Vec<String> = list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.into_iter().map(|record| record.body).collect();
        assert_eq!(active, vec!["Prefers spaces".to_string()]);
    }

    #[test]
    fn approving_an_update_at_full_budget_is_allowed() {
        let (_dir, db) = ledger_db();
        crate::memory_consolidation::update_settings(&db, ACCOUNT_MEMORY_SCOPE, "off", Some(2), None, None).unwrap();
        let first = save(&db, "Prefers tabs", Some("preference"), None).unwrap();
        let second = save(&db, "Uses Postgres", Some("fact"), None).unwrap();
        let single = update_of(&db, &first, "Prefers spaces");
        approve(&db, &single.id).unwrap();
        let bulk = update_of(&db, &second, "Uses SQLite");
        let result = review_batch(&db, ACCOUNT_MEMORY_SCOPE, &[bulk.id.clone()], &[]).unwrap();
        assert_eq!(result.approved, vec![bulk.id], "{:?}", result.refused);
        assert_eq!(list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().total, 2, "a swap never grows the scope");
    }

    #[test]
    fn rejecting_an_update_leaves_the_target_active() {
        let (_dir, db) = ledger_db();
        let target = save(&db, "Prefers tabs", Some("preference"), None).unwrap();
        let update = update_of(&db, &target, "Prefers spaces");
        reject(&db, &update.id).unwrap();
        assert_eq!(load_record(&db, &target.id).unwrap().unwrap().status, STATUS_ACTIVE);
    }

    #[test]
    fn an_update_of_a_pending_update_keeps_the_active_predecessor() {
        for full in [false, true] {
            let (_dir, db) = ledger_db();
            let a = save(&db, "Prefers tabs", Some("preference"), None).unwrap();
            if full {
                crate::memory_consolidation::update_settings(&db, ACCOUNT_MEMORY_SCOPE, "off", Some(1), None, None).unwrap();
            }
            let b = update_of(&db, &a, "Prefers spaces");
            let c = update_of(&db, &b, "Prefers two-space indents");
            assert_eq!(c.supersedes.as_deref(), Some(a.id.as_str()), "C inherits B's target");
            assert_eq!(load_record(&db, &b.id).unwrap().unwrap().status, STATUS_SUPERSEDED, "B leaves the queue");
            approve(&db, &c.id).unwrap_or_else(|error| panic!("full={full}: {error}"));
            let active: Vec<String> = list(&db, ACCOUNT_MEMORY_SCOPE, None).unwrap().records.into_iter().map(|record| record.body).collect();
            assert_eq!(active, vec!["Prefers two-space indents".to_string()], "full={full}");
            assert_eq!(load_record(&db, &a.id).unwrap().unwrap().status, STATUS_SUPERSEDED);
        }
    }

}
