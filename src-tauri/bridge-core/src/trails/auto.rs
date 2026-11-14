//! Automatic adds: records that join a trail because a hard link already ties
//! them to one of its chats. No model decides anything here.
//!
//! For each chat S on an open trail:
//! 1. a pull request attached to S,
//! 2. a message S drafted (not discarded),
//! 3. a notes page linked to S,
//! 4. a chat forked from S or spawned by it,
//!
//! joins the trail, unless it sits on another trail or the person took it off
//! this one. Rules repeat until nothing new joins, so a fork of a fork follows.

use std::collections::HashSet;

use bridge_protocol::messages::{TrailAddedBy, TrailMemberAdded, TrailMemberKind, TrailMemberRef};
use rusqlite::{params_from_iter, Connection};

use super::store;
use super::timeline::{pull_facts, session_facts, NoteFacts, NoteSource};
use crate::BridgeError;

/// One pass over every open trail. Returns what joined, once: a second sweep
/// with no new links returns nothing.
pub fn sweep(db: &Connection, notes: &dyn NoteSource, now: &str) -> Result<Vec<TrailMemberAdded>, BridgeError> {
    let trails = store::list(db, false)?;
    if trails.is_empty() {
        return Ok(Vec::new());
    }
    let all_notes = notes.all_notes();
    let mut added = Vec::new();
    for trail in trails {
        loop {
            let sessions: Vec<String> = store::members(db, &trail.id)?
                .into_iter()
                .filter(|member| member.kind == TrailMemberKind::Session)
                .map(|member| member.reference)
                .collect();
            if sessions.is_empty() {
                break;
            }
            let mut joined = 0;
            for candidate in candidates(db, &sessions, &all_notes)? {
                if store::trail_of(db, &candidate)?.is_some() || store::is_dismissed(db, &trail.id, &candidate)? {
                    continue;
                }
                if store::add_member(db, &trail.id, &candidate, TrailAddedBy::Auto, now)? {
                    joined += 1;
                    added.push(TrailMemberAdded {
                        trail_id: trail.id.clone(),
                        trail_title: trail.title.clone(),
                        label: label(db, notes, &candidate)?,
                        member: candidate,
                    });
                }
            }
            if joined == 0 {
                break;
            }
        }
    }
    Ok(added)
}

fn placeholders(count: usize) -> String {
    (1..=count).map(|index| format!("?{index}")).collect::<Vec<_>>().join(",")
}

