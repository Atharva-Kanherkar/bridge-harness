use crate::{
    context::ContextProjector,
    model::{RestorationMode, ResumeEligibility},
    session_forest::{EntryKind, SessionForest},
    store, BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestorationPlan {
    Hot,
    /// Resume the stored provider thread in place.
    Native,
    /// Fork the stored provider thread into a NEW thread before the first
    /// turn — a Codex-native side chat: the fork reads the source thread's
    /// full history and every aside write lands on the fork, so the source
    /// conversation is never appended to.
    NativeFork,
    CheckpointRestored,
    Fresh,
}

pub fn fallback_after_failure(
    failed: RestorationPlan,
    has_stored_context: bool,
) -> Option<RestorationPlan> {
    match failed {
        RestorationPlan::NativeFork | RestorationPlan::Native if has_stored_context => {
            Some(RestorationPlan::CheckpointRestored)
        }
        RestorationPlan::NativeFork | RestorationPlan::Native | RestorationPlan::CheckpointRestored => {
            Some(RestorationPlan::Fresh)
        }
        RestorationPlan::Fresh | RestorationPlan::Hot => None,
    }
}

/// Decide how a cold start restores context.
///
/// `head_says_fork` is the persisted aside instruction (`native_fork` head
/// mode), independent of whether the adapter can actually fork — that gate is
/// the ladder's job. The stored thread id of a fork-headed session belongs to
/// the SOURCE conversation, so plain native resume is forbidden for it in
/// every branch: resuming would continue the parent conversation, the one
/// write a side chat must never make. The fork-honoring arm forks; every
/// other arm projects the stored brief or starts fresh.
pub fn select_plan(
    process_is_hot: bool,
    provider_session_id: Option<&str>,
    adapter_supports_native: bool,
    adapter_supports_fork: bool,
    has_stored_context: bool,
    head_says_fork: bool,
) -> RestorationPlan {
    if process_is_hot {
        RestorationPlan::Hot
    } else if head_says_fork
        && provider_session_id.is_some()
        && adapter_supports_native
        && adapter_supports_fork
    {
        RestorationPlan::NativeFork
    } else if head_says_fork {
        // A fork instruction that cannot be honored (no thread yet — e.g. a
        // model switch cleared it — or an adapter without the fork verb) must
        // still never resume whatever id is stored: that id is the parent's.
        if has_stored_context {
            RestorationPlan::CheckpointRestored
        } else {
            RestorationPlan::Fresh
        }
    } else if provider_session_id.is_some() && adapter_supports_native {
        RestorationPlan::Native
    } else if has_stored_context {
        RestorationPlan::CheckpointRestored
    } else {
        RestorationPlan::Fresh
    }
}

pub fn checkpoint_context(
    db: &Connection,
    session_id: &str,
) -> Result<Option<String>, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let projection = ContextProjector::project(&branch, 128_000)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let mut selected = Vec::new();
    if let Some(restoration) = projection.restoration_context {
        selected.push(format!("compaction: {}", restoration.summary));
        for decision in restoration.decisions {
            selected.push(format!("decision: {decision}"));
        }
    } else if let Some(summary) = branch.iter().rev().find_map(|entry| {
        (entry.kind == "checkpoint")
            .then(|| {
                entry
                    .payload
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
            })
            .flatten()
    }) {
        selected.push(format!("checkpoint: {summary}"));
    }
    for entry in projection.render_entries.iter().rev() {
        let value = match entry.kind.as_str() {
            "checkpoint" | "compaction" | "branch.summary" | "handoff.brief" => entry
                .payload
                .get("summary")
                .or_else(|| entry.payload.get("text"))
                .and_then(serde_json::Value::as_str),
            "user.message" | "assistant.message" | "worker.result" => entry
                .payload
                .get("text")
                .or_else(|| entry.payload.get("summary"))
                .or_else(|| entry.payload.pointer("/data/text"))
                .and_then(serde_json::Value::as_str),
            _ => None,
        };
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            selected.push(format!("{}: {value}", entry.kind));
        }
        if selected.len() >= 12 {
            break;
        }
    }
    if selected.is_empty() {
        return Ok(None);
    }
    selected.reverse();
    let mut context = selected.join("\n");
    if context.len() > 8_000 {
        let mut start = context.len() - 8_000;
        while !context.is_char_boundary(start) {
            start += 1;
        }
        context = context[start..].to_owned();
    }
    Ok(Some(format!(
        "Bridge checkpoint-restoration context (stored history, not native provider resume):\n{context}"
    )))
}

