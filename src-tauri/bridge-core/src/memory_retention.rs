//! How the account memory scope cleans itself up, without a model.
//!
//! Proposals have their own allowance apart from the active budget, and expire
//! after a short wait for review. Active memories that nothing has used for a
//! while retire, except pins a person saved on purpose. Both are lifecycle
//! transitions to `expired`, never deletions: every retired memory can be
//! restored, and every sweep that changed something is logged for the Activity
//! view.

use crate::memory_ledger;
use crate::BridgeError;
use bridge_protocol::messages::{MemoryRecord, MemoryRetentionSettings, ACCOUNT_MEMORY_SCOPE};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use uuid::Uuid;

pub const DEFAULT_PROPOSAL_TTL_DAYS: i64 = 7;
pub const PROPOSAL_TTL_CHOICES: [i64; 4] = [3, 7, 14, 30];
pub const DEFAULT_RETIRE_UNUSED_AFTER_DAYS: Option<i64> = Some(90);
pub const RETIRE_UNUSED_CHOICES: [i64; 3] = [30, 90, 180];
pub const DEFAULT_MAX_PROPOSALS: i64 = 100;
const MIN_MAX_PROPOSALS: i64 = 10;
const MAX_MAX_PROPOSALS: i64 = 1000;

pub const SWEEP_PROPOSALS_EXPIRED: &str = "proposals_expired";
pub const SWEEP_UNUSED_RETIRED: &str = "unused_retired";

/// Migration 99: the settings row, the sweep log, and `last_used_at` on every
/// record, backfilled from the packet audits that survive.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_retention_settings (
            scope_key TEXT PRIMARY KEY,
            proposal_ttl_days INTEGER NOT NULL,
            retire_unused_after_days INTEGER,
            max_proposals INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS memory_retention_sweeps (
            id TEXT PRIMARY KEY,
            scope_key TEXT NOT NULL,
            kind TEXT NOT NULL,
            record_count INTEGER NOT NULL,
            record_ids TEXT NOT NULL,
            at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_retention_sweeps_scope_at
            ON memory_retention_sweeps(scope_key, at);",
    )?;
    crate::store::add_column_if_missing(transaction, "memory_records", "last_used_at", "TEXT")?;
    transaction.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memory_records_last_used
             ON memory_records(scope_key, status, last_used_at);
         UPDATE memory_records
         SET last_used_at = (
             SELECT MAX(audit.created_at)
             FROM memory_retrieval_audits AS audit, json_each(audit.selected_ids) AS item
             WHERE CASE WHEN item.type = 'text' THEN item.value
                        ELSE json_extract(item.value, '$.id') END = memory_records.id
         )
         WHERE last_used_at IS NULL;",
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionSettings {
    pub proposal_ttl_days: i64,
    pub retire_unused_after_days: Option<i64>,
    pub max_proposals: i64,
}

impl Default for RetentionSettings {
    fn default() -> Self {
        Self {
            proposal_ttl_days: DEFAULT_PROPOSAL_TTL_DAYS,
            retire_unused_after_days: DEFAULT_RETIRE_UNUSED_AFTER_DAYS,
            max_proposals: DEFAULT_MAX_PROPOSALS,
        }
    }
}

pub fn settings(db: &Connection) -> Result<RetentionSettings, BridgeError> {
    let row = db
        .query_row(
            "SELECT proposal_ttl_days, retire_unused_after_days, max_proposals
             FROM memory_retention_settings WHERE scope_key=?1",
            params![ACCOUNT_MEMORY_SCOPE],
            |row| {
                Ok(RetentionSettings {
                    proposal_ttl_days: row.get(0)?,
                    retire_unused_after_days: row.get(1)?,
                    max_proposals: row.get(2)?,
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or_default())
}

pub fn update_settings(
    db: &Connection,
    proposal_ttl_days: i64,
    retire_unused_after_days: Option<i64>,
    max_proposals: Option<i64>,
) -> Result<RetentionSettings, BridgeError> {
    if !PROPOSAL_TTL_CHOICES.contains(&proposal_ttl_days) {
        return Err(BridgeError::Invalid(format!(
            "Proposals can wait 3, 7, 14 or 30 days for review, not {proposal_ttl_days}."
        )));
    }
    if let Some(days) = retire_unused_after_days {
        if !RETIRE_UNUSED_CHOICES.contains(&days) {
            return Err(BridgeError::Invalid(format!(
                "Unused memories can retire after 30, 90 or 180 days, or never; not {days}."
            )));
        }
    }
    let max_proposals = max_proposals.unwrap_or(settings(db)?.max_proposals);
    if !(MIN_MAX_PROPOSALS..=MAX_MAX_PROPOSALS).contains(&max_proposals) {
        return Err(BridgeError::Invalid(format!(
            "The review queue can hold {MIN_MAX_PROPOSALS} to {MAX_MAX_PROPOSALS} proposals, not {max_proposals}."
        )));
    }
    db.execute(
        "INSERT INTO memory_retention_settings(
            scope_key, proposal_ttl_days, retire_unused_after_days, max_proposals, updated_at
         ) VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(scope_key) DO UPDATE SET proposal_ttl_days=?2,
            retire_unused_after_days=?3, max_proposals=?4, updated_at=?5",
        params![
            ACCOUNT_MEMORY_SCOPE,
            proposal_ttl_days,
            retire_unused_after_days,
            max_proposals,
            Utc::now().to_rfc3339()
        ],
    )?;
    settings(db)
}

pub fn held_proposals(db: &Connection, scope_key: &str) -> Result<i64, BridgeError> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM memory_records WHERE scope_key=?1 AND status='proposed'",
        params![scope_key],
        |row| row.get(0),
    )?)
}

