//! Read-only analytics behind the Memory screen's Activity tab: how often
//! memories reached a packet, which kinds carry their weight, what the gate
//! held back, and the settled extraction and consolidation runs.
//!
//! Everything is folded from tables Bridge already writes: the retrieval
//! audits (one row per built packet), the ledger, and the two run tables.
//! It measures delivery, not benefit. A memory that reached a prompt may or
//! may not have changed the answer, and nothing here claims it did.

use std::collections::BTreeMap;

use bridge_protocol::messages as wire;
use chrono::{DateTime, Duration, Local, Utc};
use rusqlite::{params, Connection};

use crate::memory_ledger;
use crate::memory_packet::{GATE_EXCLUSION_CODES, MAX_PACKET_CHARS};
use crate::BridgeError;

const DAYS: usize = 14;
const LOG_LIMIT: i64 = 30;

/// 0 = 13 days ago … 13 = today, in the user's local calendar. `None` outside
/// the window or for an unparseable timestamp.
fn day_bucket(created_at: &str, today: chrono::NaiveDate) -> Option<usize> {
    let at = DateTime::parse_from_rfc3339(created_at).ok()?;
    let age = (today - at.with_timezone(&Local).date_naive()).num_days();
    (0..DAYS as i64).contains(&age).then(|| DAYS - 1 - age as usize)
}

/// One audit's selection: current rows hold objects, older rows bare ids.
fn selected_ids(raw: &str) -> Vec<String> {
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| entry.get("id").unwrap_or(entry).as_str().map(str::to_owned))
        .collect()
}

