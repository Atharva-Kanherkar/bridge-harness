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
/// The share of the tail budget one entry may take before it is clipped: a
/// pasted file or a log dump is one turn, and one turn must not be the whole
/// carried conversation.
const RESTORATION_ENTRY_DIVISOR: usize = 6;
/// The smallest per-entry allowance, so a tiny budget still carries a whole
/// ordinary turn rather than a fragment of one.
pub const MIN_RESTORATION_ENTRY_BYTES: usize = 2_000;

/// How many bytes of one entry's text a cold start keeps verbatim under a
/// given tail budget. What is over the allowance is cut from the middle, so
/// the entry's opening and its most recent words both survive.
pub fn restoration_entry_bytes(budget: usize) -> usize {
    (budget / RESTORATION_ENTRY_DIVISOR).max(MIN_RESTORATION_ENTRY_BYTES)
}

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

/// What heads a cold start's handoff text: the newest valid compaction
/// boundary, else the newest checkpoint on the branch.
///
/// A model switch's summary is a checkpoint, not a boundary, so it heads the
/// text without cutting the tail: everything older stays eligible under the
/// budget. Decisions and open work ride with either source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffHeader {
    /// `compaction` or `checkpoint`: the line's label in the handoff text.
    pub label: &'static str,
    pub summary: String,
    pub decisions: Vec<String>,
    pub files_touched: Vec<String>,
    pub open_work: Vec<String>,
}