/// Whether one more proposal would be refused. Extraction asks this before it
/// spends anything; active space does not matter, since a run only proposes.
pub fn proposals_full(db: &Connection, scope_key: &str) -> Result<bool, BridgeError> {
    if scope_key != ACCOUNT_MEMORY_SCOPE {
        return Ok(false);
    }
    Ok(held_proposals(db, scope_key)? >= settings(db)?.max_proposals)
}

pub(crate) fn enforce_proposal_cap(db: &Connection, scope_key: &str) -> Result<(), BridgeError> {
    if scope_key != ACCOUNT_MEMORY_SCOPE {
        return Ok(());
    }
    let cap = settings(db)?.max_proposals;
    let held = held_proposals(db, scope_key)?;
    if held < cap {
        return Ok(());
    }
    Err(BridgeError::Invalid(format!(
        "The review queue holds {held} of {cap} proposals and nothing was proposed. \
         Review or reject waiting proposals; unreviewed ones expire on their own."
    )))
}

/// Record that the packet or a search handed these records to a model.
pub(crate) fn stamp_used<'a>(
    db: &Connection,
    record_ids: impl IntoIterator<Item = &'a str>,
    now: DateTime<Utc>,
) -> Result<(), BridgeError> {
    let now = now.to_rfc3339();
    let mut statement =
        db.prepare("UPDATE memory_records SET last_used_at=?1 WHERE id=?2 AND status='active'")?;
    for id in record_ids {
        statement.execute(params![now, id])?;
    }
    Ok(())
}

fn log_sweep(db: &Connection, kind: &str, ids: &[String], now: DateTime<Utc>) -> Result<(), BridgeError> {
    if ids.is_empty() {
        return Ok(());
    }
    db.execute(
        "INSERT INTO memory_retention_sweeps(id, scope_key, kind, record_count, record_ids, at)
         VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            Uuid::new_v4().to_string(),
            ACCOUNT_MEMORY_SCOPE,
            kind,
            ids.len() as i64,
            serde_json::to_string(ids).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            now.to_rfc3339()
        ],
    )?;
    Ok(())
}

/// Expire proposals older than the configured wait, and log the sweep.
pub fn sweep_stale_proposals(db: &Connection, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let ttl = settings(db)?.proposal_ttl_days;
    let expired = memory_ledger::sweep_proposals_older_than(db, now, Duration::days(ttl))?;
    log_sweep(db, SWEEP_PROPOSALS_EXPIRED, &expired, now)?;
    Ok(expired)
}

