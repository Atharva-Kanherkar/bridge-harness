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
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

const KIND_PREFERENCE: &str = "preference";
const KIND_FACT: &str = "fact";
const KIND_DECISION: &str = "decision";
const KIND_CONSTRAINT: &str = "constraint";
const PROVENANCE_USER_EXPLICIT: &str = "user_explicit";
const STATUS_ACTIVE: &str = "active";
const STATUS_DELETED: &str = "deleted";

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

fn parse_kind(raw: Option<&str>) -> Result<&'static str, BridgeError> {
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

fn require_body(body: &str) -> Result<String, BridgeError> {
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
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

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
    let now = Utc::now().to_rfc3339();
    let record = MemoryRecord {
        id: Uuid::new_v4().to_string(),
        scope_key,
        kind: kind.to_string(),
        body,
        provenance: PROVENANCE_USER_EXPLICIT.to_string(),
        status: STATUS_ACTIVE.to_string(),
        source_session_id,
        created_at: now.clone(),
        updated_at: now,
    };
    db.execute(
        "INSERT INTO memory_records(
            id, scope_key, kind, body, provenance, status, source_session_id, created_at, updated_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            record.id,
            record.scope_key,
            record.kind,
            record.body,
            record.provenance,
            record.status,
            record.source_session_id,
            record.created_at,
            record.updated_at,
        ],
    )?;
    Ok(record)
}

pub fn list(db: &Connection, scope_key: &str) -> Result<ListMemoryRecordsResult, BridgeError> {
    let scope_key = parse_scope_key(scope_key)?;
    let mut statement = db.prepare(
        "SELECT id, scope_key, kind, body, provenance, status, source_session_id, created_at, updated_at
         FROM memory_records
         WHERE scope_key=?1 AND status=?2
         ORDER BY updated_at DESC, id DESC
         LIMIT ?3",
    )?;
    let records = statement
        .query_map(
            params![scope_key, STATUS_ACTIVE, MAX_MEMORY_LIST_LIMIT as i64],
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
    let changed = db.execute(
        "UPDATE memory_records SET status=?1, updated_at=?2 WHERE id=?3 AND status=?4",
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
        "SELECT id, scope_key, kind, body, provenance, status, source_session_id, created_at, updated_at
         FROM memory_records WHERE id=?1",
        params![record_id],
        map_row,
    )
    .optional()
    .map_err(BridgeError::from)
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
        assert!(list(&db, "account:local").unwrap().records.is_empty());
    }

    #[test]
    fn list_account_local_does_not_return_another_named_scope() {
        let (_dir, db) = ledger_db();
        save(&db, "about me pin", None, None).unwrap();
        db.execute(
            "INSERT INTO memory_records(
                id, scope_key, kind, body, provenance, status, source_session_id, created_at, updated_at
             ) VALUES('other','workspace:other','fact','other desk','user_explicit','active',NULL,'now','now')",
            [],
        )
        .unwrap();
        let account = list(&db, "account:local").unwrap();
        assert_eq!(account.records.len(), 1);
        assert_eq!(account.records[0].body, "about me pin");
        let other = list(&db, "workspace:other").unwrap();
        assert_eq!(other.records.len(), 1);
        assert_eq!(other.records[0].body, "other desk");
        assert!(list(&db, "").is_err());
    }

    #[test]
    fn forget_tombstones_and_drops_from_list() {
        let (_dir, db) = ledger_db();
        let record = save(&db, "forget me", Some("fact"), None).unwrap();
        let forgotten = forget(&db, &record.id).unwrap();
        assert_eq!(forgotten.status, "deleted");
        assert_eq!(forgotten.body, "forget me");
        assert!(list(&db, "account:local").unwrap().records.is_empty());
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
        assert!(list(&db, "account:local").unwrap().records.is_empty());
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