pub fn handoff_header(
    branch: &[crate::model::SessionEntry],
    projection: &crate::context::ContextProjection,
) -> Option<HandoffHeader> {
    if let Some(restoration) = &projection.restoration_context {
        return Some(HandoffHeader {
            label: "compaction",
            summary: restoration.summary.clone(),
            decisions: restoration.decisions.clone(),
            files_touched: restoration.files_touched.clone(),
            open_work: restoration.open_work.clone(),
        });
    }
    let strings = |entry: &crate::model::SessionEntry, key: &str| {
        entry
            .payload
            .get(key)
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    branch.iter().rev().find_map(|entry| {
        let summary = (entry.kind == "checkpoint")
            .then(|| entry.payload.get("summary").and_then(serde_json::Value::as_str))
            .flatten()
            .map(str::trim)
            .filter(|summary| !summary.is_empty())?;
        Some(HandoffHeader {
            label: "checkpoint",
            summary: summary.to_owned(),
            decisions: strings(entry, "decisions"),
            files_touched: strings(entry, "filesTouched"),
            open_work: strings(entry, "openWork"),
        })
    })
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
/// The header ([`handoff_header`]: the newest valid compaction, else the newest
/// checkpoint) is never dropped. Below it, the conversation
/// tail is walked newest-first and kept verbatim, whole entries only, until
/// [`restoration_budget_bytes`] is spent; if even the newest entry alone does
/// not fit, its head is trimmed so the most recent words survive. The former
/// fixed twelve-line stop and 8 000-byte cut were the "new model forgot
/// everything" experience: a 40-turn conversation arrived as a paragraph.
///
/// One entry is allowed [`restoration_entry_bytes`] of that budget. Without
/// the allowance a single pasted file spent the whole tail, and the model
/// inherited the paste and none of the thirty turns that discussed it.
pub fn checkpoint_context_with_window(
    db: &Connection,
    session_id: &str,
    context_window_tokens: i64,
) -> Result<Option<String>, BridgeError> {
    Ok(restoration_handoff(db, session_id, context_window_tokens)?.map(|handoff| handoff.text))
}

/// The handoff a cold start would inject, with what it is made of: the header
/// and how many of the branch's conversation entries fit the budget. The
/// switch notice reads this so it can say what the new model actually gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorationHandoff {
    pub text: String,
    pub header: Option<HandoffHeader>,
    /// Conversation entries carried in the tail.
    pub carried_entries: usize,
    /// Conversation entries the projection held, carried or not.
    pub eligible_entries: usize,
    /// What Bridge wrote itself from the branch, with no model.
    pub facts: crate::handoff_facts::HandoffFacts,
}

pub fn restoration_handoff(
    db: &Connection,
    session_id: &str,
    context_window_tokens: i64,
) -> Result<Option<RestorationHandoff>, BridgeError> {
    let branch = SessionForest::new(db)
        .active_branch(session_id)
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let projection = ContextProjector::project(&branch, context_window_tokens.max(1))
        .map_err(|error| BridgeError::Invalid(error.to_string()))?;
    let budget = restoration_budget_bytes(context_window_tokens);
    let source = handoff_header(&branch, &projection);
    let mut header = Vec::new();
    if let Some(source) = &source {
        header.push(format!("{}: {}", source.label, source.summary));
        for decision in &source.decisions {
            header.push(format!("decision: {decision}"));
        }
        // What was unfinished at the boundary. Carried under the decisions so
        // the budget trim, which cuts the header's end, drops it before it
        // drops the summary or the earliest decisions.
        for open in &source.open_work {
            header.push(format!("still open: {open}"));
        }
    }
    // Reserve the header's room first, and bound the header itself: checkpoint
    // fields have no length limit, and the compiler's ceiling must hold even
    // for a pathological summary. Cutting the header's *end* keeps the summary
    // line and the earliest decisions — the front is what carries meaning.
    let header = bound_header(header, MAX_RESTORATION_BUDGET_BYTES);
    let header_bytes = header.iter().map(|line| line.len() + 1).sum::<usize>();
    // Bare checkpoints never render as conversation, so the header's own
    // checkpoint is not repeated below it.
    let entries = projection.render_entries.iter().collect::<Vec<_>>();
    let eligible_entries = entries.iter().filter(|entry| entry_text(entry).is_some()).count();
    // Bridge's own facts follow the model's summary, and never wait on one.
    // Their widest form (the first ask, and a pointer with the largest numbers
    // it can hold) is reserved before the tail is cut, so whatever the tail
    // turns out to carry, the final block fits the room it was given.
    let facts = crate::handoff_facts::collect(&branch);
    let reads_history = crate::history_tools::reads_own_history(db, session_id);
    let past_newest = branch.iter().map(|entry| entry.sequence).max().unwrap_or(0).saturating_add(1);
    let widest = facts.lines(
        false,
        reads_history.then(|| crate::handoff_facts::earlier_line(eligible_entries, past_newest)),
    );
    let facts_room = crate::handoff_facts::MAX_FACTS_BYTES.min(MAX_RESTORATION_BUDGET_BYTES.saturating_sub(header_bytes));
    let reserved = crate::handoff_facts::bound(widest, facts_room)
        .iter()
        .map(|line| line.len() + 1)
        .sum::<usize>();
    let tail = render_tail(
        &entries,
        budget.saturating_sub(header_bytes + reserved),
        restoration_entry_bytes(budget),
    );
    let first_ask_carried = facts
        .first_ask
        .as_ref()
        .is_some_and(|(sequence, _)| tail.oldest_sequence.is_some_and(|oldest| oldest <= *sequence));
    let older = eligible_entries.saturating_sub(tail.messages);
    let earlier = (reads_history && older > 0).then(|| {
        crate::handoff_facts::earlier_line(older, tail.oldest_sequence.unwrap_or(past_newest))
    });
    let facts_lines = crate::handoff_facts::bound(facts.lines(first_ask_carried, earlier), reserved);
    if header.is_empty() && facts_lines.is_empty() && tail.lines.is_empty() {
        return Ok(None);
    }
    let carried_entries = tail.messages;
    let mut lines = header;
    lines.extend(facts_lines);
    lines.extend(tail.lines);
    // By construction: header ≤ MAX, facts ≤ MAX − header, and
    // tail ≤ budget − header − facts.
    let context = lines.join("\n");
    Ok(Some(RestorationHandoff {
        text: format!(
            "Bridge checkpoint-restoration context (stored history, not native provider resume):\n{context}"
        ),
        header: source,
        carried_entries,
        eligible_entries,
        facts,
    }))
}

/// The text a handoff carries for one forest entry, if it carries any.
pub(crate) fn entry_text(entry: &crate::model::SessionEntry) -> Option<&str> {
    match entry.kind.as_str() {
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
    }
    .map(str::trim)
    .filter(|value| !value.is_empty())
}

/// What [`render_tail`] carried.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tail {
    /// The lines, oldest first.
    pub lines: Vec<String>,
    /// How many of the lines are conversation entries (not `tools:` lines).
    pub messages: usize,
    /// The sequence of the oldest conversation entry carried.
    pub oldest_sequence: Option<i64>,
}