pub fn record_resume_failed(
    db: &Connection,
    session_id: &str,
    reason: &str,
) -> Result<(), BridgeError> {
    SessionForest::new(db)
        .append(
            session_id,
            EntryKind::SessionResumeFailed,
            serde_json::json!({"reason": reason}),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "restoration",
        "session.resume_failed",
        session_id,
        reason,
    )
}

pub fn record_checkpoint_restore_failed(
    db: &Connection,
    session_id: &str,
    reason: &str,
) -> Result<(), BridgeError> {
    SessionForest::new(db)
        .append(
            session_id,
            EntryKind::SessionResumeFailed,
            serde_json::json!({"reason": reason, "stage": "checkpoint_restored"}),
        )
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    store::event(
        db,
        "restoration",
        "session.checkpoint_restore_failed",
        session_id,
        reason,
    )
}

pub fn set_head_state(
    db: &Connection,
    session_id: &str,
    mode: RestorationMode,
    eligibility: ResumeEligibility,
    provider_session_id: Option<&str>,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO session_heads(session_id,native_provider_session_id,restoration_mode,resume_eligibility,updated_at)
         VALUES(?1,?2,?3,?4,?5)
         ON CONFLICT(session_id) DO UPDATE SET
            native_provider_session_id=COALESCE(excluded.native_provider_session_id,session_heads.native_provider_session_id),
            restoration_mode=excluded.restoration_mode,
            resume_eligibility=excluded.resume_eligibility,
            updated_at=excluded.updated_at",
        params![
            session_id,
            provider_session_id,
            mode.as_str(),
            eligibility.as_str(),
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_forest::EntryKind;
    use std::path::Path;

    fn database() -> Connection {
        let db = store::open(Path::new(":memory:")).unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/restore-demo','now')", []).unwrap();
        db.execute("INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','Oslo','Task','bridge/task','/tmp/restore-workspace','idle','now')", []).unwrap();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source) VALUES('s','w','codex','Orchestrator','stopped','reported')", []).unwrap();
        db
    }

    #[test]
    fn restoration_order_is_hot_native_checkpoint_then_fresh() {
        assert_eq!(
            select_plan(true, Some("p"), true, false, true, false),
            RestorationPlan::Hot
        );
        assert_eq!(
            select_plan(false, Some("p"), true, false, true, false),
            RestorationPlan::Native
        );
        assert_eq!(
            select_plan(false, Some("p"), false, false, true, false),
            RestorationPlan::CheckpointRestored
        );
        assert_eq!(
            select_plan(false, None, false, false, false, false),
            RestorationPlan::Fresh
        );
        assert_eq!(
            fallback_after_failure(RestorationPlan::Native, true),
            Some(RestorationPlan::CheckpointRestored)
        );
        assert_eq!(
            fallback_after_failure(RestorationPlan::Native, false),
            Some(RestorationPlan::Fresh)
        );
        assert_eq!(
            fallback_after_failure(RestorationPlan::CheckpointRestored, true),
            Some(RestorationPlan::Fresh)
        );
        assert_eq!(fallback_after_failure(RestorationPlan::Fresh, true), None);
    }

    #[test]
    fn a_fork_head_outranks_the_resume_ladder_and_falls_back_like_a_resume() {
        // The aside's explicit instruction: fork the stored thread, never
        // resume it in place.
        assert_eq!(
            select_plan(false, Some("parent-thread"), true, true, true, true),
            RestorationPlan::NativeFork
        );
        assert_eq!(
            fallback_after_failure(RestorationPlan::NativeFork, true),
            Some(RestorationPlan::CheckpointRestored),
            "a failed fork falls back to the projected brief, never to resuming the parent thread"
        );
        assert_eq!(
            fallback_after_failure(RestorationPlan::NativeFork, false),
            Some(RestorationPlan::Fresh)
        );
    }

    // The bug bugbot caught: a fork-headed aside stores the PARENT's thread id
    // in provider_session_id. If the fork cannot be honored at start time, the
    // ladder must not fall through to plain native resume — that resumes the
    // parent conversation, the one write a side chat must never make.
    #[test]
    fn a_fork_head_never_degrades_to_resuming_the_parent_thread() {
        // Fork support lost since creation (binary downgraded, backend swap):
        // the stored id is still the parent's, so project the brief.
        assert_eq!(
            select_plan(false, Some("parent-thread"), true, false, true, true),
            RestorationPlan::CheckpointRestored
        );
        // Nothing stored to project and no fork verb: fresh, still never a
        // parent-thread resume.
        assert_eq!(
            select_plan(false, Some("parent-thread"), true, false, false, true),
            RestorationPlan::Fresh
        );
        // A model switch cleared the thread but left the fork head: brief if
        // there is one, fresh otherwise — never a resume of nothing.
        assert_eq!(
            select_plan(false, None, true, true, true, true),
            RestorationPlan::CheckpointRestored
        );
        assert_eq!(
            select_plan(false, None, true, true, false, true),
            RestorationPlan::Fresh
        );
        // Resume support itself is gone: same veto.
        assert_eq!(
            select_plan(false, Some("parent-thread"), false, false, true, true),
            RestorationPlan::CheckpointRestored
        );
    }

    #[test]
    fn checkpoint_projection_uses_active_semantic_entries_only() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest
            .append(
                "s",
                EntryKind::UserMessage,
                serde_json::json!({"text":"goal"}),
            )
            .unwrap();
        forest
            .append(
                "s",
                EntryKind::Checkpoint,
                serde_json::json!({"schemaVersion":1,"summary":"stable decision"}),
            )
            .unwrap();
        forest
            .append(
                "s",
                EntryKind::ToolCompleted,
                serde_json::json!({"toolId":"secret-log","text":"raw log"}),
            )
            .unwrap();
        let context = checkpoint_context(&db, "s").unwrap().unwrap();
        assert!(context.contains("goal"));
        assert!(context.contains("stable decision"));
        assert!(!context.contains("raw log"));
        assert!(context.contains("not native provider resume"));
    }

    #[test]
    fn handoff_brief_entries_feed_the_checkpoint_projection() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest
            .append(
                "s",
                EntryKind::HandoffBrief,
                serde_json::json!({
                    "text":"Continued from another chat: we chose the SQLite token store.",
                    "sourceSessionId":"source-session",
                    "sourceHarness":"claude",
                }),
            )
            .unwrap();
        let context = checkpoint_context(&db, "s").unwrap().unwrap();
        assert!(context.contains("SQLite token store"));
        assert!(context.contains("handoff.brief"));
        assert!(context.contains("not native provider resume"));
    }

    #[test]
    fn handoff_brief_payload_requires_text() {
        let db = database();
        let forest = SessionForest::new(&db);
        assert!(forest
            .append("s", EntryKind::HandoffBrief, serde_json::json!({"summary":"x"}))
            .is_err());
    }

    #[test]
    fn resume_failure_and_mode_are_queryable_without_relabeling_fresh_as_native() {
        let db = database();
        record_resume_failed(&db, "s", "provider rejected thread").unwrap();
        set_head_state(
            &db,
            "s",
            RestorationMode::CheckpointRestored,
            ResumeEligibility::CheckpointRestored,
            Some("new-provider"),
        )
        .unwrap();
        let head = store::session_head(&db, "s").unwrap().unwrap();
        assert_eq!(head.restoration_mode, RestorationMode::CheckpointRestored);
        assert_ne!(head.restoration_mode, RestorationMode::Native);
        let entries = store::session_entries(&db, "s").unwrap();
        assert_eq!(entries.last().unwrap().kind, "session.resume_failed");
    }

    #[test]
    fn checkpoint_failure_is_audited_before_fresh_fallback() {
        let db = database();
        record_checkpoint_restore_failed(&db, "s", "checkpoint launch rejected").unwrap();
        let entries = store::session_entries(&db, "s").unwrap();
        let failure = entries.last().unwrap();
        assert_eq!(failure.kind, "session.resume_failed");
        assert_eq!(failure.payload["stage"], "checkpoint_restored");
        let events = store::state(&db).unwrap().events;
        assert_eq!(events[0].kind, "session.checkpoint_restore_failed");
    }
}
