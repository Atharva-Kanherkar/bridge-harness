use crate::{
    context::ContextProjector,
    model::{RestorationMode, ResumeEligibility},
    model_catalog,
    session_forest::{EntryKind, SessionForest},
    store, BridgeError,
};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

/// The smallest restoration context a cold start will inject, whatever the
/// window: the old fixed cap, kept as the floor so a tiny or unknown window
/// never carries less than before.
pub const MIN_RESTORATION_BUDGET_BYTES: usize = 8_000;
/// The largest restoration context, sized under
/// [`crate::prompt_compiler::MAX_VARIABLE_SUFFIX_BYTES`] (128 KiB) with room for
/// the other variable sections — the memory packet is capped at 4k chars and
/// the session capabilities are a paragraph — so a full budget can never make
/// the compiler reject the launch.
pub const MAX_RESTORATION_BUDGET_BYTES: usize = 96 * 1024;
/// The share of the incoming model's window the carried conversation may take.
const RESTORATION_WINDOW_DIVISOR: i64 = 8;
const BYTES_PER_TOKEN_ESTIMATE: i64 = 4;

/// How many bytes of stored conversation a cold start may inject for a model
/// with this context window: one eighth of the window at four bytes per token,
/// clamped to `[MIN_RESTORATION_BUDGET_BYTES, MAX_RESTORATION_BUDGET_BYTES]`.
pub fn restoration_budget_bytes(context_window_tokens: i64) -> usize {
    let bytes = (context_window_tokens.max(0) / RESTORATION_WINDOW_DIVISOR)
        .saturating_mul(BYTES_PER_TOKEN_ESTIMATE);
    usize::try_from(bytes)
        .unwrap_or(usize::MAX)
        .clamp(MIN_RESTORATION_BUDGET_BYTES, MAX_RESTORATION_BUDGET_BYTES)
}

/// The context window of the model a session is (now) served by, read from
/// its own row. After a switch commits, that is the incoming model — exactly
/// the one whose window should size what the cold start injects.
pub fn session_context_window_tokens(
    db: &Connection,
    session_id: &str,
) -> Result<i64, BridgeError> {
    let selection: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT harness,model FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(selection
        .map(|(harness, model)| model_catalog::context_window_tokens(&harness, model.as_deref()))
        .unwrap_or(model_catalog::DEFAULT_CONTEXT_WINDOW_TOKENS))
}

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

/// The stored history a cold start injects when no native thread can be
/// resumed, sized by the session's own (incoming) model. See
/// [`checkpoint_context_with_window`].
pub fn checkpoint_context(
    db: &Connection,
    session_id: &str,
) -> Result<Option<String>, BridgeError> {
    let window = session_context_window_tokens(db, session_id)?;
    checkpoint_context_with_window(db, session_id, window)
}

