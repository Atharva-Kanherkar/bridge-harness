//! What a new chat on a trail starts with.
//!
//! The brief rides the path the `$harness` shortcut already uses: each chosen
//! item becomes a `handoff.brief` entry in the new chat's forest before its
//! first start, and the cold start projects the active branch into labelled
//! restoration context for whichever harness runs the chat. Nothing here
//! touches the session's harness, model, access, worktree or scope.
//!
//! Restoration clips one entry to a share of its budget and drops the oldest
//! entries once the budget is spent, so a carry splits a long item into parts
//! that each fit, and refuses a selection that would not fit at all.

use std::collections::HashSet;

use bridge_protocol::messages::{
    OutgoingState, PreviewTrailBriefResult, TrailBriefItem, TrailMemberKind, TrailMemberRef, TrailAddedBy,
};
use rusqlite::{params, Connection};
use serde_json::json;

use super::store::{self, TrailRow};
use super::timeline::{harness_name, pull_facts, session_facts, NoteSource};
use crate::github_surface::{CheckConclusion, CheckStatus, PullRequestState};
use crate::restoration;
use crate::session_forest::{append_in_transaction, EntryKind};
use crate::BridgeError;

/// The most of one note a brief carries.
pub const NOTE_MAX_BYTES: usize = 12_000;
/// The window a chat's history is projected at: about 12 KB, 3k tokens.
pub const CHAT_WINDOW_TOKENS: i64 = 24_000;
/// The share of the restoration budget a brief may take, leaving room for the
/// first message and the restoration header.
const BUDGET_PERCENT: usize = 90;

pub fn tokens(text: &str) -> u32 {
    text.len().div_ceil(4) as u32
}

/// The bytes a new chat on a window of `window_tokens` can carry.
pub fn budget_bytes(window_tokens: i64) -> usize {
    restoration::restoration_budget_bytes(window_tokens) * BUDGET_PERCENT / 100
}

pub struct BriefPart {
    pub item: TrailBriefItem,
    pub text: String,
}

fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… (cut at {max} bytes)", &text[..end])
}