/// One thing a tail can carry: a conversation entry, or the facts of the tool
/// calls that ran between two of them.
enum TailItem<'a> {
    Message(&'a crate::model::SessionEntry, &'a str),
    Tools(Vec<crate::tool_facts::ToolFact>),
}

/// `entries` (oldest first) as tail items: each conversation entry on its own,
/// and each run of completed tool calls between two of them as one item.
fn tail_items<'a>(entries: &[&'a crate::model::SessionEntry]) -> Vec<TailItem<'a>> {
    let started = crate::tool_facts::StartedIndex::new(entries.iter().copied());
    let mut items = Vec::new();
    let mut run = Vec::new();
    for entry in entries {
        if let Some(text) = entry_text(entry) {
            if !run.is_empty() {
                items.push(TailItem::Tools(std::mem::take(&mut run)));
            }
            items.push(TailItem::Message(entry, text));
        } else if let Some(fact) = crate::tool_facts::fact(entry, &started) {
            run.push(fact);
        }
    }
    if !run.is_empty() {
        items.push(TailItem::Tools(run));
    }
    items
}

/// Labelled lines for `entries` (oldest first), kept newest-first until
/// `budget` bytes are spent, each clipped to `entry_allowance`, and returned
/// oldest first. If even the newest entry does not fit, its most recent bytes
/// are kept rather than nothing.
///
/// The tool calls between two conversation entries ride as one `tools:` line
/// ([`crate::tool_facts::line`]): what ran and what it touched, never its
/// output. They spend the same budget, so a tool-heavy turn costs a line.
pub(crate) fn render_tail(
    entries: &[&crate::model::SessionEntry],
    budget: usize,
    entry_allowance: usize,
) -> Tail {
    let mut remaining = budget;
    let mut tail = Tail::default();
    for item in tail_items(entries).iter().rev() {
        let (entry, value) = match item {
            TailItem::Message(entry, value) => (*entry, *value),
            TailItem::Tools(facts) => {
                let Some(line) = crate::tool_facts::line(facts) else {
                    continue;
                };
                let cost = line.len() + 1;
                if cost > remaining {
                    break;
                }
                remaining -= cost;
                tail.lines.push(line);
                continue;
            }
        };
        let clipped = clip_middle(value, entry_allowance);
        let value = clipped.as_str();
        let line = format!("{}: {value}", entry.kind);
        let cost = line.len() + 1;
        if cost <= remaining {
            remaining -= cost;
            tail.lines.push(line);
            tail.messages += 1;
            tail.oldest_sequence = Some(entry.sequence);
            continue;
        }
        let prefix = format!("{}: …", entry.kind);
        if tail.lines.is_empty() && remaining > prefix.len() + 1 {
            let kept = keep_tail(value, remaining - prefix.len() - 1);
            tail.lines.push(format!("{prefix}{kept}"));
            tail.messages += 1;
            tail.oldest_sequence = Some(entry.sequence);
        }
        break;
    }
    tail.lines.reverse();
    tail
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

/// `text` when it fits in `max_bytes`, else its opening and closing halves
/// around a marker naming what was cut. Both cuts land on char boundaries.
fn clip_middle(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let half = max_bytes / 2;
    let mut head_end = half.min(text.len());
    while !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let tail = keep_tail(text, half);
    let omitted = text.len() - head_end - tail.len();
    format!("{}\n… {omitted} bytes omitted …\n{tail}", &text[..head_end])
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
pub(crate) mod tests {
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
    fn a_switch_checkpoint_heads_the_handoff_without_cutting_older_history() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest
            .append("s", EntryKind::UserMessage, serde_json::json!({"text":"check whether the masked MCP leaks prod data"}))
            .unwrap();
        forest
            .append("s", EntryKind::AssistantMessage, serde_json::json!({"text":"the experiment is set up on staging"}))
            .unwrap();
        forest
            .append(
                "s",
                EntryKind::Checkpoint,
                serde_json::json!({
                    "schemaVersion":1,
                    "summary":"This session only attempted to inspect Safari",
                    "decisions":["Use staging only"],
                    "filesTouched":["scripts/probe.sh"],
                    "openWork":["Send the query to Yamada-san"],
                    "reason":"before_downgrade",
                    "landing":"switch"
                }),
            )
            .unwrap();
        let handoff = restoration_handoff(&db, "s", 128_000).unwrap().unwrap();
        let text = &handoff.text;
        assert!(text.contains("checkpoint: This session only attempted to inspect Safari"), "{text}");
        assert!(text.contains("decision: Use staging only"), "{text}");
        assert!(text.contains("still open: Send the query to Yamada-san"), "{text}");
        assert!(text.contains("check whether the masked MCP leaks prod data"), "older history survives: {text}");
        assert!(text.contains("the experiment is set up on staging"), "{text}");
        assert_eq!(text.matches("only attempted to inspect Safari").count(), 1, "said once: {text}");
        let header = handoff.header.unwrap();
        assert_eq!(header.label, "checkpoint");
        assert_eq!(header.files_touched, vec!["scripts/probe.sh".to_owned()]);
        assert_eq!((handoff.carried_entries, handoff.eligible_entries), (2, 2));
    }

    /// A tool entry as an adapter writes it: through the normalized-event
    /// envelope, not `forest.append`.
    pub(crate) fn tool_event(db: &Connection, session: &str, kind: &str, item: &str, title: Option<&str>, text: Option<&str>, data: serde_json::Value) {
        let mut event = crate::agent::NormalizedEvent::new(kind);
        event.item_id = Some(item.into());
        event.title = title.map(str::to_owned);
        event.text = text.map(str::to_owned);
        event.status = Some(if kind.ends_with(".started") { "inProgress" } else { "completed" }.into());
        event.data = data;
        store::session_event(db, session, &event, &serde_json::json!({})).unwrap();
    }

    #[test]
    fn tool_lines_ride_in_the_tail_and_messages_are_still_what_is_counted() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"run the tests and open the PR"})).unwrap();
        tool_event(&db, "s", "command.started", "exec-1", Some("/bin/zsh -lc 'cargo test'"), None, serde_json::json!({"command":"/bin/zsh -lc 'cargo test'"}));
        tool_event(&db, "s", "command.output_delta", "exec-1", None, Some("RAW TEST OUTPUT"), serde_json::json!({}));
        tool_event(&db, "s", "command.completed", "exec-1", None, Some("RAW TEST OUTPUT"), serde_json::json!({"command":"/bin/zsh -lc 'cargo test'","exitCode":101,"aggregatedOutput":"RAW TEST OUTPUT"}));
        tool_event(&db, "s", "file_change.completed", "exec-2", None, None, serde_json::json!({"type":"fileChange","changes":[{"path":"src/retry.rs","diff":"RAW DIFF"}]}));
        forest.append("s", EntryKind::AssistantMessage, serde_json::json!({"text":"one test fails; I fixed retry.rs"})).unwrap();
        tool_event(&db, "s", "command.completed", "exec-3", None, None, serde_json::json!({"command":"gh pr create --fill","exitCode":0}));

        let handoff = restoration_handoff(&db, "s", 200_000).unwrap().unwrap();
        let text = &handoff.text;
        let lines = text.lines().collect::<Vec<_>>();
        let at = |needle: &str| lines.iter().position(|line| line.contains(needle)).unwrap_or_else(|| panic!("no {needle} in {text}"));
        assert!(at("user.message: run the tests") < at("tools: ran `cargo test` (exit 101); edited src/retry.rs"), "{text}");
        assert!(at("tools: ran `cargo test`") < at("assistant.message: one test fails"), "{text}");
        assert!(at("assistant.message: one test fails") < at("tools: ran `gh pr create --fill` (exit 0)"), "{text}");
        assert!(!text.contains("RAW"), "no output reaches the handoff: {text}");
        assert_eq!((handoff.carried_entries, handoff.eligible_entries), (2, 2), "tool lines are not messages");
    }

    #[test]
    fn a_tool_line_that_does_not_fit_stops_the_tail_like_a_message() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"older ask"})).unwrap();
        for index in 0..6 {
            tool_event(&db, "s", "command.completed", &format!("e{index}"), None, None, serde_json::json!({"command": format!("echo {}", "y".repeat(70)), "exitCode": 0}));
        }
        forest.append("s", EntryKind::AssistantMessage, serde_json::json!({"text":"newest reply"})).unwrap();
        let entries = SessionForest::new(&db).active_branch("s").unwrap();
        let refs = entries.iter().collect::<Vec<_>>();
        let newest_only = "assistant.message: newest reply".len() + 1;
        let tail = render_tail(&refs, newest_only + 10, 2_000);
        assert_eq!(tail.lines, vec!["assistant.message: newest reply".to_owned()]);
        assert_eq!((tail.messages, tail.oldest_sequence), (1, Some(entries.last().unwrap().sequence)));
        let all = render_tail(&refs, 10_000, 2_000);
        assert_eq!(all.lines.len(), 3, "{:?}", all.lines);
        assert_eq!(all.messages, 2);
        assert_eq!(all.oldest_sequence, Some(entries[0].sequence));
    }

    #[test]
    fn the_facts_header_exists_without_any_summary() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"ship the importer"})).unwrap();
        tool_event(&db, "s", "file_change.completed", "x", None, None, serde_json::json!({"changes":[{"path":"src/import.rs"}]}));
        tool_event(&db, "s", "command.completed", "y", None, Some("https://github.com/acme/app/pull/41"), serde_json::json!({"command":"gh pr create","exitCode":0}));
        forest.append("s", EntryKind::AssistantMessage, serde_json::json!({"text":"opened the PR"})).unwrap();
        let handoff = restoration_handoff(&db, "s", 200_000).unwrap().unwrap();
        assert!(handoff.header.is_none(), "no model summary");
        let text = &handoff.text;
        for line in ["goal: ship the importer", "files touched: src/import.rs", "links: https://github.com/acme/app/pull/41"] {
            assert!(text.lines().any(|candidate| candidate == line), "{line} missing: {text}");
        }
        assert!(!text.contains("earlier:"), "everything fit: {text}");
        assert!(text.find("links:").unwrap() < text.find("user.message: ship").unwrap(), "facts head the tail: {text}");
        assert_eq!((handoff.facts.file_count(), handoff.facts.links.len()), (1, 1));
    }

    #[test]
    fn a_model_summary_comes_first_and_the_facts_follow() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"audit the token store"})).unwrap();
        forest.append("s", EntryKind::Checkpoint, serde_json::json!({
            "schemaVersion":1,"summary":"audited the store","decisions":["keep SQLite"],
            "filesTouched":["src/store.rs"],"openWork":["rotate keys"],"reason":"before_downgrade"
        })).unwrap();
        let text = restoration_handoff(&db, "s", 200_000).unwrap().unwrap().text;
        let order = ["checkpoint: audited the store", "decision: keep SQLite", "still open: rotate keys", "goal: audit the token store", "files touched: src/store.rs", "user.message: audit the token store"]
            .iter()
            .map(|needle| text.find(needle).unwrap_or_else(|| panic!("{needle} missing: {text}")))
            .collect::<Vec<_>>();
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}: {text}");
    }

    #[test]
    fn started_with_appears_only_when_the_tail_lost_the_first_ask() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"FIRST ASK: build the Conductor importer"})).unwrap();
        for index in 0..30 {
            forest.append("s", EntryKind::AssistantMessage, serde_json::json!({"text": format!("{index} {}", "z".repeat(900))})).unwrap();
        }
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"now add tests"})).unwrap();
        let narrow = restoration_handoff(&db, "s", 1_000).unwrap().unwrap();
        assert!(narrow.text.contains("started with: FIRST ASK: build the Conductor importer"), "{}", narrow.text);
        assert!(!narrow.text.contains("user.message: FIRST ASK"), "the tail lost it");
        let wide = restoration_handoff(&db, "s", 1_000_000).unwrap().unwrap();
        assert!(wide.text.contains("user.message: FIRST ASK"));
        assert!(!wide.text.contains("started with:"), "carried in the tail, so not said twice");
        let envelope = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n".len();
        assert!(narrow.text.len() - envelope <= MIN_RESTORATION_BUDGET_BYTES, "{}", narrow.text.len());
    }

    #[test]
    fn the_handoff_names_chats_history_and_the_oldest_carried_sequence() {
        let db = database();
        let forest = SessionForest::new(&db);
        for index in 0..40 {
            forest.append("s", EntryKind::UserMessage, serde_json::json!({"text": format!("{index}: {}", "x".repeat(900))})).unwrap();
        }
        let handoff = restoration_handoff(&db, "s", 1_000).unwrap().unwrap();
        let entries = SessionForest::new(&db).active_branch("s").unwrap();
        let oldest_carried = entries
            .iter()
            .find(|entry| handoff.text.contains(&format!("\nuser.message: {}: ", entry.payload["text"].as_str().unwrap().split(':').next().unwrap())))
            .unwrap()
            .sequence;
        let older = handoff.eligible_entries - handoff.carried_entries;
        let expected = format!(
            "earlier: {older} older messages are not in this handoff; call chats_history with before={oldest_carried} to read them, tool output included."
        );
        assert!(handoff.text.lines().any(|line| line == expected), "{expected}\n{}", handoff.text);
        let envelope = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n".len();
        assert!(handoff.text.len() - envelope <= MIN_RESTORATION_BUDGET_BYTES);
    }

    #[test]
    fn a_worker_handoff_never_names_chats_history() {
        let db = database();
        db.execute("INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,parent_session_id) VALUES('child','w','codex','Worker','stopped','reported','s')", []).unwrap();
        let forest = SessionForest::new(&db);
        for index in 0..40 {
            forest.append("child", EntryKind::UserMessage, serde_json::json!({"text": format!("{index}: {}", "x".repeat(900))})).unwrap();
        }
        let handoff = restoration_handoff(&db, "child", 1_000).unwrap().unwrap();
        assert!(handoff.carried_entries < handoff.eligible_entries);
        assert!(!handoff.text.contains("chats_history"), "{}", handoff.text);
    }

    #[test]
    fn the_facts_header_is_deterministic() {
        let db = database();
        let forest = SessionForest::new(&db);
        forest.append("s", EntryKind::UserMessage, serde_json::json!({"text":"see https://github.com/a/b/issues/1 and https://github.com/a/b/pull/2"})).unwrap();
        for index in 0..5 {
            tool_event(&db, "s", "file_change.completed", &format!("f{index}"), None, None, serde_json::json!({"changes":[{"path": format!("src/{index}.rs")}]}));
        }
        let first = restoration_handoff(&db, "s", 64_000).unwrap().unwrap().text;
        for _ in 0..5 {
            assert_eq!(restoration_handoff(&db, "s", 64_000).unwrap().unwrap().text, first);
        }
    }

    #[test]
    fn the_handoff_counts_what_fit_against_what_was_stored() {
        let db = database();
        let forest = SessionForest::new(&db);
        for index in 0..40 {
            forest
                .append("s", EntryKind::UserMessage, serde_json::json!({"text": format!("{index}: {}", "x".repeat(900))}))
                .unwrap();
        }
        let handoff = restoration_handoff(&db, "s", 1_000).unwrap().unwrap();
        assert_eq!(handoff.eligible_entries, 40);
        assert!(handoff.carried_entries > 0 && handoff.carried_entries < 40, "{}", handoff.carried_entries);
        assert!(handoff.text.contains("39: "), "the newest survive");
        assert!(!handoff.text.contains("\nuser.message: 0: "), "the oldest do not");
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
    fn one_pasted_file_cannot_spend_the_whole_tail() {
        let db = database();
        db.execute("UPDATE sessions SET harness='claude', model='claude-opus-4-6' WHERE id='s'", [])
            .unwrap();
        let forest = SessionForest::new(&db);
        forest
            .append("s", EntryKind::UserMessage, serde_json::json!({"text":"turn 00 user: before the paste"}))
            .unwrap();
        let paste = format!(
            "turn 01 user: here is the whole file\n{}\nEND OF FILE",
            "fn noise() { let x = 1; }\n".repeat(4_000)
        );
        assert!(paste.len() > MAX_RESTORATION_BUDGET_BYTES, "the paste alone exceeds the budget");
        forest
            .append("s", EntryKind::UserMessage, serde_json::json!({"text": paste}))
            .unwrap();
        for turn in 2..22 {
            let (kind, speaker) = if turn % 2 == 0 {
                (EntryKind::UserMessage, "user")
            } else {
                (EntryKind::AssistantMessage, "assistant")
            };
            forest
                .append(
                    "s",
                    kind,
                    serde_json::json!({"text": format!("turn {turn:02} {speaker}: we keep the retry in src/billing/retry.ts")}),
                )
                .unwrap();
        }
        let context = checkpoint_context(&db, "s").unwrap().unwrap();
        for turn in 2..22 {
            assert!(
                context.contains(&format!("turn {turn:02} ")),
                "turn {turn} must survive the paste verbatim"
            );
        }
        assert!(context.contains("turn 00 user: before the paste"), "the turn before the paste survives too");
        assert!(context.contains("turn 01 user: here is the whole file"), "the paste's opening survives");
        assert!(context.contains("END OF FILE"), "the paste's closing survives");
        assert!(context.contains("bytes omitted"), "the cut is marked, not silent");
        let budget = restoration_budget_bytes(200_000);
        let paste_line = context
            .lines()
            .skip_while(|line| !line.starts_with("user.message: turn 01"))
            .take_while(|line| !line.starts_with("user.message: turn 02"))
            .map(|line| line.len() + 1)
            .sum::<usize>();
        assert!(
            paste_line <= restoration_entry_bytes(budget) + 64,
            "the paste is held to its allowance: {paste_line}"
        );
        let envelope = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n".len();
        assert!(context.len() - envelope <= budget);
    }

    #[test]
    fn the_entry_allowance_scales_with_the_budget_and_keeps_a_floor() {
        assert_eq!(restoration_entry_bytes(MAX_RESTORATION_BUDGET_BYTES), MAX_RESTORATION_BUDGET_BYTES / 6);
        assert_eq!(restoration_entry_bytes(MIN_RESTORATION_BUDGET_BYTES), MIN_RESTORATION_ENTRY_BYTES);
        assert_eq!(restoration_entry_bytes(0), MIN_RESTORATION_ENTRY_BYTES);
        assert_eq!(clip_middle("short", 100), "short");
        let clipped = clip_middle(&"ünïcödé ".repeat(1_000), 200);
        assert!(clipped.starts_with("ünïcödé "));
        assert!(clipped.ends_with("ünïcödé "));
        assert!(clipped.contains("bytes omitted"));
        assert!(clipped.len() < 300);
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