/// Retire active memories nothing has used for the configured number of days.
/// A pin a person saved on purpose never retires. The interval closes now, so
/// an as-of read before this instant still finds the memory.
pub fn sweep_unused(db: &Connection, now: DateTime<Utc>) -> Result<Vec<String>, BridgeError> {
    let Some(days) = settings(db)?.retire_unused_after_days else {
        return Ok(Vec::new());
    };
    let cutoff = (now - Duration::days(days)).to_rfc3339();
    let mut statement = db.prepare(
        "SELECT id FROM memory_records
         WHERE scope_key=?1 AND status='active' AND provenance<>'user_explicit'
           AND COALESCE(last_used_at, created_at) <= ?2
         ORDER BY id",
    )?;
    let ids: Vec<String> = statement
        .query_map(params![ACCOUNT_MEMORY_SCOPE, cutoff], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let at = now.to_rfc3339();
    for id in &ids {
        db.execute(
            "UPDATE memory_records SET status=?1, valid_to=?2, updated_at=?2
             WHERE id=?3 AND status='active'",
            params![memory_ledger::STATUS_EXPIRED, at, id],
        )?;
    }
    log_sweep(db, SWEEP_UNUSED_RETIRED, &ids, now)?;
    Ok(ids)
}

/// Bring back a retired or expired memory: a new active record with the same
/// body, kind and provenance that supersedes it, so history still says when
/// it stopped and when it came back. A proposal that expired unreviewed never
/// held, and is not restorable here.
pub fn restore(db: &Connection, record_id: &str) -> Result<MemoryRecord, BridgeError> {
    let record_id = record_id.trim();
    let old = memory_ledger::load_record(db, record_id)?
        .ok_or_else(|| BridgeError::Invalid(format!("Memory record '{record_id}' does not exist.")))?;
    if old.status != memory_ledger::STATUS_EXPIRED {
        return Err(BridgeError::Invalid(format!(
            "Only a retired or expired memory can be restored; this one is {}.",
            old.status
        )));
    }
    if old.valid_to.as_deref() == Some(old.valid_from.as_str()) {
        return Err(BridgeError::Invalid(
            "This proposal expired before anyone approved it, so there is nothing to restore.".into(),
        ));
    }
    crate::memory_consolidation::enforce_scope_budget(db, &old.scope_key)?;
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();
    let transaction = db.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status, source_session_id,
             confidence_bps, rationale, supersedes, valid_from, valid_to, expires_at, conflict_group,
             created_at, updated_at, last_used_at)
         VALUES(?1,?2,?3,?4,?5,'active',?6,?7,?8,?9,?10,NULL,NULL,?11,?10,?10,?10)",
        params![
            id,
            old.scope_key,
            old.kind,
            old.body,
            old.provenance,
            old.source_session_id,
            old.confidence_bps,
            old.rationale,
            old.id,
            now,
            old.conflict_group
        ],
    )?;
    transaction.execute(
        "UPDATE memory_records SET superseded_by=?1 WHERE id=?2",
        params![id, old.id],
    )?;
    transaction.commit()?;
    memory_ledger::load_record(db, &id)?
        .ok_or_else(|| BridgeError::Invalid("The restored record was not written.".into()))
}