/// Every item a carry could write, in carry order: notes, pull requests,
/// chats, messages. `exclude` is the chat being briefed.
pub fn parts(db: &Connection, notes: &dyn NoteSource, trail: &TrailRow, exclude: Option<&str>) -> Result<Vec<BriefPart>, BridgeError> {
    let members = store::members(db, &trail.id)?;
    let of = |kind: TrailMemberKind| members.iter().filter(move |member| member.kind == kind);
    let header = |what: &str| format!("From the trail “{}” · {what}", trail.title);
    let mut parts = Vec::new();
    let mut push = |id: String, label: String, text: String| {
        parts.push(BriefPart { item: TrailBriefItem { id, label, tokens: tokens(&text), default_on: true }, text });
    };

    for member in of(TrailMemberKind::Note) {
        let (Some(note), Some(markdown)) = (notes.note(&member.reference), notes.markdown(&member.reference)) else { continue };
        let title = note.title.filter(|title| !title.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
        let text = format!("{}\n\n{}", header(&format!("note “{title}”")), clip(markdown.trim(), NOTE_MAX_BYTES));
        push(format!("note:{}", member.reference), format!("Note “{title}”"), text);
    }
    for member in of(TrailMemberKind::PullRequest) {
        let Some(pull) = pull_facts(db, &member.reference)? else { continue };
        let mut lines = vec![header(&format!("pull request #{}", pull.number)), format!("#{} {}", pull.number, pull.title), pull.url.clone()];
        if !pull.head_branch.is_empty() {
            lines.push(format!("Branch: {}", pull.head_branch));
        }
        let mut label = format!("PR #{} {}", pull.number, pull.title);
        match &pull.view {
            Some(view) => {
                let state = match view.state {
                    PullRequestState::Merged => "merged",
                    PullRequestState::Closed => "closed",
                    PullRequestState::Open if view.is_draft => "open, draft",
                    PullRequestState::Open => "open",
                };
                lines.push(format!("State: {state}"));
                let checks = &view.checks;
                if checks.total > 0 {
                    lines.push(format!("Checks: {} of {} passing, {} failing, {} running", checks.passed, checks.total, checks.failed, checks.in_progress + checks.queued));
                    let failing: Vec<String> = view
                        .check_details
                        .iter()
                        .filter(|check| {
                            check.status == CheckStatus::Completed
                                && matches!(
                                    check.conclusion,
                                    Some(CheckConclusion::Failure | CheckConclusion::TimedOut | CheckConclusion::StartupFailure | CheckConclusion::ActionRequired)
                                )
                        })
                        .map(|check| if check.log_url.is_empty() { check.name.clone() } else { format!("{} ({})", check.name, check.log_url) })
                        .collect();
                    if !failing.is_empty() {
                        lines.push(format!("Failing: {}", failing.join(", ")));
                        label.push_str(&format!(": {} failing", failing.len()));
                    }
                }
                if let Some(fetched) = &pull.fetched_at {
                    lines.push(format!("As of {fetched}"));
                }
            }
            None => lines.push("State: not read from GitHub yet".into()),
        }
        push(format!("pr:{}", member.reference), label, lines.join("\n"));
    }
    for member in of(TrailMemberKind::Session) {
        if exclude == Some(member.reference.as_str()) {
            continue;
        }
        let Some(chat) = session_facts(db, &member.reference)? else { continue };
        // One chat whose history cannot be projected costs its own item, not
        // the whole brief.
        let Ok(Some(history)) = restoration::checkpoint_context_with_window(db, &chat.id, CHAT_WINDOW_TOKENS) else { continue };
        let who = harness_name(&chat.harness);
        let text = format!("{}\n\n{history}", header(&format!("recent history of the {who} chat “{}”", chat.title)));
        push(format!("chat:{}", member.reference), format!("Chat “{}” ({who})", chat.title), text);
    }
    for member in of(TrailMemberKind::Outgoing) {
        let Some(message) = crate::outgoing::get(db, &member.reference)? else { continue };
        if message.state == OutgoingState::Discarded {
            continue;
        }
        let state = if message.state == OutgoingState::Draft { "drafted, not sent yet" } else { "copied to send" };
        let text = format!("{}\n\n{}", header(&format!("message to {} ({state})", message.label)), message.body.trim());
        push(format!("message:{}", member.reference), format!("Message to {}", message.label), text);
    }
    Ok(parts)
}

pub fn preview(db: &Connection, notes: &dyn NoteSource, trail: &TrailRow) -> Result<PreviewTrailBriefResult, BridgeError> {
    let items: Vec<TrailBriefItem> = parts(db, notes, trail, None)?.into_iter().map(|part| part.item).collect();
    let total_tokens = items.iter().filter(|item| item.default_on).map(|item| item.tokens).sum();
    let budget_tokens = (budget_bytes(crate::model_catalog::DEFAULT_CONTEXT_WINDOW_TOKENS) / 4) as u32;
    Ok(PreviewTrailBriefResult { items, total_tokens, budget_tokens })
}

/// `text` in pieces of at most `max` bytes, cut at line ends where it can.
pub fn split(text: &str, max: usize) -> Vec<String> {
    let max = max.max(64);
    let mut pieces = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        let mut line = line;
        while current.len() + line.len() > max {
            if !current.is_empty() {
                pieces.push(std::mem::take(&mut current));
                continue;
            }
            let mut end = max;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            pieces.push(line[..end].to_owned());
            line = &line[end..];
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

/// Write the chosen items into a new chat and put the chat on the trail.
/// Returns the number of forest entries written.
pub fn carry(
    db: &Connection,
    notes: &dyn NoteSource,
    trail_id: &str,
    session_id: &str,
    items: &[String],
    now: &str,
) -> Result<u32, BridgeError> {
    let trail = store::require(db, trail_id)?;
    if session_facts(db, session_id)?.is_none() {
        return Err(BridgeError::Invalid("That chat no longer exists.".into()));
    }
    let started: i64 = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_entries WHERE session_id=?1 AND kind='user.message')",
        params![session_id],
        |row| row.get(0),
    )?;
    if started == 1 {
        return Err(BridgeError::Invalid("A trail brief can only start a new chat, before its first message.".into()));
    }
    let available = parts(db, notes, &trail, Some(session_id))?;
    let known: HashSet<&str> = available.iter().map(|part| part.item.id.as_str()).collect();
    if let Some(unknown) = items.iter().find(|item| !known.contains(item.as_str())) {
        return Err(BridgeError::Invalid(format!("The trail has no brief item “{unknown}”. Reload the trail and try again.")));
    }
    let chosen: HashSet<&str> = items.iter().map(String::as_str).collect();
    let selected: Vec<&BriefPart> = available.iter().filter(|part| chosen.contains(part.item.id.as_str())).collect();

    let window = restoration::session_context_window_tokens(db, session_id)?;
    let budget = budget_bytes(window);
    let wanted: usize = selected.iter().map(|part| part.text.len()).sum();
    if wanted > budget {
        return Err(BridgeError::Invalid(format!(
            "These items need about {} tokens; this chat can start with at most {}. Untick some and try again.",
            wanted.div_ceil(4),
            budget / 4
        )));
    }
    let member = TrailMemberRef { kind: TrailMemberKind::Session, reference: session_id.to_owned() };
    if let Some(owner) = store::trail_of(db, &member)?.filter(|owner| owner != &trail.id) {
        let title = store::get(db, &owner)?.map(|other| other.title).unwrap_or_default();
        return Err(BridgeError::Invalid(format!("This is already on the trail “{title}”. Remove it there first.")));
    }
    let allowance = restoration::restoration_entry_bytes(restoration::restoration_budget_bytes(window));
    // Every refusal above wrote nothing; from here the entries, the
    // membership and the audit row land together or not at all.
    let transaction = db.unchecked_transaction()?;
    let mut written = 0u32;
    for part in selected {
        // Restoration prefixes each entry's kind; keep that room inside the allowance.
        let pieces = split(&part.text, allowance.saturating_sub(64));
        let count = pieces.len();
        for (index, piece) in pieces.into_iter().enumerate() {
            let text = if count > 1 { format!("(part {} of {count}) {piece}", index + 1) } else { piece };
            append_in_transaction(&transaction, session_id, EntryKind::HandoffBrief, json!({ "text": text, "trailId": trail.id, "briefItem": part.item.id }))
                .map_err(|error| BridgeError::Invalid(error.to_string()))?;
            written += 1;
        }
    }
    store::add_member(&transaction, &trail.id, &member, TrailAddedBy::Person, now)?;
    crate::store::event(&transaction, "chat", "trail.brief_carried", session_id, &format!("Started on trail {} with {written} brief entries", trail.id))?;
    transaction.commit()?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trails::store::tests::{database, session};
    use crate::trails::timeline::tests::{add_session, attach_pr, view, FakeNotes};
    use bridge_protocol::messages::OutgoingDestination;
    use crate::session_forest::SessionForest;

    const URL: &str = "https://github.com/o/r/pull/12";

    fn talk(db: &Connection, session: &str, user: &str, assistant: &str) {
        let forest = SessionForest::new(db);
        forest.append(session, EntryKind::UserMessage, json!({"text": user})).unwrap();
        forest.append(session, EntryKind::AssistantMessage, json!({"text": assistant})).unwrap();
    }

    fn fixture() -> (Connection, FakeNotes, TrailRow) {
        let db = database();
        add_session(&db, "s1", "codex", "Build it", Some("2026-11-10T00:00:00Z"));
        talk(&db, "s1", "Where does the undo toast live?", "We decided undo lasts 8 seconds, in ArchiveToast.tsx.");
        attach_pr(&db, "s1", URL, "2026-11-10T01:00:00Z", Some(&view(PullRequestState::Open, vec![("vitest", Some(CheckConclusion::Failure))])));
        crate::outgoing::create_draft(&db, "s1", OutgoingDestination::SlackDm { user: "@mika".into() }, "Please review #12", "review", &[], "t").unwrap();
        let notes = FakeNotes::default().with("n1", "Plan", "t", &[], "# Plan\n\n- Archiving keeps the worktree.");
        let trail = store::create(&db, "Archive a chat", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t0").unwrap();
        store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Note, reference: "n1".into() }, TrailAddedBy::Person, "t0").unwrap();
        super::super::auto::sweep(&db, &notes, "t1").unwrap();
        (db, notes, trail)
    }

    #[test]
    fn preview_orders_items_and_counts_tokens() {
        let (db, notes, trail) = fixture();
        let preview = preview(&db, &notes, &trail).unwrap();
        let ids: Vec<&str> = preview.items.iter().map(|item| item.id.split(':').next().unwrap()).collect();
        assert_eq!(ids, vec!["note", "pr", "chat", "message"]);
        assert_eq!(preview.items[1].label, "PR #12 Archive a chat: 1 failing");
        assert_eq!(preview.items[2].label, "Chat “Build it” (Codex)");
        let texts = parts(&db, &notes, &trail, None).unwrap();
        for part in &texts {
            assert_eq!(part.item.tokens, part.text.len().div_ceil(4) as u32);
        }
        assert!(texts[1].text.contains("Failing: vitest (https://ci/vitest)"));
        assert_eq!(preview.total_tokens, preview.items.iter().map(|item| item.tokens).sum::<u32>());
        assert_eq!(preview.budget_tokens as usize, budget_bytes(crate::model_catalog::DEFAULT_CONTEXT_WINDOW_TOKENS) / 4);
        // A chat whose history cannot be projected offers nothing, and the
        // rest of the brief still does.
        db.execute_batch("PRAGMA foreign_keys=OFF; UPDATE session_heads SET active_entry_id='missing' WHERE session_id='s1'; PRAGMA foreign_keys=ON;").unwrap();
        assert!(restoration::checkpoint_context_with_window(&db, "s1", CHAT_WINDOW_TOKENS).is_err(), "the head now points nowhere");
        let degraded = super::preview(&db, &notes, &trail).unwrap();
        assert!(degraded.items.iter().all(|item| !item.id.starts_with("chat:")), "{:?}", degraded.items);
        assert_eq!(degraded.items.len(), 3);
        // A discarded draft offers nothing.
        db.execute("UPDATE outgoing_messages SET state='discarded'", []).unwrap();
        assert_eq!(super::preview(&db, &notes, &trail).unwrap().items.len(), 2);
    }

    #[test]
    fn carry_writes_entries_restoration_reads() {
        let (db, notes, trail) = fixture();
        add_session(&db, "new", "claude", "Fix the check", None);
        let items: Vec<String> = parts(&db, &notes, &trail, Some("new")).unwrap().into_iter().map(|part| part.item.id).filter(|id| !id.starts_with("message:")).collect();
        let written = carry(&db, &notes, &trail.id, "new", &items, "t2").unwrap();
        assert_eq!(written, 3);
        let context = restoration::checkpoint_context(&db, "new").unwrap().unwrap();
        assert!(context.contains("Archiving keeps the worktree"), "{context}");
        assert!(context.contains("Failing: vitest"), "{context}");
        assert!(context.contains("undo lasts 8 seconds"), "{context}");
        assert!(!context.contains("Please review #12"), "an unchosen item stays out");
        assert_eq!(store::trail_of(&db, &session("new")).unwrap().as_deref(), Some(trail.id.as_str()));
        // Zero items still joins the trail.
        add_session(&db, "empty", "codex", "Nothing", None);
        assert_eq!(carry(&db, &notes, &trail.id, "empty", &[], "t3").unwrap(), 0);
        assert!(store::trail_of(&db, &session("empty")).unwrap().is_some());
    }

    #[test]
    fn carry_refuses_a_started_chat_and_unknown_items() {
        let (db, notes, trail) = fixture();
        let refused = carry(&db, &notes, &trail.id, "s1", &[], "t2").unwrap_err();
        assert!(refused.to_string().contains("before its first message"), "{refused}");
        add_session(&db, "new", "claude", "New", None);
        let refused = carry(&db, &notes, &trail.id, "new", &["note:nope".into()], "t2").unwrap_err();
        assert!(refused.to_string().contains("note:nope"), "{refused}");
        assert!(carry(&db, &notes, &trail.id, "missing", &[], "t2").is_err());
        assert!(carry(&db, &notes, "no-trail", "new", &[], "t2").is_err());
        let entries: i64 = db.query_row("SELECT COUNT(*) FROM session_entries WHERE session_id='new'", [], |row| row.get(0)).unwrap();
        assert_eq!(entries, 0, "a refused carry writes nothing");
    }

    #[test]
    fn a_carry_refused_for_another_trail_writes_nothing() {
        let (db, notes, trail) = fixture();
        add_session(&db, "new", "claude", "New", None);
        let other = store::create(&db, "Other", None, "t0").unwrap();
        store::add_member(&db, &other.id, &session("new"), TrailAddedBy::Person, "t0").unwrap();
        let items: Vec<String> = parts(&db, &notes, &trail, Some("new")).unwrap().into_iter().map(|part| part.item.id).collect();
        assert!(!items.is_empty());
        let refused = carry(&db, &notes, &trail.id, "new", &items, "t2").unwrap_err().to_string();
        assert!(refused.contains("“Other”"), "{refused}");
        let entries: i64 = db.query_row("SELECT COUNT(*) FROM session_entries WHERE session_id='new'", [], |row| row.get(0)).unwrap();
        assert_eq!(entries, 0, "a refused carry leaves the chat's history unchanged");
        assert!(restoration::checkpoint_context(&db, "new").unwrap().is_none());
        let audits: i64 = db.query_row("SELECT COUNT(*) FROM events WHERE kind='trail.brief_carried'", [], |row| row.get(0)).unwrap();
        assert_eq!(audits, 0);
    }

    #[test]
    fn a_carry_that_fails_after_writing_rolls_everything_back() {
        let (db, notes, trail) = fixture();
        add_session(&db, "new", "claude", "New", None);
        let items: Vec<String> = parts(&db, &notes, &trail, Some("new")).unwrap().into_iter().map(|part| part.item.id).collect();
        // The audit row is the last write; make it fail.
        db.execute_batch("ALTER TABLE events RENAME TO events_gone;").unwrap();
        assert!(carry(&db, &notes, &trail.id, "new", &items, "t2").is_err());
        let entries: i64 = db.query_row("SELECT COUNT(*) FROM session_entries WHERE session_id='new'", [], |row| row.get(0)).unwrap();
        assert_eq!(entries, 0, "the entries roll back with the failed audit");
        assert!(store::trail_of(&db, &session("new")).unwrap().is_none(), "so does the membership");
    }

    #[test]
    fn carry_refuses_over_budget_and_splits_long_items() {
        let db = database();
        let long = (0..1200).map(|line| format!("- decision {line}: keep the worktree until it is reclaimed")).collect::<Vec<_>>().join("\n");
        let mut notes = FakeNotes::default();
        for index in 0..6 {
            notes = notes.with(&format!("n{index}"), &format!("Plan {index}"), "t", &[], &long);
        }
        let trail = store::create(&db, "T", None, "t0").unwrap();
        for index in 0..6 {
            store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Note, reference: format!("n{index}") }, TrailAddedBy::Person, "t0").unwrap();
        }
        // No model: the default 128k window. Restoration budget 64 000 bytes,
        // one entry at most 10 666; a brief at most 57 600.
        add_session(&db, "new", "codex", "New", None);
        let window = restoration::session_context_window_tokens(&db, "new").unwrap();
        assert_eq!(window, crate::model_catalog::DEFAULT_CONTEXT_WINDOW_TOKENS);
        let allowance = restoration::restoration_entry_bytes(restoration::restoration_budget_bytes(window));
        let note_bytes = parts(&db, &notes, &trail, None).unwrap()[0].text.len();
        assert!(note_bytes > allowance && note_bytes < budget_bytes(window), "{note_bytes} {allowance}");

        let written = carry(&db, &notes, &trail.id, "new", &["note:n0".into()], "t1").unwrap();
        assert_eq!(written as usize, note_bytes.div_ceil(allowance - 64), "split into parts that fit");
        let context = restoration::checkpoint_context(&db, "new").unwrap().unwrap();
        assert!(context.contains("(part 1 of 2)") && context.contains("(part 2 of 2)"), "{context}");
        assert!(context.contains("decision 0:"), "the first line survives");
        assert!(context.contains("(cut at 12000 bytes)"), "the note's own clip survives, nothing else is cut");

        add_session(&db, "other", "codex", "Other", None);
        let all: Vec<String> = (0..6).map(|index| format!("note:n{index}")).collect();
        let refused = carry(&db, &notes, &trail.id, "other", &all, "t1").unwrap_err().to_string();
        assert!(refused.contains("Untick some") && refused.contains("at most 14400"), "{refused}");
        let entries: i64 = db.query_row("SELECT COUNT(*) FROM session_entries WHERE session_id='other'", [], |row| row.get(0)).unwrap();
        assert_eq!(entries, 0);
    }

    #[test]
    fn split_keeps_lines_and_bounds_pieces() {
        let text = "aaaa\nbbbb\ncccc\n";
        assert_eq!(split(text, 64), vec![text.to_owned()]);
        let long = "x".repeat(200);
        let pieces = split(&long, 64);
        assert!(pieces.iter().all(|piece| piece.len() <= 64));
        assert_eq!(pieces.concat(), long);
        let lines = (0..40).map(|n| format!("line {n}\n")).collect::<String>();
        let pieces = split(&lines, 70);
        assert!(pieces.iter().all(|piece| piece.len() <= 70 && piece.ends_with('\n')));
        assert_eq!(pieces.concat(), lines);
    }

    #[test]
    fn carry_changes_no_session_policy_fields() {
        let (db, notes, trail) = fixture();
        add_session(&db, "new", "claude", "New", None);
        let snapshot = |db: &Connection| -> String {
            db.query_row(
                "SELECT json_array(harness,model,kind,depth,status,workspace_id,parent_session_id,requested_tier,effort,account,purpose) FROM sessions WHERE id='new'",
                [],
                |row| row.get(0),
            )
            .unwrap()
        };
        let before = snapshot(&db);
        let items: Vec<String> = parts(&db, &notes, &trail, Some("new")).unwrap().into_iter().map(|part| part.item.id).collect();
        carry(&db, &notes, &trail.id, "new", &items, "t2").unwrap();
        assert_eq!(snapshot(&db), before);
        let access: i64 = db.query_row("SELECT COUNT(*) FROM session_entries WHERE session_id='new' AND kind<>'handoff.brief'", [], |row| row.get(0)).unwrap();
        assert_eq!(access, 0, "only brief entries were written");
    }
}
