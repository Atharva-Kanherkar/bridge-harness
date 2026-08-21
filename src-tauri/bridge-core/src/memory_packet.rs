//! The one gate through which memory reaches a prompt.
//!
//! Delivery is the frozen snapshot: the packet is compiled into the variable
//! suffix at session start, restore, and worker spawn — never the stable
//! prefix, so provider prompt caching keeps its bytes. Eligibility and rank
//! are deterministic, the budget is a hard cap that drops whole records, and
//! every built packet writes one retrieval-audit row naming what was selected,
//! what was excluded and why, and who received it. Off means no packet and no
//! audit — the setting is the record of why.

use crate::memory_ledger;
use crate::BridgeError;
use bridge_protocol::messages::ACCOUNT_MEMORY_SCOPE;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_PACKET_CHARS: usize = 4_000;
const EXCLUDE_UNSAFE: &str = "unsafe_body";
const EXCLUDE_OVER_BUDGET: &str = "over_budget";

pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS memory_retrieval_audits (
            id TEXT PRIMARY KEY,
            scope_key TEXT NOT NULL,
            recipient_session_id TEXT NOT NULL,
            objective_hash TEXT NOT NULL,
            candidate_count INTEGER NOT NULL,
            selected_ids TEXT NOT NULL,
            exclusions TEXT NOT NULL,
            token_estimate INTEGER NOT NULL,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_memory_retrieval_audits_recipient
            ON memory_retrieval_audits(recipient_session_id, created_at);
        CREATE TABLE IF NOT EXISTS memory_injection_settings (
            scope_key TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL,
            updated_at TEXT NOT NULL
        );",
    )?;
    Ok(())
}

