//! What a person can put on a trail by hand: the chats, notes pages, pull
//! requests and drafted messages Bridge already keeps, newest first, never
//! one already on this trail. A record sits on at most one trail, so each
//! candidate names the trail it sits on, if any.

use bridge_protocol::messages::{Note, OutgoingState, TrailCandidate, TrailMemberKind, TrailMemberRef};
use rusqlite::{params, Connection};

use super::store;
use super::timeline::{actor, pull_facts};
use crate::github_surface::PullRequestState;
use crate::BridgeError;

/// The most candidates one call returns.
pub const LIMIT: usize = 50;

/// Candidates of `kind` for the trail. `notes` is the notes store's own
/// search for the query, read before the core database lock; only the `note`
/// kind uses it.
pub fn list(
    db: &Connection,
    trail_id: &str,
    kind: TrailMemberKind,
    query: Option<&str>,
    notes: &[Note],
) -> Result<Vec<TrailCandidate>, BridgeError> {
    store::require(db, trail_id)?;
    let query = query.map(str::trim).filter(|query| !query.is_empty()).map(str::to_lowercase);
    let matches = |text: &str| query.as_deref().is_none_or(|query| text.to_lowercase().contains(query));
    let found = match kind {
        TrailMemberKind::Session => chats(db)?,
        TrailMemberKind::Note => notes.iter().map(note).collect(),
        TrailMemberKind::PullRequest => pulls(db)?,
        TrailMemberKind::Outgoing => drafts(db)?,
        TrailMemberKind::Issue => return Err(BridgeError::Invalid("An issue is added by its link.".into())),
    };
    let mut kept = Vec::new();
    for mut candidate in found {
        // Notes come back already matched by the notes store's own search,
        // which reads bodies too.
        if kind != TrailMemberKind::Note && !matches(&candidate.title) {
            continue;
        }
        match store::trail_of(db, &candidate.member)? {
            Some(id) if id == trail_id => continue,
            Some(id) => {
                candidate.on_trail_title = store::get(db, &id)?.map(|trail| trail.title);
                candidate.on_trail_id = Some(id);
            }
            None => {}
        }
        if kind == TrailMemberKind::PullRequest {
            candidate.detail = pull_detail(db, &candidate.member.reference)?;
        }
        kept.push(candidate);
        if kept.len() == LIMIT {
            break;
        }
    }
    Ok(kept)
}

fn candidate(kind: TrailMemberKind, reference: String, title: String, at: String) -> TrailCandidate {
    TrailCandidate {
        member: TrailMemberRef { kind, reference },
        title,
        detail: None,
        harness: None,
        at,
        on_trail_id: None,
        on_trail_title: None,
    }
}