/// Project the active branch into labelled restoration text for a model with
/// `context_window_tokens` of room.
///
/// The header — the newest valid compaction's summary and decisions, else the
/// latest checkpoint summary — is never dropped. Below it, the conversation
/// tail is walked newest-first and kept verbatim, whole entries only, until
/// [`restoration_budget_bytes`] is spent; if even the newest entry alone does
/// not fit, its head is trimmed so the most recent words survive. The former
/// fixed twelve-line stop and 8 000-byte cut were the "new model forgot
/// everything" experience: a 40-turn conversation arrived as a paragraph.
pub fn checkpoint_context_with_window(
    db: &Connection,
    session_id: &str,
    context_window_tokens: i64,
) -> Result<Option<String>, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let projection = ContextProjector::project(&branch, context_window_tokens.max(1))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let budget = restoration_budget_bytes(context_window_tokens);
    let mut header = Vec::new();
    if let Some(restoration) = projection.restoration_context {
        header.push(format!("compaction: {}", restoration.summary));
        for decision in restoration.decisions {
            header.push(format!("decision: {decision}"));
        }
        // What was unfinished at the boundary. Carried under the decisions so
        // the budget trim, which cuts the header's end, drops it before it
        // drops the summary or the earliest decisions.
        for open in restoration.open_work {
            header.push(format!("still open: {open}"));
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
        header.push(format!("checkpoint: {summary}"));
    }
    // Reserve the header's room first, and bound the header itself: checkpoint
    // fields have no length limit, and the compiler's ceiling must hold even
    // for a pathological summary. Cutting the header's *end* keeps the summary
    // line and the earliest decisions — the front is what carries meaning.
    let header = bound_header(header, MAX_RESTORATION_BUDGET_BYTES);
    let header_bytes = header.iter().map(|line| line.len() + 1).sum::<usize>();
    let mut remaining = budget.saturating_sub(header_bytes);
    let mut tail = Vec::new();
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
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        let line = format!("{}: {value}", entry.kind);
        let cost = line.len() + 1;
        if cost <= remaining {
            remaining -= cost;
            tail.push(line);
            continue;
        }
        let prefix = format!("{}: …", entry.kind);
        if tail.is_empty() && remaining > prefix.len() + 1 {
            // The newest entry alone is over budget: keep its most recent bytes,
            // still labelled, rather than nothing.
            let kept = keep_tail(value, remaining - prefix.len() - 1);
            tail.push(format!("{prefix}{kept}"));
        }
        break;
    }
    if header.is_empty() && tail.is_empty() {
        return Ok(None);
    }
    tail.reverse();
    let mut header = header;
    header.extend(tail);
    // By construction: header ≤ MAX, tail ≤ budget − header ≤ MAX − header.
    let context = header.join("\n");
    Ok(Some(format!(
        "Bridge checkpoint-restoration context (stored history, not native provider resume):\n{context}"
    )))
}

/// Keep the header's lines in order until `max_bytes` is spent; the line that
/// overflows is cut at its end (on a char boundary) and marked, and anything
/// after it is dropped. The summary line always survives in some form.
fn bound_header(lines: Vec<String>, max_bytes: usize) -> Vec<String> {
    let mut kept = Vec::new();
    let mut used = 0usize;
    for line in lines {
        let cost = line.len() + 1;
        if used + cost <= max_bytes {
            used += cost;
            kept.push(line);
            continue;
        }
        let room = max_bytes.saturating_sub(used + 1 + '…'.len_utf8());
        if room > 0 {
            let mut end = room.min(line.len());
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            kept.push(format!("{}…", &line[..end]));
        }
        break;
    }
    kept
}