pub fn wire(db: &Connection) -> Result<MemoryRetentionSettings, BridgeError> {
    let current = settings(db)?;
    let consolidation = crate::memory_consolidation::settings(db, ACCOUNT_MEMORY_SCOPE)?;
    Ok(MemoryRetentionSettings {
        scope_key: ACCOUNT_MEMORY_SCOPE.to_string(),
        proposal_ttl_days: current.proposal_ttl_days,
        retire_unused_after_days: current.retire_unused_after_days,
        max_proposals: current.max_proposals,
        max_active: consolidation.max_records,
        held_active: crate::memory_consolidation::held_records(db, ACCOUNT_MEMORY_SCOPE)?,
        held_proposals: held_proposals(db, ACCOUNT_MEMORY_SCOPE)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn open() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn insert(db: &Connection, id: &str, status: &str, provenance: &str, created_at: &str, last_used: Option<&str>) {
        let valid_to = if status == "proposed" { Some(created_at) } else { None };
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status,
                 valid_from, valid_to, created_at, updated_at, last_used_at)
             VALUES(?1, ?2, 'fact', ?1, ?3, ?4, ?5, ?6, ?5, ?5, ?7)",
            params![id, ACCOUNT_MEMORY_SCOPE, provenance, status, created_at, valid_to, last_used],
        )
        .unwrap();
    }

    fn status(db: &Connection, id: &str) -> String {
        db.query_row("SELECT status FROM memory_records WHERE id=?1", params![id], |row| row.get(0)).unwrap()
    }

    fn budget(db: &Connection, max: i64) {
        crate::memory_consolidation::update_settings(db, ACCOUNT_MEMORY_SCOPE, "off", Some(max), None, None).unwrap();
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn proposals_no_longer_block_saves_or_approvals() {
        let (_dir, db) = open();
        budget(&db, 20);
        for index in 0..138 {
            insert(&db, &format!("p{index}"), "proposed", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        }
        for index in 0..10 {
            insert(&db, &format!("a{index}"), "active", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        }
        assert_eq!(crate::memory_consolidation::held_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap(), 10);
        memory_ledger::save(&db, "A new pin", Some("fact"), None).unwrap();
        memory_ledger::approve(&db, "p0").unwrap();
        assert_eq!(status(&db, "p0"), "active");
    }

    #[test]
    fn bulk_approval_stops_at_the_active_budget_and_says_why() {
        let (_dir, db) = open();
        budget(&db, 200);
        for index in 0..190 {
            insert(&db, &format!("a{index}"), "active", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        }
        let ids: Vec<String> = (0..60).map(|index| format!("p{index:02}")).collect();
        for id in &ids {
            insert(&db, id, "proposed", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        }
        let result = memory_ledger::review_batch(&db, ACCOUNT_MEMORY_SCOPE, &ids, &[]).unwrap();
        assert_eq!((result.approved.len(), result.refused.len()), (10, 50));
        assert!(result.refused[0].reason.contains("200"), "{}", result.refused[0].reason);
        assert_eq!(crate::memory_consolidation::held_records(&db, ACCOUNT_MEMORY_SCOPE).unwrap(), 200);
    }

    #[test]
    fn the_proposal_cap_refuses_a_proposal_and_skips_extraction() {
        let (_dir, db) = open();
        update_settings(&db, 7, Some(90), Some(10)).unwrap();
        for index in 0..10 {
            insert(&db, &format!("p{index}"), "proposed", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        }
        assert!(proposals_full(&db, ACCOUNT_MEMORY_SCOPE).unwrap());
        let refused = memory_ledger::insert_proposal(&db, ACCOUNT_MEMORY_SCOPE, "One more", "fact", Some(7000), None, "s1");
        assert!(refused.unwrap_err().to_string().contains("10 of 10 proposals"));
        assert!(!crate::memory_consolidation::scope_full(&db, ACCOUNT_MEMORY_SCOPE).unwrap(), "active space is untouched");
    }

    #[test]
    fn proposals_expire_at_exactly_the_configured_wait() {
        let (_dir, db) = open();
        update_settings(&db, 3, None, None).unwrap();
        insert(&db, "old", "proposed", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        insert(&db, "young", "proposed", "model_proposal", "2026-11-20T00:00:01+00:00", None);
        let swept = sweep_stale_proposals(&db, at("2026-11-23T00:00:00+00:00")).unwrap();
        assert_eq!(swept, vec!["old".to_string()]);
        assert_eq!(status(&db, "young"), "proposed");
        let log: (String, i64) = db
            .query_row("SELECT kind, record_count FROM memory_retention_sweeps", [], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        assert_eq!(log, (SWEEP_PROPOSALS_EXPIRED.to_string(), 1));
    }

    #[test]
    fn unused_memories_retire_but_pins_never_do() {
        let (_dir, db) = open();
        let now = at("2026-11-26T00:00:00+00:00");
        insert(&db, "stale", "active", "model_proposal", "2026-01-01T00:00:00+00:00", Some("2026-08-01T00:00:00+00:00"));
        insert(&db, "used", "active", "model_proposal", "2026-01-01T00:00:00+00:00", Some("2026-11-20T00:00:00+00:00"));
        insert(&db, "fresh", "active", "model_proposal", "2026-11-01T00:00:00+00:00", None);
        insert(&db, "pin", "active", "user_explicit", "2025-01-01T00:00:00+00:00", None);
        assert_eq!(sweep_unused(&db, now).unwrap(), vec!["stale".to_string()]);
        assert_eq!(status(&db, "pin"), "active");
        assert_eq!(status(&db, "used"), "active");
        let valid_to: String = db.query_row("SELECT valid_to FROM memory_records WHERE id='stale'", [], |row| row.get(0)).unwrap();
        assert_eq!(valid_to, now.to_rfc3339());

        update_settings(&db, 7, None, None).unwrap();
        db.execute("UPDATE memory_records SET last_used_at='2020-01-01T00:00:00+00:00' WHERE id='used'", []).unwrap();
        assert!(sweep_unused(&db, now).unwrap().is_empty(), "never retires nothing");
    }

    #[test]
    fn a_restored_memory_is_active_again_and_reaches_the_packet() {
        let (_dir, db) = open();
        insert(&db, "gone", "active", "model_proposal", "2026-01-01T00:00:00+00:00", None);
        sweep_unused(&db, at("2026-11-26T00:00:00+00:00")).unwrap();
        let restored = restore(&db, "gone").unwrap();
        assert_eq!((restored.status.as_str(), restored.supersedes.as_deref()), ("active", Some("gone")));
        let packet = crate::memory_packet::for_session(
            &db,
            ACCOUNT_MEMORY_SCOPE,
            "recipient",
            crate::memory_packet::Retrieval::PacketOnly,
        )
        .unwrap()
        .unwrap();
        assert!(packet.selected.iter().any(|item| item.record_id == restored.id));
        let stamped: Option<String> = db
            .query_row("SELECT last_used_at FROM memory_records WHERE id=?1", params![restored.id], |row| row.get(0))
            .unwrap();
        assert!(stamped.is_some(), "the packet stamps what it sends");
    }

    #[test]
    fn restore_refuses_active_records_unreviewed_proposals_and_a_full_budget() {
        let (_dir, db) = open();
        insert(&db, "live", "active", "model_proposal", "2026-11-20T00:00:00+00:00", None);
        assert!(restore(&db, "live").is_err());
        insert(&db, "unreviewed", "proposed", "model_proposal", "2026-01-01T00:00:00+00:00", None);
        sweep_stale_proposals(&db, at("2026-11-26T00:00:00+00:00")).unwrap();
        assert!(restore(&db, "unreviewed").unwrap_err().to_string().contains("nothing to restore"));
        insert(&db, "retired", "active", "model_proposal", "2026-01-01T00:00:00+00:00", None);
        sweep_unused(&db, at("2026-11-26T00:00:00+00:00")).unwrap();
        budget(&db, 10);
        for index in 0..10 {
            insert(&db, &format!("a{index}"), "active", "user_explicit", "2026-11-25T00:00:00+00:00", None);
        }
        assert!(restore(&db, "retired").unwrap_err().to_string().contains("of 10 memories"));
        assert_eq!(status(&db, "retired"), "expired");
    }

    #[test]
    fn settings_validate_and_a_refusal_changes_nothing() {
        let (_dir, db) = open();
        assert_eq!(settings(&db).unwrap(), RetentionSettings::default());
        assert!(update_settings(&db, 5, Some(90), None).is_err());
        assert!(update_settings(&db, 7, Some(45), None).is_err());
        assert!(update_settings(&db, 7, Some(90), Some(5)).is_err());
        assert_eq!(settings(&db).unwrap(), RetentionSettings::default());
        let saved = update_settings(&db, 14, None, Some(50)).unwrap();
        assert_eq!(saved, RetentionSettings { proposal_ttl_days: 14, retire_unused_after_days: None, max_proposals: 50 });
    }

    #[test]
    fn expired_records_list_and_sweeps_show_in_activity() {
        let (_dir, db) = open();
        insert(&db, "gone", "active", "model_proposal", "2026-01-01T00:00:00+00:00", None);
        sweep_unused(&db, at("2026-11-26T00:00:00+00:00")).unwrap();
        let listed = memory_ledger::list_page(&db, ACCOUNT_MEMORY_SCOPE, Some("expired"), None, None).unwrap();
        assert_eq!((listed.total, listed.records[0].id.as_str()), (1, "gone"));
        let log = crate::memory_activity::activity_log(&db, ACCOUNT_MEMORY_SCOPE).unwrap();
        assert_eq!(log.entries[0].source, "retention");
        assert_eq!(log.entries[0].detail.as_deref(), Some(SWEEP_UNUSED_RETIRED));
    }

    #[test]
    fn the_migration_backfills_last_used_from_packet_audits() {
        let (_dir, db) = open();
        insert(&db, "seen", "active", "model_proposal", "2026-01-01T00:00:00+00:00", None);
        db.execute(
            "INSERT INTO memory_retrieval_audits(id, scope_key, recipient_session_id, objective_hash, candidate_count,
                 selected_ids, exclusions, token_estimate, created_at)
             VALUES('a1', ?1, 's1', 'h', 1, '[{\"id\":\"seen\"}]', '[]', 1, '2026-11-01T00:00:00+00:00')",
            params![ACCOUNT_MEMORY_SCOPE],
        )
        .unwrap();
        db.execute("UPDATE memory_records SET last_used_at=NULL", []).unwrap();
        let transaction = db.unchecked_transaction().unwrap();
        install(&transaction).unwrap();
        transaction.commit().unwrap();
        let stamped: Option<String> =
            db.query_row("SELECT last_used_at FROM memory_records WHERE id='seen'", [], |row| row.get(0)).unwrap();
        assert_eq!(stamped.as_deref(), Some("2026-11-01T00:00:00+00:00"));
    }
}