/// The person's own chats, not workers, internal passes or archived ones,
/// newest started first.
fn chats(db: &Connection) -> Result<Vec<TrailCandidate>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id, COALESCE(NULLIF(title,''), label), harness, COALESCE(started_at,'') FROM sessions
         WHERE archived_at IS NULL AND parent_session_id IS NULL AND (kind IS NULL OR kind IN ('direct','orchestrator'))
           AND NOT EXISTS(SELECT 1 FROM worker_runtime w WHERE w.session_id=sessions.id)
         ORDER BY started_at DESC, id",
    )?;
    let rows = statement.query_map([], |row| {
        let mut chat = candidate(TrailMemberKind::Session, row.get(0)?, row.get(1)?, row.get(3)?);
        chat.harness = Some(row.get(2)?);
        Ok(chat)
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn note(note: &Note) -> TrailCandidate {
    let title = note.title.clone().filter(|title| !title.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
    let mut page = candidate(TrailMemberKind::Note, note.id.clone(), title, note.updated_at.clone());
    page.detail = Some(format!("last edited by {}", actor(&note.last_actor)));
    page
}

/// Every pull request a chat attached, newest attached first.
fn pulls(db: &Connection) -> Result<Vec<TrailCandidate>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT url, MAX(number), (SELECT title FROM session_pull_requests t WHERE t.url=p.url ORDER BY fetched_at DESC, created_at DESC LIMIT 1), MAX(created_at) AS at
         FROM session_pull_requests p GROUP BY url ORDER BY at DESC, url",
    )?;
    let rows = statement.query_map([], |row| {
        let number: i64 = row.get(1)?;
        let title: String = row.get(2)?;
        Ok(candidate(TrailMemberKind::PullRequest, row.get(0)?, format!("#{number} {title}"), row.get(3)?))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

fn pull_detail(db: &Connection, url: &str) -> Result<Option<String>, BridgeError> {
    Ok(pull_facts(db, url)?.and_then(|pull| pull.view).map(|view| {
        match view.state {
            PullRequestState::Merged => "merged",
            PullRequestState::Closed => "closed",
            PullRequestState::Open if view.is_draft => "open · draft",
            PullRequestState::Open => "open",
        }
        .to_owned()
    }))
}

/// Messages chats drafted, not discarded ones, newest changed first.
fn drafts(db: &Connection) -> Result<Vec<TrailCandidate>, BridgeError> {
    let ids: Vec<String> = {
        let mut statement = db.prepare("SELECT id FROM outgoing_messages WHERE state<>'discarded' ORDER BY updated_at DESC, id")?;
        let rows = statement.query_map(params![], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut found = Vec::new();
    for id in ids {
        let Some(message) = crate::outgoing::get(db, &id)? else { continue };
        let mut draft = candidate(TrailMemberKind::Outgoing, id, format!("Message to {}", message.label), message.updated_at.clone());
        draft.detail = Some(
            match message.state {
                OutgoingState::Draft => "draft, not sent yet",
                OutgoingState::Copied => "copied to send",
                OutgoingState::Discarded => "discarded",
                OutgoingState::Sending => "sending",
                OutgoingState::Sent => "sent",
                OutgoingState::Failed => "send failed",
            }
            .to_owned(),
        );
        found.push(draft);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trails::store::tests::{database, session};
    use crate::trails::timeline::tests::{add_session, attach_pr};
    use bridge_protocol::messages::TrailAddedBy;

    fn refs(found: &[TrailCandidate]) -> Vec<String> {
        found.iter().map(|candidate| candidate.member.reference.clone()).collect()
    }

    #[test]
    fn chats_are_the_persons_own_newest_first_and_skip_this_trail() {
        let db = database();
        add_session(&db, "old", "codex", "Old work", Some("2026-11-01T00:00:00Z"));
        add_session(&db, "new", "claude", "New work", Some("2026-11-10T00:00:00Z"));
        add_session(&db, "here", "claude", "On this trail", Some("2026-11-12T00:00:00Z"));
        add_session(&db, "worker", "codex", "A worker", Some("2026-11-13T00:00:00Z"));
        add_session(&db, "pass", "claude", "An extraction pass", Some("2026-11-13T00:00:00Z"));
        add_session(&db, "gone", "claude", "Archived", Some("2026-11-13T00:00:00Z"));
        db.execute("UPDATE sessions SET parent_session_id='new' WHERE id='worker'", []).unwrap();
        db.execute("UPDATE sessions SET kind='extraction' WHERE id='pass'", []).unwrap();
        db.execute("UPDATE sessions SET archived_at='2026-11-14T00:00:00Z' WHERE id='gone'", []).unwrap();
        let trail = store::create(&db, "Mine", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &session("here"), TrailAddedBy::Person, "t0").unwrap();
        let other = store::create(&db, "Elsewhere", None, "t0").unwrap();
        store::add_member(&db, &other.id, &session("old"), TrailAddedBy::Person, "t0").unwrap();

        let found = list(&db, &trail.id, TrailMemberKind::Session, None, &[]).unwrap();
        assert_eq!(refs(&found), vec!["new", "old"]);
        assert_eq!(found[0].harness.as_deref(), Some("claude"));
        assert_eq!(found[0].on_trail_id, None);
        assert_eq!((found[1].on_trail_id.as_deref(), found[1].on_trail_title.as_deref()), (Some(other.id.as_str()), Some("Elsewhere")));

        let found = list(&db, &trail.id, TrailMemberKind::Session, Some("  OLD "), &[]).unwrap();
        assert_eq!(refs(&found), vec!["old"]);
    }

    #[test]
    fn pull_requests_drafts_and_notes_are_listed_by_kind() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", None);
        attach_pr(&db, "s1", "https://github.com/o/r/pull/7", "2026-11-01T00:00:00Z", None);
        attach_pr(&db, "s1", "https://github.com/o/r/pull/9", "2026-11-02T00:00:00Z", None);
        let draft = crate::outgoing::create_draft(
            &db,
            "s1",
            bridge_protocol::messages::OutgoingDestination::SlackDm { user: "@mika".into() },
            "Please review",
            "review",
            &[],
            "2026-11-03T00:00:00Z",
        )
        .unwrap();
        let trail = store::create(&db, "T", None, "t0").unwrap();

        let pulls = list(&db, &trail.id, TrailMemberKind::PullRequest, None, &[]).unwrap();
        assert_eq!(refs(&pulls), vec!["https://github.com/o/r/pull/9", "https://github.com/o/r/pull/7"]);
        assert!(pulls[0].title.starts_with("#9 "), "{}", pulls[0].title);
        assert_eq!(refs(&list(&db, &trail.id, TrailMemberKind::PullRequest, Some("#7"), &[]).unwrap()), vec!["https://github.com/o/r/pull/7"]);

        let drafts = list(&db, &trail.id, TrailMemberKind::Outgoing, None, &[]).unwrap();
        assert_eq!(refs(&drafts), vec![draft.id.clone()]);
        assert_eq!(drafts[0].detail.as_deref(), Some("draft, not sent yet"));

        let note = |id: &str, title: &str| Note {
            id: id.into(),
            space_id: "space".into(),
            parent_id: None,
            source_id: None,
            title: Some(title.into()),
            kind: "note".into(),
            tags: Vec::new(),
            workspace_id: None,
            state: "open".into(),
            session_ids: Vec::new(),
            source_excerpt: None,
            preview: String::new(),
            path: None,
            seq: 1,
            created_by: "agent:claude:s1".into(),
            last_actor: "agent:claude:s1".into(),
            created_at: "2026-11-04T00:00:00Z".into(),
            updated_at: "2026-11-04T00:00:00Z".into(),
        };
        // The notes store already matched the query; the list keeps its order.
        let notes = [note("plan", "Plan"), note("pr-notes", "")];
        let found = list(&db, &trail.id, TrailMemberKind::Note, Some("plan"), &notes).unwrap();
        assert_eq!(refs(&found), vec!["plan", "pr-notes"]);
        assert_eq!(found[1].title, "Untitled");
        assert_eq!(found[0].detail.as_deref(), Some("last edited by Claude"));
        let note_ref = TrailMemberRef { kind: TrailMemberKind::Note, reference: "plan".into() };
        store::add_member(&db, &trail.id, &note_ref, TrailAddedBy::Person, "t1").unwrap();
        assert_eq!(refs(&list(&db, &trail.id, TrailMemberKind::Note, None, &notes).unwrap()), vec!["pr-notes"], "already on this trail");

        assert!(list(&db, &trail.id, TrailMemberKind::Issue, None, &[]).is_err(), "an issue is added by its link");
    }
}