pub fn recall_stats(db: &Connection, scope_key: &str) -> Result<wire::MemoryRecallStats, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    let today = Local::now().date_naive();

    struct Record {
        id: String,
        kind: String,
        status: String,
        chars: i64,
    }
    let records: Vec<Record> = {
        let mut statement = db.prepare(
            "SELECT id, kind, status, body FROM memory_records
             WHERE scope_key=?1 AND status!='deleted'",
        )?;
        let rows = statement
            .query_map(params![scope_key], |row| {
                let body: String = row.get(3)?;
                Ok(Record {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    status: row.get(2)?,
                    chars: body.trim().chars().count() as i64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };

    // Per record: the 14-day series and the newest day it was recalled.
    let mut per_record: BTreeMap<&str, ([i64; DAYS], i64)> =
        records.iter().map(|record| (record.id.as_str(), ([0; DAYS], -1))).collect();
    let mut injections = vec![0_i64; DAYS];
    let mut packets = 0_i64;
    let mut packets_with_memories = 0_i64;
    let mut exclusions: BTreeMap<String, i64> = BTreeMap::new();

    // A day of slack on the cut so the local-calendar bucket decides, not the
    // UTC boundary.
    let cutoff = (Utc::now() - Duration::days(DAYS as i64 + 1)).to_rfc3339();
    let audits = {
        let mut statement = db.prepare(
            "SELECT selected_ids, exclusions, created_at FROM memory_retrieval_audits
             WHERE scope_key=?1 AND created_at>=?2",
        )?;
        let rows = statement
            .query_map(params![scope_key, cutoff], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    };
    for (selected, excluded, created_at) in audits {
        let Some(day) = day_bucket(&created_at, today) else { continue };
        packets += 1;
        injections[day] += 1;
        let ids = selected_ids(&selected);
        if !ids.is_empty() {
            packets_with_memories += 1;
        }
        for id in &ids {
            if let Some((daily, last)) = per_record.get_mut(id.as_str()) {
                daily[day] += 1;
                *last = (*last).max(day as i64);
            }
        }
        if let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(&excluded) {
            for item in items {
                if let Some(code) = item.get("code").and_then(|code| code.as_str()) {
                    if GATE_EXCLUSION_CODES.contains(&code) {
                        *exclusions.entry(code.to_owned()).or_default() += 1;
                    }
                }
            }
        }
    }

    let mut by_kind: BTreeMap<&str, wire::MemoryKindUse> = BTreeMap::new();
    let mut active_records = 0_i64;
    let mut recalled_records = 0_i64;
    let mut budget_chars = 0_i64;
    for record in records.iter().filter(|record| record.status == "active") {
        let recalls: i64 = per_record[record.id.as_str()].0.iter().sum();
        active_records += 1;
        budget_chars += record.chars;
        let entry = by_kind.entry(record.kind.as_str()).or_insert_with(|| wire::MemoryKindUse {
            kind: record.kind.clone(),
            active: 0,
            recalled: 0,
            recalls: 0,
        });
        entry.active += 1;
        entry.recalls += recalls;
        if recalls > 0 {
            entry.recalled += 1;
            recalled_records += 1;
        }
    }

    Ok(wire::MemoryRecallStats {
        per_record: records
            .iter()
            .map(|record| {
                let (daily, last) = per_record[record.id.as_str()];
                let recalls: i64 = daily.iter().sum();
                wire::MemoryRecallStat {
                    id: record.id.clone(),
                    recalls,
                    last_recalled_day: last,
                    in_packet_ratio: if packets == 0 { 0.0 } else { recalls as f64 / packets as f64 },
                    daily: daily.to_vec(),
                }
            })
            .collect(),
        injections_per_day: injections,
        budget_chars_used: budget_chars.min(MAX_PACKET_CHARS as i64),
        budget_chars_max: MAX_PACKET_CHARS as i64,
        packets,
        packets_with_memories,
        active_records,
        recalled_records,
        by_kind: by_kind.into_values().collect(),
        exclusions: exclusions
            .into_iter()
            .map(|(code, count)| wire::MemoryExclusionCount { code, count })
            .collect(),
    })
}

/// Settled extraction and consolidation runs, newest first.
pub fn activity_log(db: &Connection, scope_key: &str) -> Result<wire::MemoryActivityLog, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    let mut statement = db.prepare(
        "SELECT source, status, applied, refused, detail, at FROM (
             SELECT 'extraction' AS source, status, proposal_count AS applied, 0 AS refused,
                    detail, updated_at AS at
             FROM memory_extraction_runs
             WHERE scope_key=?1 AND status IN ('completed','failed','cancelled')
             UNION ALL
             SELECT 'consolidation', status, applied_count, refused_count, detail, updated_at
             FROM memory_consolidation_runs
             WHERE scope_key=?1 AND status IN ('completed','failed','cancelled')
         ) ORDER BY at DESC LIMIT ?2",
    )?;
    let entries = statement
        .query_map(params![scope_key, LOG_LIMIT], |row| {
            Ok(wire::MemoryActivityEntry {
                source: row.get(0)?,
                status: row.get(1)?,
                applied: row.get(2)?,
                refused: row.get(3)?,
                detail: row.get(4)?,
                at: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(wire::MemoryActivityLog { entries })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE memory_records(id TEXT PRIMARY KEY, scope_key TEXT NOT NULL, kind TEXT NOT NULL,
                 body TEXT NOT NULL, status TEXT NOT NULL);
             CREATE TABLE memory_retrieval_audits(id TEXT PRIMARY KEY, scope_key TEXT NOT NULL,
                 selected_ids TEXT NOT NULL, exclusions TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE memory_extraction_runs(id TEXT PRIMARY KEY, scope_key TEXT NOT NULL, status TEXT NOT NULL,
                 proposal_count INTEGER NOT NULL DEFAULT 0, detail TEXT, updated_at TEXT NOT NULL);
             CREATE TABLE memory_consolidation_runs(id TEXT PRIMARY KEY, scope_key TEXT NOT NULL, status TEXT NOT NULL,
                 applied_count INTEGER NOT NULL DEFAULT 0, refused_count INTEGER NOT NULL DEFAULT 0,
                 detail TEXT, updated_at TEXT NOT NULL);",
        )
        .unwrap();
        db
    }

    fn record(db: &Connection, id: &str, kind: &str, body: &str, status: &str) {
        db.execute(
            "INSERT INTO memory_records VALUES(?1,'account:local',?2,?3,?4)",
            params![id, kind, body, status],
        )
        .unwrap();
    }

    fn audit(db: &Connection, id: &str, selected: &str, exclusions: &str, days_ago: i64) {
        db.execute(
            "INSERT INTO memory_retrieval_audits VALUES(?1,'account:local',?2,?3,?4)",
            params![id, selected, exclusions, (Utc::now() - Duration::days(days_ago)).to_rfc3339()],
        )
        .unwrap();
    }

    #[test]
    fn an_empty_ledger_is_all_zeros_never_nan() {
        let stats = recall_stats(&db(), "account:local").unwrap();
        assert_eq!(stats.packets, 0);
        assert_eq!(stats.injections_per_day, vec![0; 14]);
        assert_eq!(stats.budget_chars_used, 0);
        assert_eq!(stats.budget_chars_max, 4000);
        assert!(stats.per_record.is_empty() && stats.by_kind.is_empty() && stats.exclusions.is_empty());
    }

    #[test]
    fn folds_objects_and_bare_ids_and_counts_only_gate_exclusions() {
        let db = db();
        record(&db, "a", "preference", "Prefers tabs", "active");
        record(&db, "b", "fact", "Uses bun", "active");
        record(&db, "c", "fact", "Old claim", "superseded");
        audit(
            &db,
            "1",
            r#"[{"id":"a","body":"x","kind":"preference","reason":"r"}]"#,
            r#"[{"id":"c","code":"superseded"},{"id":"b","code":"over_budget"}]"#,
            0,
        );
        audit(&db, "2", r#"["a","b"]"#, "[]", 3);
        audit(&db, "3", "[]", r#"[{"id":null,"code":"candidate_window_truncated"}]"#, 3);
        audit(&db, "4", r#"["a"]"#, "[]", 30);
        let stats = recall_stats(&db, "account:local").unwrap();
        assert_eq!((stats.packets, stats.packets_with_memories), (3, 2));
        assert_eq!(stats.injections_per_day[13], 1);
        assert_eq!(stats.injections_per_day[10], 2);
        let a = stats.per_record.iter().find(|stat| stat.id == "a").unwrap();
        assert_eq!((a.recalls, a.last_recalled_day), (2, 13));
        assert!((a.in_packet_ratio - 2.0 / 3.0).abs() < 1e-9);
        let c = stats.per_record.iter().find(|stat| stat.id == "c").unwrap();
        assert_eq!((c.recalls, c.last_recalled_day), (0, -1));
        assert_eq!((stats.active_records, stats.recalled_records), (2, 2));
        let codes: Vec<_> = stats.exclusions.iter().map(|item| (item.code.as_str(), item.count)).collect();
        assert_eq!(codes, vec![("candidate_window_truncated", 1), ("over_budget", 1)]);
        let fact = stats.by_kind.iter().find(|item| item.kind == "fact").unwrap();
        assert_eq!((fact.active, fact.recalled, fact.recalls), (1, 1, 1));
        assert_eq!(stats.budget_chars_used, "Prefers tabs".len() as i64 + "Uses bun".len() as i64);
    }

    #[test]
    fn the_log_merges_settled_runs_newest_first() {
        let db = db();
        db.execute_batch(
            "INSERT INTO memory_extraction_runs VALUES('e1','account:local','completed',3,NULL,'2026-01-02T00:00:00+00:00');
             INSERT INTO memory_extraction_runs VALUES('e2','account:local','queued',0,NULL,'2026-01-09T00:00:00+00:00');
             INSERT INTO memory_consolidation_runs VALUES('c1','account:local','failed',0,2,'budget','2026-01-03T00:00:00+00:00');",
        )
        .unwrap();
        let log = activity_log(&db, "account:local").unwrap();
        let rows: Vec<_> = log
            .entries
            .iter()
            .map(|entry| (entry.source.as_str(), entry.status.as_str(), entry.applied, entry.refused))
            .collect();
        assert_eq!(rows, vec![("consolidation", "failed", 0, 2), ("extraction", "completed", 3, 0)]);
    }
}