fn strings(db: &Connection, sql: &str, sessions: &[String]) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(sql)?;
    let rows = statement.query_map(params_from_iter(sessions.iter()), |row| row.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Linked records in a stable order: PRs, drafts, notes, chats.
fn candidates(db: &Connection, sessions: &[String], notes: &[NoteFacts]) -> Result<Vec<TrailMemberRef>, BridgeError> {
    let marks = placeholders(sessions.len());
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |kind: TrailMemberKind, reference: String| {
        if seen.insert((kind, reference.clone())) {
            found.push(TrailMemberRef { kind, reference });
        }
    };
    for url in strings(
        db,
        &format!("SELECT url FROM session_pull_requests WHERE session_id IN ({marks}) GROUP BY url ORDER BY MIN(created_at), url"),
        sessions,
    )? {
        push(TrailMemberKind::PullRequest, url);
    }
    for id in strings(
        db,
        &format!("SELECT id FROM outgoing_messages WHERE session_id IN ({marks}) AND state<>'discarded' ORDER BY created_at, id"),
        sessions,
    )? {
        push(TrailMemberKind::Outgoing, id);
    }
    let members: HashSet<&str> = sessions.iter().map(String::as_str).collect();
    for note in notes {
        if note.session_ids.iter().any(|id| members.contains(id.as_str())) {
            push(TrailMemberKind::Note, note.id.clone());
        }
    }
    for id in strings(
        db,
        &format!(
            "SELECT id FROM sessions WHERE (parent_session_id IN ({marks}) OR fork_parent_session_id IN ({marks}))
             AND archived_at IS NULL ORDER BY started_at, id"
        ),
        sessions,
    )? {
        push(TrailMemberKind::Session, id);
    }
    Ok(found)
}

/// What a toast names: `PR #12 Fix login`, `Chat “Fix login”`, ...
pub fn label(db: &Connection, notes: &dyn NoteSource, member: &TrailMemberRef) -> Result<String, BridgeError> {
    Ok(match member.kind {
        TrailMemberKind::PullRequest => match pull_facts(db, &member.reference)? {
            Some(pull) => format!("PR #{} {}", pull.number, pull.title),
            None => "A pull request".into(),
        },
        TrailMemberKind::Session => match session_facts(db, &member.reference)? {
            Some(chat) => format!("Chat “{}”", chat.title),
            None => "A chat".into(),
        },
        TrailMemberKind::Note => match notes.note(&member.reference) {
            Some(note) => format!("Note “{}”", note.title.filter(|title| !title.trim().is_empty()).unwrap_or_else(|| "Untitled".into())),
            None => "A note".into(),
        },
        TrailMemberKind::Outgoing => match crate::outgoing::get(db, &member.reference)? {
            Some(message) => format!("Message to {}", message.label),
            None => "A message".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trails::store::tests::{database, session};
    use crate::trails::timeline::tests::{add_session, attach_pr, FakeNotes};
    use rusqlite::params;

    const URL: &str = "https://github.com/o/r/pull/12";

    fn refs(added: &[TrailMemberAdded]) -> Vec<(TrailMemberKind, String)> {
        added.iter().map(|added| (added.member.kind, added.member.reference.clone())).collect()
    }

    #[test]
    fn pr_draft_note_and_fork_of_a_member_join() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", Some("2026-11-10T00:00:00Z"));
        add_session(&db, "fork", "claude", "Try another way", Some("2026-11-11T00:00:00Z"));
        add_session(&db, "fork-of-fork", "claude", "Deeper", Some("2026-11-12T00:00:00Z"));
        db.execute("UPDATE sessions SET fork_parent_session_id='s1' WHERE id='fork'", []).unwrap();
        db.execute("UPDATE sessions SET parent_session_id='fork' WHERE id='fork-of-fork'", []).unwrap();
        attach_pr(&db, "s1", URL, "2026-11-10T01:00:00Z", None);
        let draft = crate::outgoing::create_draft(
            &db,
            "s1",
            bridge_protocol::messages::OutgoingDestination::SlackDm { user: "@mika".into() },
            "Please review",
            "review",
            &[],
            "2026-11-10T02:00:00Z",
        )
        .unwrap();
        let notes = FakeNotes::default().with("n1", "Plan", "2026-11-09T00:00:00Z", &["s1"], "plan");
        let trail = store::create(&db, "Archive", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t0").unwrap();

        let added = sweep(&db, &notes, "t1").unwrap();
        assert_eq!(
            refs(&added),
            vec![
                (TrailMemberKind::PullRequest, URL.to_string()),
                (TrailMemberKind::Outgoing, draft.id.clone()),
                (TrailMemberKind::Note, "n1".into()),
                (TrailMemberKind::Session, "fork".into()),
                (TrailMemberKind::Session, "fork-of-fork".into()),
            ]
        );
        assert_eq!(added[0].label, "PR #12 Archive a chat");
        assert_eq!(added[3].label, "Chat “Try another way”");
        assert!(added.iter().all(|added| added.trail_title == "Archive"));
        let members = store::members(&db, &trail.id).unwrap();
        assert_eq!(members.iter().filter(|member| member.added_by == TrailAddedBy::Auto).count(), 5);
    }

    #[test]
    fn non_member_links_add_nothing() {
        let db = database();
        add_session(&db, "member", "codex", "Build", None);
        add_session(&db, "other", "codex", "Unrelated", None);
        attach_pr(&db, "other", URL, "t", None);
        let notes = FakeNotes::default().with("n1", "Other plan", "t", &["other"], "");
        let trail = store::create(&db, "T", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &session("member"), TrailAddedBy::Person, "t0").unwrap();
        assert!(sweep(&db, &notes, "t1").unwrap().is_empty());
        // A trail with no chats has nothing to follow.
        store::create(&db, "Empty", None, "t0").unwrap();
        assert!(sweep(&db, &notes, "t1").unwrap().is_empty());
    }

    #[test]
    fn dismissed_and_claimed_records_are_skipped() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", None);
        add_session(&db, "s2", "codex", "Other", None);
        attach_pr(&db, "s1", URL, "t", None);
        attach_pr(&db, "s1", "https://github.com/o/r/pull/13", "t", None);
        let first = store::create(&db, "First", None, "t0").unwrap();
        let second = store::create(&db, "Second", None, "t0").unwrap();
        store::add_member(&db, &first.id, &session("s1"), TrailAddedBy::Person, "t0").unwrap();
        store::add_member(&db, &second.id, &session("s2"), TrailAddedBy::Person, "t0").unwrap();
        let claimed = TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: "https://github.com/o/r/pull/13".into() };
        store::add_member(&db, &second.id, &claimed, TrailAddedBy::Person, "t0").unwrap();
        let dismissed = TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: URL.into() };
        store::remove_member(&db, &first.id, &dismissed, "t0").unwrap();
        assert!(sweep(&db, &FakeNotes::default(), "t1").unwrap().is_empty());
        assert_eq!(store::trail_of(&db, &claimed).unwrap().as_deref(), Some(second.id.as_str()));
        // An archived trail is left alone.
        store::add_member(&db, &first.id, &dismissed, TrailAddedBy::Person, "t2").unwrap();
        store::remove_member(&db, &first.id, &dismissed, "t3").unwrap();
        db.execute("DELETE FROM trail_dismissals", params![]).unwrap();
        store::set_archived(&db, &first.id, true, "t4").unwrap();
        assert!(sweep(&db, &FakeNotes::default(), "t5").unwrap().is_empty());
    }

    #[test]
    fn a_second_sweep_reports_nothing() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", None);
        attach_pr(&db, "s1", URL, "t", None);
        let trail = store::create(&db, "T", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t0").unwrap();
        assert_eq!(sweep(&db, &FakeNotes::default(), "t1").unwrap().len(), 1);
        assert!(sweep(&db, &FakeNotes::default(), "t2").unwrap().is_empty());
    }
}
