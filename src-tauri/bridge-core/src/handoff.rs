//! Portable, versioned continuation contract for cross-harness work.

use crate::{
    model::ContinuationFidelity,
    restoration,
    session_forest::{append_in_transaction, EntryKind, SessionForest},
    store, BridgeError,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const HANDOFF_PACKET_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffAssessment {
    pub source_harness: String,
    pub target_harness: String,
    pub cross_harness: bool,
    pub at_phase_boundary: bool,
}

pub fn assess(
    db: &Connection,
    parent_session_id: &str,
    target_harness: &str,
) -> Result<HandoffAssessment, BridgeError> {
    let (source_harness, active_turn_id): (String, Option<String>) = db.query_row(
        "SELECT harness,active_turn_id FROM sessions WHERE id=?1",
        params![parent_session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let head_kind: Option<String> = db
        .query_row(
            "SELECT e.kind FROM session_heads h LEFT JOIN session_entries e ON e.id=h.active_entry_id WHERE h.session_id=?1",
            params![parent_session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let at_phase_boundary = active_turn_id.is_none()
        || matches!(
            head_kind.as_deref(),
            Some("checkpoint" | "compaction" | "worker.result")
        );
    Ok(HandoffAssessment {
        cross_harness: source_harness != target_harness,
        source_harness,
        target_harness: target_harness.to_owned(),
        at_phase_boundary,
    })
}

pub fn fidelity_for_projection(assessment: &HandoffAssessment) -> ContinuationFidelity {
    if assessment.at_phase_boundary {
        ContinuationFidelity::ProjectedAtBoundary
    } else {
        ContinuationFidelity::ProjectedMidTurn
    }
}

pub fn record_fidelity(
    db: &Connection,
    session_id: &str,
    fidelity: ContinuationFidelity,
) -> Result<(), BridgeError> {
    db.execute(
        "UPDATE sessions SET continuation_fidelity=?2 WHERE id=?1",
        params![session_id, fidelity.as_str()],
    )?;
    Ok(())
}

/// Project a source session's stored context into a different session as a
/// durable, labelled [`EntryKind::HandoffBrief`] entry. This is how the
/// `$harness` composer shortcut gives a brand-new sibling chat the
/// conversation it was asked about: the new session's first cold start finds
/// the brief on its own active branch and injects it as restoration context.
///
/// Best-effort by contract: anything uncarriable — self-carry, unknown
/// session, an empty or unprojectable source branch — reports `Ok(false)` and
/// leaves both forests untouched rather than failing the caller.
pub fn carry_brief(
    db: &Connection,
    target_session_id: &str,
    source_session_id: &str,
) -> Result<bool, BridgeError> {
    if target_session_id == source_session_id {
        return Ok(false);
    }
    for session_id in [target_session_id, source_session_id] {
        let exists: i64 = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            params![session_id],
            |row| row.get(0),
        )?;
        if exists != 1 {
            return Ok(false);
        }
    }
    // Sized by the TARGET: the brief is what the new chat's model reads.
    let window = restoration::session_context_window_tokens(db, target_session_id)?;
    let Some(context) =
        restoration::checkpoint_context_with_window(db, source_session_id, window)?
    else {
        return Ok(false);
    };
    let source_harness: String = db.query_row(
        "SELECT harness FROM sessions WHERE id=?1",
        params![source_session_id],
        |row| row.get(0),
    )?;
    SessionForest::new(db)
        .append(
            target_session_id,
            EntryKind::HandoffBrief,
            brief_payload(&context, source_session_id, &source_harness),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "chat",
        "chat.handoff_carried",
        target_session_id,
        &format!("Carried projected context from session {source_session_id}"),
    )?;
    Ok(true)
}

/// Transaction-aware variant used when session creation and handoff must
/// commit as one operation. It shares the caller's forest and audit commit.
pub fn carry_brief_in_transaction(
    transaction: &Transaction<'_>,
    target_session_id: &str,
    source_session_id: &str,
) -> Result<bool, BridgeError> {
    if target_session_id == source_session_id { return Ok(false); }
    for session_id in [target_session_id, source_session_id] {
        let exists: i64 = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)", params![session_id], |row| row.get(0),
        )?;
        if exists != 1 { return Ok(false); }
    }
    let window = restoration::session_context_window_tokens(transaction, target_session_id)?;
    let Some(context) = restoration::checkpoint_context_with_window(transaction, source_session_id, window)? else { return Ok(false); };
    let source_harness: String = transaction.query_row(
        "SELECT harness FROM sessions WHERE id=?1", params![source_session_id], |row| row.get(0),
    )?;
    append_in_transaction(transaction, target_session_id, EntryKind::HandoffBrief, brief_payload(&context, source_session_id, &source_harness))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        transaction, "chat", "chat.handoff_carried", target_session_id,
        &format!("Carried projected context from session {source_session_id}"),
    )?;
    Ok(true)
}

/// Longest `summary` a brief carries for a UI card. `text` is the full
/// budgeted context and can now run to tens of kilobytes; a card wants a line.
pub const BRIEF_SUMMARY_MAX_CHARS: usize = 200;

/// The `handoff.brief` payload: the full carried context as `text` (required
/// by the forest) plus a one-line `summary` for any card that renders it.
fn brief_payload(context: &str, source_session_id: &str, source_harness: &str) -> serde_json::Value {
    json!({
        "text": context,
        "summary": brief_summary(context),
        "sourceSessionId": source_session_id,
        "sourceHarness": source_harness,
    })
}

/// The first content line after the restoration envelope, cut to
/// [`BRIEF_SUMMARY_MAX_CHARS`] characters.
fn brief_summary(context: &str) -> String {
    let line = context
        .lines()
        .skip(1)
        .map(str::trim)
        .find(|line| !line.is_empty())
        .or_else(|| context.lines().next())
        .unwrap_or_default();
    let mut summary: String = line.chars().take(BRIEF_SUMMARY_MAX_CHARS).collect();
    if summary.chars().count() < line.chars().count() {
        summary.push('…');
    }
    summary
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HandoffPacket {
    pub schema_version: u32,
    pub trace_id: String,
    pub source_harness: String,
    pub target_harness: String,
    pub repository_revision: Option<String>,
    pub permission_mode: String,
    pub budget: serde_json::Value,
    pub evidence_ids: Vec<String>,
    pub parent_entry_id: Option<String>,
    pub output_contract: String,
}

impl HandoffPacket {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != HANDOFF_PACKET_SCHEMA_VERSION {
            return Err(format!(
                "unsupported handoff packet schema {}",
                self.schema_version
            ));
        }
        for (field, value) in [
            ("traceId", self.trace_id.as_str()),
            ("sourceHarness", self.source_harness.as_str()),
            ("targetHarness", self.target_harness.as_str()),
            ("permissionMode", self.permission_mode.as_str()),
            ("outputContract", self.output_contract.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{field} is required"));
            }
        }
        if self.evidence_ids.iter().any(|id| id.trim().is_empty()) {
            return Err("evidenceIds cannot contain empty values".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        session_forest::{EntryKind, SessionForest},
        store,
    };
    use std::path::Path;

    fn database(session_ids: &[&str]) -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/handoff','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/handoff-w','idle','now')", []).unwrap();
        for session_id in session_ids {
            db.execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES(?1,'w','codex','Chat','idle','reported')",
                params![session_id],
            )
            .unwrap();
            db.execute(
                "INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES(?1,'fresh','now')",
                params![session_id],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn carry_handoff_projects_the_source_branch_into_the_target() {
        let db = database(&["source", "target"]);
        SessionForest::new(&db)
            .append(
                "source",
                EntryKind::UserMessage,
                json!({"text":"we chose the SQLite token store"}),
            )
            .unwrap();

        assert!(carry_brief(&db, "target", "source").unwrap());

        let entries = store::session_entries(&db, "target").unwrap();
        let brief = entries.last().unwrap();
        assert_eq!(brief.kind, "handoff.brief");
        assert_eq!(brief.payload["sourceSessionId"], "source");
        assert!(brief.payload["text"]
            .as_str()
            .unwrap()
            .contains("SQLite token store"));
        let summary = brief.payload["summary"].as_str().unwrap();
        assert!(summary.contains("SQLite token store"), "{summary}");
        assert!(!summary.contains("Bridge checkpoint-restoration"), "the envelope is not the summary");
        assert!(summary.chars().count() <= BRIEF_SUMMARY_MAX_CHARS + 1);
    }

    #[test]
    fn a_long_carried_context_yields_a_one_line_summary_and_a_target_sized_text() {
        let db = database(&["source", "target"]);
        let forest = SessionForest::new(&db);
        for turn in 0..40 {
            forest
                .append(
                    "source",
                    EntryKind::UserMessage,
                    json!({"text": format!("turn {turn:02}: {}", "we chose the SQLite token store. ".repeat(10))}),
                )
                .unwrap();
        }
        // The target is a Claude chat: its window, not the source's, sizes the brief.
        db.execute("UPDATE sessions SET harness='claude', model='claude-opus-4-6' WHERE id='target'", [])
            .unwrap();
        assert!(carry_brief(&db, "target", "source").unwrap());
        let entries = store::session_entries(&db, "target").unwrap();
        let brief = entries.last().unwrap();
        let text = brief.payload["text"].as_str().unwrap();
        assert!(text.len() > 8_000, "the brief is no longer capped at 8 KB: {}", text.len());
        assert!(text.contains("turn 39:"), "the newest turn is carried verbatim");
        let summary = brief.payload["summary"].as_str().unwrap();
        assert!(summary.chars().count() <= BRIEF_SUMMARY_MAX_CHARS + 1, "{summary}");
        assert!(!summary.contains('\n'));
    }

    #[test]
    fn carry_handoff_refuses_self_unknown_and_empty_sources() {
        let db = database(&["source", "target"]);
        // Self-carry is refused without touching the forest.
        assert!(!carry_brief(&db, "source", "source").unwrap());
        // Unknown sessions are refused.
        assert!(!carry_brief(&db, "target", "no-such-session").unwrap());
        assert!(!carry_brief(&db, "no-such-session", "source").unwrap());
        // An empty source branch has nothing to project.
        assert!(!carry_brief(&db, "target", "source").unwrap());
        assert!(store::session_entries(&db, "target").unwrap().is_empty());
    }

    #[test]
    fn versioned_packet_requires_portable_handoff_fields() {
        let packet = HandoffPacket {
            schema_version: 1,
            trace_id: "a".into(),
            source_harness: "codex".into(),
            target_harness: "claude".into(),
            repository_revision: Some("abc".into()),
            permission_mode: "workspace-write".into(),
            budget: serde_json::json!({"tokens": 10}),
            evidence_ids: vec!["e1".into()],
            parent_entry_id: Some("entry".into()),
            output_contract: "implementation-result".into(),
        };
        assert!(packet.validate().is_ok());
        assert!(HandoffPacket {
            schema_version: 2,
            ..packet
        }
        .validate()
        .is_err());
    }

    #[test]
    fn assessment_prefers_completed_turns_checkpoints_and_worker_results() {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/handoff','now')",
            [],
        )
        .unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/handoff-w','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,active_turn_id) VALUES('parent','w','codex','Parent','working','reported','turn')", []).unwrap();
        db.execute("INSERT INTO session_heads(session_id,restoration_mode,updated_at) VALUES('parent','fresh','now')", []).unwrap();

        let mid_turn = assess(&db, "parent", "claude").unwrap();
        assert!(mid_turn.cross_harness);
        assert!(!mid_turn.at_phase_boundary);
        assert_eq!(
            fidelity_for_projection(&mid_turn),
            ContinuationFidelity::ProjectedMidTurn
        );

        SessionForest::new(&db)
            .append(
                "parent",
                EntryKind::WorkerResult,
                serde_json::json!({"status":"completed","summary":"verified"}),
            )
            .unwrap();
        let verified = assess(&db, "parent", "claude").unwrap();
        assert!(verified.at_phase_boundary);
        assert_eq!(
            fidelity_for_projection(&verified),
            ContinuationFidelity::ProjectedAtBoundary
        );

        db.execute(
            "UPDATE sessions SET active_turn_id=NULL WHERE id='parent'",
            [],
        )
        .unwrap();
        assert!(assess(&db, "parent", "claude").unwrap().at_phase_boundary);
        assert!(!assess(&db, "parent", "codex").unwrap().cross_harness);
    }
}