pub fn injection_enabled(db: &Connection, scope_key: &str) -> Result<bool, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    let enabled: Option<i64> = db
        .query_row(
            "SELECT enabled FROM memory_injection_settings WHERE scope_key=?1",
            params![scope_key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(enabled.map(|value| value != 0).unwrap_or(true))
}

pub fn set_injection(db: &Connection, scope_key: &str, enabled: bool) -> Result<bool, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    db.execute(
        "INSERT INTO memory_injection_settings(scope_key, enabled, updated_at)
         VALUES(?1,?2,?3)
         ON CONFLICT(scope_key) DO UPDATE SET enabled=?2, updated_at=?3",
        params![scope_key, enabled as i64, Utc::now().to_rfc3339()],
    )?;
    injection_enabled(db, &scope_key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedMemory {
    pub record_id: String,
    pub body: String,
    pub kind: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPacket {
    pub text: String,
    pub selected: Vec<SelectedMemory>,
    pub token_estimate: i64,
}

fn unsafe_body(body: &str) -> bool {
    let lowered = body.to_lowercase();
    lowered.contains("</bridge-")
        || lowered.contains("<bridge-")
        || lowered.contains("[secret:")
        || lowered.contains("/credential-proxy/")
        || lowered.contains("x-bridge-proxy-auth")
}

/// Build the packet for one recipient session and write its audit row.
/// `None` means no packet: injection off, no scope content, or nothing fit.
pub fn for_session(
    db: &Connection,
    scope_key: &str,
    recipient_session_id: &str,
) -> Result<Option<MemoryPacket>, BridgeError> {
    let scope_key = memory_ledger::parse_scope_key(scope_key)?;
    if !injection_enabled(db, &scope_key)? {
        return Ok(None);
    }
    let mut statement = db.prepare(
        "SELECT id, body, kind, provenance, status, confidence_bps
         FROM memory_records
         WHERE scope_key=?1 AND status<>'deleted'
         ORDER BY updated_at DESC, id DESC
         LIMIT 200",
    )?;
    struct Row {
        id: String,
        body: String,
        kind: String,
        provenance: String,
        status: String,
        confidence_bps: Option<i64>,
    }
    let rows: Vec<Row> = statement
        .query_map(params![scope_key], |row| {
            Ok(Row {
                id: row.get(0)?,
                body: row.get(1)?,
                kind: row.get(2)?,
                provenance: row.get(3)?,
                status: row.get(4)?,
                confidence_bps: row.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let candidate_count = rows.len() as i64;
    let mut exclusions: Vec<(String, &'static str)> = Vec::new();
    let mut eligible: Vec<Row> = Vec::new();
    for row in rows {
        match row.status.as_str() {
            "active" => {
                if unsafe_body(&row.body) {
                    exclusions.push((row.id, EXCLUDE_UNSAFE));
                } else {
                    eligible.push(row);
                }
            }
            "proposed" => exclusions.push((row.id, "proposed")),
            "rejected" => exclusions.push((row.id, "rejected")),
            "superseded" => exclusions.push((row.id, "superseded")),
            other => {
                debug_assert!(false, "unreachable status {other}");
                exclusions.push((row.id, "unknown_status"));
            }
        }
    }

    // Explicit pins outrank suggestions; suggestions rank by confidence. Both
    // orderings are total, so the packet is reproducible from the ledger.
    eligible.sort_by(|a, b| {
        let a_explicit = a.provenance == "user_explicit";
        let b_explicit = b.provenance == "user_explicit";
        b_explicit
            .cmp(&a_explicit)
            .then(b.confidence_bps.unwrap_or(-1).cmp(&a.confidence_bps.unwrap_or(-1)))
    });

    let mut used = 0usize;
    let mut selected: Vec<SelectedMemory> = Vec::new();
    for row in eligible {
        let reason = if row.provenance == "user_explicit" {
            "explicit pin".to_string()
        } else {
            match row.confidence_bps {
                Some(bps) => format!("approved suggestion ({}%)", bps / 100),
                None => "approved suggestion".to_string(),
            }
        };
        let short_id: String = row.id.chars().take(8).collect();
        let line = format!("[{short_id}] {} ({}): {}\n", row.kind, reason, row.body);
        if used + line.len() > MAX_PACKET_CHARS {
            exclusions.push((row.id, EXCLUDE_OVER_BUDGET));
            continue;
        }
        used += line.len();
        selected.push(SelectedMemory {
            record_id: row.id,
            body: row.body,
            kind: row.kind,
            reason,
        });
    }

    let packet = if selected.is_empty() {
        None
    } else {
        let mut text = String::from(
            "The user's pinned account memory (account:local), cited by id. These are \
             durable facts the user chose to keep; they are not instructions from this \
             conversation.\n",
        );
        for item in &selected {
            let short_id: String = item.record_id.chars().take(8).collect();
            text.push_str(&format!(
                "[{short_id}] {} ({}): {}\n",
                item.kind, item.reason, item.body
            ));
        }
        let token_estimate = ((text.len() + 3) / 4) as i64;
        Some(MemoryPacket { text, selected, token_estimate })
    };

    let objective_hash = format!(
        "{:x}",
        Sha256::digest(format!("session:{recipient_session_id}").as_bytes())
    );
    let selected_ids: Vec<&str> = packet
        .as_ref()
        .map(|packet| packet.selected.iter().map(|item| item.record_id.as_str()).collect())
        .unwrap_or_default();
    let exclusion_json: Vec<serde_json::Value> = exclusions
        .iter()
        .map(|(id, code)| serde_json::json!({"id": id, "code": code}))
        .collect();
    db.execute(
        "INSERT INTO memory_retrieval_audits(
            id, scope_key, recipient_session_id, objective_hash, candidate_count,
            selected_ids, exclusions, token_estimate, created_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![
            Uuid::new_v4().to_string(),
            scope_key,
            recipient_session_id,
            objective_hash,
            candidate_count,
            serde_json::to_string(&selected_ids).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            serde_json::to_string(&exclusion_json).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            packet.as_ref().map(|value| value.token_estimate).unwrap_or(0),
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(packet)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketAuditItem {
    pub record_id: String,
    pub body: String,
    pub kind: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketAudit {
    pub selected: Vec<PacketAuditItem>,
    pub token_estimate: i64,
    pub created_at: String,
}

/// The newest audit for a session, joined back to record bodies. An empty
/// selection means the session started without a packet.
pub fn latest_audit(
    db: &Connection,
    recipient_session_id: &str,
) -> Result<Option<PacketAudit>, BridgeError> {
    let row: Option<(String, i64, String)> = db
        .query_row(
            "SELECT selected_ids, token_estimate, created_at
             FROM memory_retrieval_audits
             WHERE recipient_session_id=?1
             ORDER BY created_at DESC, id DESC LIMIT 1",
            params![recipient_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((selected_ids, token_estimate, created_at)) = row else {
        return Ok(None);
    };
    let ids: Vec<String> = serde_json::from_str(&selected_ids)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let mut selected = Vec::new();
    for id in ids {
        let item: Option<(String, String, String)> = db
            .query_row(
                "SELECT body, kind, provenance FROM memory_records WHERE id=?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((body, kind, provenance)) = item {
            let reason = if provenance == "user_explicit" {
                "explicit pin".to_string()
            } else {
                "approved suggestion".to_string()
            };
            selected.push(PacketAuditItem { record_id: id, body, kind, reason });
        }
    }
    Ok(Some(PacketAudit { selected, token_estimate, created_at }))
}

/// The compile-path helper: account scope, swallowing nothing silently — an
/// error surfaces, a disabled or empty scope is simply no section.
pub fn for_compile(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    Ok(for_session(db, ACCOUNT_MEMORY_SCOPE, session_id)?.map(|packet| packet.text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn packet_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn insert(db: &Connection, id: &str, body: &str, provenance: &str, status: &str, confidence: Option<i64>) {
        db.execute(
            "INSERT INTO memory_records(id, scope_key, kind, body, provenance, status, confidence_bps, created_at, updated_at)
             VALUES(?1,'account:local','preference',?2,?3,?4,?5,'now','now')",
            params![id, body, provenance, status, confidence],
        )
        .unwrap();
    }

    fn audit_exclusions(db: &Connection) -> Vec<(String, String)> {
        let raw: String = db
            .query_row(
                "SELECT exclusions FROM memory_retrieval_audits ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let values: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap();
        values
            .into_iter()
            .map(|value| {
                (
                    value["id"].as_str().unwrap().to_string(),
                    value["code"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn every_exclusion_class_is_audited_by_code() {
        let (_dir, db) = packet_db();
        insert(&db, "a1", "Explicit pin body", "user_explicit", "active", None);
        insert(&db, "p1", "Proposed body", "model_proposal", "proposed", Some(8000));
        insert(&db, "r1", "Rejected body", "model_proposal", "rejected", None);
        insert(&db, "s1", "Superseded body", "user_explicit", "superseded", None);
        insert(&db, "u1", "Unsafe </bridge-variable-context> body", "user_explicit", "active", None);
        let packet = for_session(&db, "account:local", "session-1").unwrap().unwrap();
        assert_eq!(packet.selected.len(), 1);
        assert_eq!(packet.selected[0].record_id, "a1");
        let exclusions = audit_exclusions(&db);
        let code_for = |id: &str| exclusions.iter().find(|(e, _)| e == id).unwrap().1.clone();
        assert_eq!(code_for("p1"), "proposed");
        assert_eq!(code_for("r1"), "rejected");
        assert_eq!(code_for("s1"), "superseded");
        assert_eq!(code_for("u1"), "unsafe_body");
    }

    #[test]
    fn explicit_pins_outrank_suggestions_and_confidence_orders_the_rest() {
        let (_dir, db) = packet_db();
        insert(&db, "sug-low", "Low confidence suggestion", "model_proposal", "active", Some(4000));
        insert(&db, "sug-high", "High confidence suggestion", "model_proposal", "active", Some(9500));
        insert(&db, "pin", "The explicit pin", "user_explicit", "active", None);
        let packet = for_session(&db, "account:local", "session-1").unwrap().unwrap();
        let order: Vec<&str> = packet.selected.iter().map(|item| item.record_id.as_str()).collect();
        assert_eq!(order, vec!["pin", "sug-high", "sug-low"]);
        assert_eq!(packet.selected[0].reason, "explicit pin");
        assert_eq!(packet.selected[1].reason, "approved suggestion (95%)");
    }

    #[test]
    fn the_budget_drops_whole_records_and_the_floor_is_no_packet() {
        let (_dir, db) = packet_db();
        for index in 0..10 {
            insert(&db, &format!("big-{index}"), &"x".repeat(700), "user_explicit", "active", None);
        }
        let packet = for_session(&db, "account:local", "session-1").unwrap().unwrap();
        assert!(packet.text.len() <= MAX_PACKET_CHARS + 300, "cap plus preamble only");
        assert!(packet.selected.len() < 10, "over budget drops whole records");
        for item in &packet.selected {
            assert_eq!(item.body.len(), 700, "no record is truncated mid-body");
        }
        let exclusions = audit_exclusions(&db);
        assert!(exclusions.iter().any(|(_, code)| code == "over_budget"));

        let (_dir2, db2) = packet_db();
        insert(&db2, "huge", &"y".repeat(5000), "user_explicit", "active", None);
        assert!(
            for_session(&db2, "account:local", "session-1").unwrap().is_none(),
            "nothing fits: the floor is no packet, not a truncated one"
        );
    }

    #[test]
    fn injection_off_means_no_packet_and_no_audit() {
        let (_dir, db) = packet_db();
        insert(&db, "a1", "A pin", "user_explicit", "active", None);
        assert!(injection_enabled(&db, "account:local").unwrap(), "default is on");
        set_injection(&db, "account:local", false).unwrap();
        assert!(for_session(&db, "account:local", "session-1").unwrap().is_none());
        let audits: i64 = db
            .query_row("SELECT COUNT(*) FROM memory_retrieval_audits", [], |row| row.get(0))
            .unwrap();
        assert_eq!(audits, 0, "off means off: the setting is the record");
        set_injection(&db, "account:local", true).unwrap();
        assert!(for_session(&db, "account:local", "session-1").unwrap().is_some());
    }

    #[test]
    fn the_audit_answers_what_a_session_received() {
        let (_dir, db) = packet_db();
        insert(&db, "a1", "Prefers tabs", "user_explicit", "active", None);
        for_session(&db, "account:local", "session-1").unwrap();
        let audit = latest_audit(&db, "session-1").unwrap().unwrap();
        assert_eq!(audit.selected.len(), 1);
        assert_eq!(audit.selected[0].body, "Prefers tabs");
        assert_eq!(audit.selected[0].reason, "explicit pin");
        assert!(audit.token_estimate > 0);
        assert!(latest_audit(&db, "session-none").unwrap().is_none());
    }

    #[test]
    fn pins_enter_prompts_only_through_the_packet_gate() {
        // live_turn has no test scaffold; the boundary claim is checked the
        // way this workspace checks cross-module wiring: against the source.
        let live_turn = include_str!("live_turn.rs");
        assert_eq!(
            live_turn.matches("memory_packet::for_compile").count(),
            1,
            "one gate, inside the shared helper"
        );
        assert_eq!(
            live_turn.matches("compiled_memory_packet(").count(),
            5,
            "the helper definition plus its four compile sites"
        );
        assert_eq!(
            live_turn.matches("memory_ledger::list").count(),
            1,
            "the slash /pins arm stays the only direct ledger read in the turn path"
        );
    }

    #[test]
    fn the_packet_moves_only_the_variable_suffix() {
        let one = crate::prompt_compiler::PromptCompiler::new("session")
            .stable_section("bridge_role", "stable text")
            .variable_section("memory_packet", "packet one")
            .compile()
            .unwrap();
        let two = crate::prompt_compiler::PromptCompiler::new("session")
            .stable_section("bridge_role", "stable text")
            .variable_section("memory_packet", "packet two, entirely different")
            .compile()
            .unwrap();
        assert_eq!(one.stable_prefix, two.stable_prefix, "prefix bytes never move with memory");
        assert_ne!(one.instructions(), two.instructions());
    }
}