/// The last `max_bytes` of `text`, cut forward to a char boundary.
fn keep_tail(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
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

/// Reset the head for a session that is about to be served by a *different*
/// provider, clearing the native thread id instead of coalescing it.
///
/// [`set_head_state`] deliberately keeps an existing id when passed `None`, so
/// a launch that does not know the thread id cannot wipe a good one. That is
/// wrong for exactly one caller: a cross-harness switch, where the stored id
/// belongs to an agent that will never serve this session again. Leaving it
/// behind left `session_heads.native_provider_session_id` pointing at a dead
/// thread while `sessions.provider_session_id` was already NULL, and the forest
/// snapshot surfaced the stale id.
pub fn clear_head_state_for_new_provider(
    db: &Connection,
    session_id: &str,
    mode: RestorationMode,
    eligibility: ResumeEligibility,
) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO session_heads(session_id,native_provider_session_id,restoration_mode,resume_eligibility,updated_at)
         VALUES(?1,NULL,?2,?3,?4)
         ON CONFLICT(session_id) DO UPDATE SET
            native_provider_session_id=NULL,
            restoration_mode=excluded.restoration_mode,
            resume_eligibility=excluded.resume_eligibility,
            updated_at=excluded.updated_at",
        params![
            session_id,
            mode.as_str(),
            eligibility.as_str(),
            Utc::now().to_rfc3339(),
        ],
    )?;
    Ok(())
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
    fn restoration_budget_scales_with_the_window_and_respects_the_compiler_cap() {
        assert_eq!(restoration_budget_bytes(32_000), 16_000);
        assert_eq!(restoration_budget_bytes(128_000), 64_000);
        assert_eq!(restoration_budget_bytes(200_000), MAX_RESTORATION_BUDGET_BYTES);
        assert_eq!(restoration_budget_bytes(1_000_000), MAX_RESTORATION_BUDGET_BYTES);
        assert_eq!(restoration_budget_bytes(1_000), MIN_RESTORATION_BUDGET_BYTES);
        assert_eq!(restoration_budget_bytes(-5), MIN_RESTORATION_BUDGET_BYTES);
        assert!(
            MAX_RESTORATION_BUDGET_BYTES < crate::prompt_compiler::MAX_VARIABLE_SUFFIX_BYTES,
            "a full restoration budget must leave room for the other variable sections"
        );
        assert!(restoration_budget_bytes(128_000) > 8_000, "the old fixed cap is the floor, not the ceiling");
    }

    fn forty_turn_branch(db: &Connection) {
        let forest = SessionForest::new(db);
        for turn in 0..40 {
            let (kind, speaker) = if turn % 2 == 0 {
                (EntryKind::UserMessage, "user")
            } else {
                (EntryKind::AssistantMessage, "assistant")
            };
            forest
                .append(
                    "s",
                    kind,
                    serde_json::json!({
                        "text": format!("turn {turn:02} {speaker}: {}", "the payments retry lives in src/billing/retry.ts and we keep it there. ".repeat(20)),
                    }),
                )
                .unwrap();
        }
    }

    #[test]
    fn a_forty_turn_conversation_keeps_its_recent_turns_verbatim() {
        let db = database();
        db.execute("UPDATE sessions SET harness='claude', model='claude-opus-4-6' WHERE id='s'", [])
            .unwrap();
        forty_turn_branch(&db);
        let context = checkpoint_context(&db, "s").unwrap().unwrap();
        assert!(context.len() > 8_000, "the old cap would have cut this to a paragraph: {}", context.len());
        for turn in 20..40 {
            let speaker = if turn % 2 == 0 { "user" } else { "assistant" };
            assert!(
                context.contains(&format!("turn {turn:02} {speaker}: the payments retry")),
                "turn {turn} must survive verbatim"
            );
        }
        assert!(context.contains("not native provider resume"));
    }

    #[test]
    fn the_tail_is_cut_by_the_incoming_models_budget_not_a_line_count() {
        let db = database();
        forty_turn_branch(&db);
        let wide = checkpoint_context_with_window(&db, "s", 128_000).unwrap().unwrap();
        let narrow = checkpoint_context_with_window(&db, "s", 32_000).unwrap().unwrap();
        let count = |context: &str| context.matches("\nuser.message: turn ").count()
            + context.matches("\nassistant.message: turn ").count();
        assert!(count(&wide) > 12, "more than the old twelve lines: {}", count(&wide));
        assert!(count(&narrow) < count(&wide), "a smaller window carries fewer turns");
        assert!(wide.contains("turn 39 assistant") && narrow.contains("turn 39 assistant"), "the newest turn always survives");
        let envelope = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n".len();
        assert!(narrow.len() - envelope <= restoration_budget_bytes(32_000));
        assert!(wide.len() - envelope <= restoration_budget_bytes(128_000));
    }

    #[test]
    fn the_header_survives_when_the_budget_is_tiny() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest
            .append(
                "s",
                EntryKind::Checkpoint,
                serde_json::json!({"schemaVersion":1,"summary":"we chose the SQLite token store"}),
            )
            .unwrap();
        // Multibyte payload so the head trim has boundaries to respect.
        let huge = "ünïcödé ".repeat(3_000);
        forest
            .append("s", EntryKind::AssistantMessage, serde_json::json!({"text": format!("{huge} FINAL WORDS")}))
            .unwrap();
        let context = checkpoint_context_with_window(&db, "s", 1_000).unwrap().unwrap();
        assert!(context.contains("checkpoint: we chose the SQLite token store"));
        assert!(context.ends_with("FINAL WORDS"), "the newest words survive the head trim");
        let envelope = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n".len();
        assert!(context.len() - envelope <= MIN_RESTORATION_BUDGET_BYTES);
    }

    #[test]
    fn an_oversized_header_keeps_its_summary_start_and_the_cap_holds() {
        use crate::compaction_controller::{CompactionController, CompactionReason};
        let db = database();
        let forest = SessionForest::new(&db);
        forest
            .append("s", EntryKind::UserMessage, serde_json::json!({"text":"older words"}))
            .unwrap();
        // A real compaction boundary whose summary is far over the cap.
        CompactionController::begin(&db, "s", CompactionReason::Manual, 7).unwrap().prompt().unwrap();
        let pending = CompactionController::pending(&db, "s").unwrap().unwrap();
        let huge = format!("THE POINT: we chose SQLite. {}", "filler ünïcödé ".repeat(20_000));
        let output = serde_json::json!({
            "schemaVersion":1,"summary":huge,"decisions":["first decision","second decision"],
            "filesTouched":[],"sourceAgent":"s","firstRetainedEntryId":pending.first_retained_entry_id,
            "tokensBefore":pending.tokens_before,"reason":pending.reason.as_str()
        })
        .to_string();
        CompactionController::handle_output(&db, "s", &output).unwrap();
        forest
            .append("s", EntryKind::AssistantMessage, serde_json::json!({"text":"newest words"}))
            .unwrap();
        let context = checkpoint_context_with_window(&db, "s", 128_000).unwrap().unwrap();
        let envelope = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n".len();
        assert!(context.len() - envelope <= MAX_RESTORATION_BUDGET_BYTES, "{}", context.len());
        assert!(
            context.contains("compaction: THE POINT: we chose SQLite."),
            "the header's start is what survives, never cut from the front"
        );
        assert!(context.contains('…'), "the overflowing header line is marked as cut");
        assert!(!context.contains("second decision"), "what follows the cut header line is dropped, not the summary");
    }

    #[test]
    fn the_budget_reads_the_sessions_own_model() {
        let db = database();
        assert_eq!(
            session_context_window_tokens(&db, "s").unwrap(),
            model_catalog::DEFAULT_CONTEXT_WINDOW_TOKENS
        );
        db.execute("UPDATE sessions SET harness='claude', model='claude-sonnet-4-5' WHERE id='s'", [])
            .unwrap();
        assert_eq!(session_context_window_tokens(&db, "s").unwrap(), 200_000);
        assert_eq!(
            session_context_window_tokens(&db, "no-such-session").unwrap(),
            model_catalog::DEFAULT_CONTEXT_WINDOW_TOKENS
        );
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

    #[test]
    fn a_retained_provider_id_resumes_natively_which_is_what_the_kept_id_buys() {
        // The same-harness switch keeps `sessions.provider_session_id`; the
        // next cold start must select Native on exactly that state.
        assert_eq!(
            select_plan(false, Some("thread-1"), true, false, true, false),
            RestorationPlan::Native
        );
    }

    #[test]
    fn head_state_coalesces_a_none_id_but_clears_it_when_asked() {
        let db = database();
        set_head_state(
            &db,
            "s",
            RestorationMode::Native,
            ResumeEligibility::Native,
            Some("thread-1"),
        )
        .unwrap();
        // The default: a launch that does not know the thread id must not wipe
        // a good one.
        set_head_state(
            &db,
            "s",
            RestorationMode::Native,
            ResumeEligibility::Native,
            None,
        )
        .unwrap();
        let kept: Option<String> = db
            .query_row(
                "SELECT native_provider_session_id FROM session_heads WHERE session_id='s'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(kept.as_deref(), Some("thread-1"));

        // The opt-in clear: a cross-harness switch, whose stored id belongs to
        // an agent that will never serve this session again.
        clear_head_state_for_new_provider(
            &db,
            "s",
            RestorationMode::Fresh,
            ResumeEligibility::Fresh,
        )
        .unwrap();
        let (mode, cleared): (String, Option<String>) = db
            .query_row(
                "SELECT restoration_mode,native_provider_session_id FROM session_heads WHERE session_id='s'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(mode, "fresh");
        assert_eq!(cleared, None);
    }
}
