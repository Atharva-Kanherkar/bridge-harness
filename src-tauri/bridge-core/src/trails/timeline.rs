//! A trail's timeline, state, stage and open items, derived on every read from
//! the records its members point at. Nothing here is stored.

use std::collections::BTreeMap;

use bridge_protocol::messages::{
    TrailDetail, TrailEvent, TrailEventKind, TrailHarnessCount, TrailLastEvent, TrailMember,
    TrailMemberKind, TrailMemberRef, TrailOpenItem, TrailOpenKind, TrailState, TrailSummary, TrailTone,
};
use rusqlite::{params, Connection, OptionalExtension};

use super::store::{self, TrailRow};
use crate::github_surface::{CheckConclusion, CheckStatus, PullRequestState};
use crate::session_prs::SessionPrView;
use crate::BridgeError;

/// The longest message excerpt a timeline row quotes, in characters.
pub const EXCERPT_CHARS: usize = 280;

/// What the trail needs to know about one notes page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFacts {
    pub id: String,
    pub title: Option<String>,
    pub created_at: String,
    pub last_actor: String,
    pub session_ids: Vec<String>,
}

/// Notes live in their own database behind `NotesService`; the trail reads
/// them through this seam so tests need no notes store.
pub trait NoteSource {
    fn note(&self, id: &str) -> Option<NoteFacts>;
    /// Every live page, for the "linked to a member chat" rule.
    fn all_notes(&self) -> Vec<NoteFacts>;
    fn markdown(&self, id: &str) -> Option<String>;
}

impl NoteSource for crate::notes::service::NotesService {
    fn note(&self, id: &str) -> Option<NoteFacts> {
        self.read(id).ok().map(|page| facts(page.note))
    }

    fn all_notes(&self) -> Vec<NoteFacts> {
        let filter = crate::notes::service::ListFilter { include_archived: true, limit: Some(2000), ..Default::default() };
        self.list(&filter).map(|notes| notes.into_iter().map(facts).collect()).unwrap_or_default()
    }

    fn markdown(&self, id: &str) -> Option<String> {
        self.read(id).ok().map(|page| page.markdown)
    }
}

/// Notes read up front, so no caller holds the core database lock while the
/// notes store takes its own.
#[derive(Debug, Default, Clone)]
pub struct NoteSnapshot {
    notes: std::collections::HashMap<String, (NoteFacts, Option<String>)>,
}

impl NoteSnapshot {
    /// Every live page's facts, without bodies.
    pub fn all(source: &dyn NoteSource) -> Self {
        Self { notes: source.all_notes().into_iter().map(|facts| (facts.id.clone(), (facts, None))).collect() }
    }

    /// Add the bodies of `ids` (and their facts, archived or not).
    pub fn with_markdown(mut self, source: &dyn NoteSource, ids: &[String]) -> Self {
        for id in ids {
            if let (Some(facts), Some(markdown)) = (source.note(id), source.markdown(id)) {
                self.notes.insert(id.clone(), (facts, Some(markdown)));
            }
        }
        self
    }
}

impl NoteSource for NoteSnapshot {
    fn note(&self, id: &str) -> Option<NoteFacts> {
        self.notes.get(id).map(|(facts, _)| facts.clone())
    }

    fn all_notes(&self) -> Vec<NoteFacts> {
        let mut notes: Vec<NoteFacts> = self.notes.values().map(|(facts, _)| facts.clone()).collect();
        notes.sort_by(|left, right| left.id.cmp(&right.id));
        notes
    }

    fn markdown(&self, id: &str) -> Option<String> {
        self.notes.get(id).and_then(|(_, markdown)| markdown.clone())
    }
}

fn facts(note: bridge_protocol::messages::Note) -> NoteFacts {
    NoteFacts { id: note.id, title: note.title, created_at: note.created_at, last_actor: note.last_actor, session_ids: note.session_ids }
}

/// A session row as the trail shows it.
#[derive(Debug, Clone)]
pub struct SessionFacts {
    pub id: String,
    pub title: String,
    pub harness: String,
    pub model: Option<String>,
    pub workspace_id: Option<String>,
    /// `started_at`, else the first forest entry.
    pub at: Option<String>,
}

pub fn session_facts(db: &Connection, id: &str) -> Result<Option<SessionFacts>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT id, COALESCE(NULLIF(title,''), label), harness, model, workspace_id,
                    COALESCE(started_at, (SELECT MIN(created_at) FROM session_entries WHERE session_id=sessions.id))
             FROM sessions WHERE id=?1",
            params![id],
            |row| {
                Ok(SessionFacts {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    harness: row.get(2)?,
                    model: row.get(3)?,
                    workspace_id: row.get(4)?,
                    at: row.get(5)?,
                })
            },
        )
        .optional()?)
}

/// A pull request on the trail: its newest-snapshot link row across chats.
#[derive(Debug, Clone)]
pub struct PullFacts {
    pub url: String,
    pub number: u64,
    pub title: String,
    pub head_branch: String,
    pub attached_at: String,
    pub fetched_at: Option<String>,
    pub view: Option<SessionPrView>,
    /// The newest chat holding this pull request.
    pub session_id: Option<String>,
}

pub fn pull_facts(db: &Connection, url: &str) -> Result<Option<PullFacts>, BridgeError> {
    let attached_at: Option<String> =
        db.query_row("SELECT MIN(created_at) FROM session_pull_requests WHERE url=?1", params![url], |row| row.get(0))?;
    let Some(attached_at) = attached_at else { return Ok(None) };
    let (number, title, head_branch, snapshot, fetched_at): (i64, String, String, Option<String>, Option<String>) = db.query_row(
        "SELECT number,title,head_branch,snapshot,fetched_at FROM session_pull_requests WHERE url=?1
         ORDER BY snapshot IS NULL, fetched_at DESC, created_at DESC LIMIT 1",
        params![url],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
    )?;
    let view = snapshot.and_then(|snapshot| serde_json::from_str::<SessionPrView>(&snapshot).ok());
    let session_id: Option<String> = db
        .query_row("SELECT session_id FROM session_pull_requests WHERE url=?1 ORDER BY created_at DESC LIMIT 1", params![url], |row| row.get(0))
        .optional()?;
    Ok(Some(PullFacts {
        session_id,
        url: url.to_owned(),
        number: number as u64,
        title: view.as_ref().map(|view| view.title.clone()).unwrap_or(title),
        head_branch: view.as_ref().map(|view| view.head_branch.clone()).unwrap_or(head_branch),
        attached_at,
        fetched_at,
        view,
    }))
}

fn failed(conclusion: Option<CheckConclusion>) -> bool {
    matches!(
        conclusion,
        Some(CheckConclusion::Failure | CheckConclusion::TimedOut | CheckConclusion::StartupFailure | CheckConclusion::ActionRequired)
    )
}

pub fn harness_name(harness: &str) -> String {
    match harness {
        "claude" => "Claude".into(),
        "codex" => "Codex".into(),
        "opencode" => "OpenCode".into(),
        "cursor" => "Cursor".into(),
        "grok" => "Grok".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
        }
    }
}

/// Who last edited a page, as a person reads it.
fn actor(last_actor: &str) -> String {
    match last_actor.split(':').collect::<Vec<_>>().as_slice() {
        ["person"] => "you".into(),
        ["agent", harness, ..] => harness_name(harness),
        ["file-import"] => "a file edit".into(),
        _ => "Bridge".into(),
    }
}

fn excerpt(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= EXCERPT_CHARS {
        return flat;
    }
    let mut cut: String = flat.chars().take(EXCERPT_CHARS).collect();
    cut.push('…');
    cut
}

fn kind_rank(kind: TrailMemberKind) -> u8 {
    match kind {
        TrailMemberKind::Issue => 0,
        TrailMemberKind::Note => 1,
        TrailMemberKind::Session => 2,
        TrailMemberKind::PullRequest => 3,
        TrailMemberKind::Outgoing => 4,
    }
}

/// RFC 3339 times compared as instants; anything unparseable sorts by text
/// after every parseable time.
fn instant(at: &str) -> (i64, String) {
    chrono::DateTime::parse_from_rfc3339(at)
        .map(|time| (time.timestamp_nanos_opt().unwrap_or(i64::MAX), String::new()))
        .unwrap_or((i64::MAX, at.to_owned()))
}

fn reference(member: &TrailMember) -> TrailMemberRef {
    TrailMemberRef { kind: member.kind, reference: member.reference.clone() }
}

fn event(
    member: &TrailMember,
    suffix: &str,
    kind: TrailEventKind,
    at: &str,
    title: String,
    tone: TrailTone,
) -> TrailEvent {
    TrailEvent {
        id: format!("{}:{}{suffix}", member.kind.as_str(), member.reference),
        kind,
        at: at.to_owned(),
        title,
        detail: None,
        excerpt: None,
        harness: None,
        url: None,
        session_id: None,
        tone,
        member: reference(member),
        added_by: member.added_by,
    }
}

/// Everything a trail page shows.
pub fn detail(db: &Connection, notes: &dyn NoteSource, trail: &TrailRow) -> Result<TrailDetail, BridgeError> {
    let members = store::members(db, &trail.id)?;
    let mut events = Vec::new();
    let mut open = Vec::new();
    let mut harnesses: BTreeMap<String, u32> = BTreeMap::new();
    let mut session_ids = Vec::new();
    let (mut has_plan, mut has_session, mut pulls) = (false, false, Vec::<PullFacts>::new());
    let (mut failing_reason, mut draft_reason, mut waiting_reason) = (None, None, None);

    for member in &members {
        match member.kind {
            TrailMemberKind::Issue => {
                // An issue is where work starts: it counts as the plan, read
                // or not. It never needs the person and opens nothing.
                has_plan = true;
                let row = match super::issues::get(db, &member.reference)? {
                    Some(issue) => {
                        let tone = if issue.completed() { TrailTone::Success } else { TrailTone::Plain };
                        let mut row = event(member, "", TrailEventKind::Issue, &issue.created_at, format!("#{} {}", issue.number, issue.title), tone);
                        let mut detail = issue.state_words().to_owned();
                        if issue.comment_count > 0 {
                            detail.push_str(&format!(" · {} comment{}", issue.comment_count, if issue.comment_count == 1 { "" } else { "s" }));
                        }
                        row.detail = Some(detail);
                        row.url = Some(issue.url);
                        row
                    }
                    None => {
                        let title = super::issues::parse_url(&member.reference)
                            .map(|issue| format!("Issue #{}", issue.number))
                            .unwrap_or_else(|| "An issue".into());
                        let mut row = event(member, "", TrailEventKind::Issue, &member.added_at, title, TrailTone::Plain);
                        row.detail = Some("not read from GitHub yet".into());
                        row.url = Some(member.reference.clone());
                        row
                    }
                };
                events.push(row);
            }
            TrailMemberKind::Session => {
                session_ids.push(member.reference.clone());
                let Some(chat) = session_facts(db, &member.reference)? else {
                    events.push(event(member, "", TrailEventKind::Removed, &member.added_at, "A chat that was deleted".into(), TrailTone::Plain));
                    continue;
                };
                has_session = true;
                *harnesses.entry(chat.harness.clone()).or_default() += 1;
                let mut row = event(member, "", TrailEventKind::Chat, chat.at.as_deref().unwrap_or(&member.added_at), chat.title.clone(), TrailTone::Plain);
                row.harness = Some(chat.harness.clone());
                row.session_id = Some(chat.id.clone());
                row.detail = Some(match &chat.model {
                    Some(model) if !model.is_empty() => format!("{} · {model}", harness_name(&chat.harness)),
                    _ => harness_name(&chat.harness),
                });
                let waiting: Option<String> = db
                    .query_row(
                        "SELECT title FROM attention_events WHERE session_id=?1 AND kind='needs_you' AND resolved_at IS NULL
                         AND superseded_by IS NULL ORDER BY sequence DESC LIMIT 1",
                        params![chat.id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if waiting.is_some() {
                    row.tone = TrailTone::Warning;
                    open.push(TrailOpenItem {
                        kind: TrailOpenKind::ChatWaiting,
                        label: format!("“{}” is waiting for you", chat.title),
                        member: reference(member),
                        url: None,
                    });
                    waiting_reason.get_or_insert_with(|| format!("“{}” is waiting for you", chat.title));
                }
                events.push(row);
            }
            TrailMemberKind::Note => {
                let Some(note) = notes.note(&member.reference) else {
                    events.push(event(member, "", TrailEventKind::Removed, &member.added_at, "A note that was deleted".into(), TrailTone::Plain));
                    continue;
                };
                has_plan = true;
                let title = note.title.clone().filter(|title| !title.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
                let mut row = event(member, "", TrailEventKind::Note, &note.created_at, title, TrailTone::Plain);
                row.detail = Some(format!("last edited by {}", actor(&note.last_actor)));
                events.push(row);
            }
            TrailMemberKind::PullRequest => {
                let Some(pull) = pull_facts(db, &member.reference)? else {
                    events.push(event(member, "", TrailEventKind::Removed, &member.added_at, "A pull request no chat holds any more".into(), TrailTone::Plain));
                    continue;
                };
                let merged = pull.view.as_ref().is_some_and(|view| view.state == PullRequestState::Merged);
                let mut row = event(
                    member,
                    "",
                    TrailEventKind::PullRequest,
                    &pull.attached_at,
                    format!("#{} {}", pull.number, pull.title),
                    if merged { TrailTone::Success } else { TrailTone::Plain },
                );
                row.url = Some(pull.url.clone());
                row.session_id = pull.session_id.clone();
                row.detail = Some(match &pull.view {
                    Some(view) => match view.state {
                        PullRequestState::Merged => "merged".into(),
                        PullRequestState::Closed => "closed".into(),
                        PullRequestState::Open if view.is_draft => "open · draft".into(),
                        PullRequestState::Open => "open".into(),
                    },
                    None => "not read from GitHub yet".into(),
                });
                events.push(row);
                if let Some(view) = &pull.view {
                    let checks = &view.checks;
                    if checks.total > 0 {
                        let failing: Vec<&str> = view
                            .check_details
                            .iter()
                            .filter(|check| check.status == CheckStatus::Completed && failed(check.conclusion))
                            .map(|check| check.name.as_str())
                            .collect();
                        let open_pr = view.state == PullRequestState::Open;
                        let tone = if merged {
                            TrailTone::Success
                        } else if checks.failed > 0 && open_pr {
                            TrailTone::Danger
                        } else {
                            TrailTone::Plain
                        };
                        let mut row = event(
                            member,
                            "#checks",
                            TrailEventKind::Checks,
                            pull.fetched_at.as_deref().unwrap_or(&pull.attached_at),
                            format!("Checks on #{}", pull.number),
                            tone,
                        );
                        let mut detail = format!("{} of {} passing", checks.passed, checks.total);
                        if checks.failed > 0 {
                            detail.push_str(&format!(" · {} failing", checks.failed));
                        }
                        if checks.in_progress + checks.queued > 0 {
                            detail.push_str(&format!(" · {} running", checks.in_progress + checks.queued));
                        }
                        row.detail = Some(detail);
                        row.url = Some(pull.url.clone());
                        events.push(row);
                        if checks.failed > 0 && open_pr {
                            if failing.is_empty() {
                                open.push(TrailOpenItem {
                                    kind: TrailOpenKind::FailingCheck,
                                    label: format!("{} failing check{} on #{}", checks.failed, if checks.failed == 1 { "" } else { "s" }, pull.number),
                                    member: reference(member),
                                    url: Some(pull.url.clone()),
                                });
                            }
                            for name in &failing {
                                let url = view
                                    .check_details
                                    .iter()
                                    .find(|check| check.name == *name && !check.log_url.is_empty())
                                    .map(|check| check.log_url.clone())
                                    .unwrap_or_else(|| pull.url.clone());
                                open.push(TrailOpenItem {
                                    kind: TrailOpenKind::FailingCheck,
                                    label: format!("{name} failing on #{}", pull.number),
                                    member: reference(member),
                                    url: Some(url),
                                });
                            }
                            failing_reason.get_or_insert_with(|| match failing.as_slice() {
                                [one] => format!("{one} is failing on #{}", pull.number),
                                _ => format!("{} checks are failing on #{}", checks.failed, pull.number),
                            });
                        }
                    }
                }
                pulls.push(pull);
            }
            TrailMemberKind::Outgoing => {
                let Some(message) = crate::outgoing::get(db, &member.reference)? else {
                    events.push(event(member, "", TrailEventKind::Removed, &member.added_at, "A drafted message that was deleted".into(), TrailTone::Plain));
                    continue;
                };
                use bridge_protocol::messages::OutgoingState;
                let (tone, detail, at) = match message.state {
                    OutgoingState::Draft => (TrailTone::Warning, "draft, not sent yet".to_owned(), message.updated_at.clone()),
                    OutgoingState::Copied => (TrailTone::Plain, "copied to send".to_owned(), message.updated_at.clone()),
                    OutgoingState::Discarded => (TrailTone::Plain, "discarded".to_owned(), message.updated_at.clone()),
                    OutgoingState::Sending => (TrailTone::Plain, "sending".to_owned(), message.updated_at.clone()),
                    OutgoingState::Sent => (TrailTone::Success, "sent".to_owned(), message.sent_at.clone().unwrap_or_else(|| message.updated_at.clone())),
                    OutgoingState::Failed => (
                        TrailTone::Danger,
                        match message.last_error.as_deref().map(str::trim).filter(|error| !error.is_empty()) {
                            Some(error) => format!("send failed: {error}"),
                            None => "send failed".to_owned(),
                        },
                        message.updated_at.clone(),
                    ),
                };
                let mut row = event(member, "", TrailEventKind::Message, &at, format!("Message to {}", message.label), tone);
                row.detail = Some(detail);
                row.excerpt = Some(excerpt(&message.body));
                row.session_id = Some(message.session_id.clone());
                // A sent message opens where it was posted.
                row.url = message.permalink.clone().filter(|_| message.state == OutgoingState::Sent);
                events.push(row);
                match message.state {
                    OutgoingState::Draft => {
                        open.push(TrailOpenItem {
                            kind: TrailOpenKind::UnsentDraft,
                            label: format!("Send or discard the draft to {}", message.label),
                            member: reference(member),
                            url: None,
                        });
                        draft_reason.get_or_insert_with(|| format!("A draft to {} is not sent", message.label));
                    }
                    OutgoingState::Failed => {
                        open.push(TrailOpenItem {
                            kind: TrailOpenKind::UnsentDraft,
                            label: format!("Retry or discard the message to {}", message.label),
                            member: reference(member),
                            url: None,
                        });
                        draft_reason.get_or_insert_with(|| format!("A message to {} failed to send", message.label));
                    }
                    OutgoingState::Copied | OutgoingState::Discarded | OutgoingState::Sending | OutgoingState::Sent => {}
                }
            }
        }
    }

    events.sort_by(|left, right| {
        instant(&left.at)
            .cmp(&instant(&right.at))
            .then(kind_rank(left.member.kind).cmp(&kind_rank(right.member.kind)))
            .then(left.member.reference.cmp(&right.member.reference))
            .then(left.id.cmp(&right.id))
    });

    let needs_you_reason = failing_reason.or(draft_reason).or(waiting_reason);
    let shipped = needs_you_reason.is_none()
        && !pulls.is_empty()
        && pulls.iter().all(|pull| {
            pull.view.as_ref().is_some_and(|view| matches!(view.state, PullRequestState::Merged | PullRequestState::Closed))
        })
        && pulls.iter().any(|pull| pull.view.as_ref().is_some_and(|view| view.state == PullRequestState::Merged));
    let state = if needs_you_reason.is_some() {
        TrailState::NeedsYou
    } else if shipped {
        TrailState::Shipped
    } else {
        TrailState::InProgress
    };
    let stage = if shipped {
        4
    } else if !pulls.is_empty() {
        3
    } else if has_session {
        2
    } else if has_plan {
        1
    } else {
        0
    };
    let branch = pulls
        .iter()
        .max_by(|left, right| instant(&left.attached_at).cmp(&instant(&right.attached_at)))
        .map(|pull| pull.head_branch.clone())
        .filter(|branch| !branch.is_empty());
    let last_event = events.last().map(|event| TrailLastEvent { title: event.title.clone(), at: event.at.clone() });
    let summary = TrailSummary {
        id: trail.id.clone(),
        title: trail.title.clone(),
        workspace_id: trail.workspace_id.clone(),
        state,
        needs_you_reason,
        stage,
        last_event,
        harnesses: harnesses.into_iter().map(|(harness, chats)| TrailHarnessCount { harness, chats }).collect(),
        session_ids,
        member_count: members.len() as u32,
        created_at: trail.created_at.clone(),
        updated_at: trail.updated_at.clone(),
        archived_at: trail.archived_at.clone(),
    };
    Ok(TrailDetail { summary, members, events, open, branch })
}

pub fn summary(db: &Connection, notes: &dyn NoteSource, trail: &TrailRow) -> Result<TrailSummary, BridgeError> {
    Ok(detail(db, notes, trail)?.summary)
}

/// Needs you, then in progress, then shipped; newest change first in each.
pub fn sort_summaries(summaries: &mut [TrailSummary]) {
    let rank = |state: TrailState| match state {
        TrailState::NeedsYou => 0,
        TrailState::InProgress => 1,
        TrailState::Shipped => 2,
    };
    summaries.sort_by(|left, right| {
        rank(left.state)
            .cmp(&rank(right.state))
            .then(instant(&right.updated_at).cmp(&instant(&left.updated_at)))
            .then(left.id.cmp(&right.id))
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::github_surface::{CheckRollup, PullRequestCheck};
    use crate::trails::store::tests::{database, session};
    use bridge_protocol::messages::TrailAddedBy;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct FakeNotes(pub HashMap<String, (NoteFacts, String)>);

    impl FakeNotes {
        pub fn with(mut self, id: &str, title: &str, at: &str, sessions: &[&str], body: &str) -> Self {
            self.0.insert(
                id.into(),
                (
                    NoteFacts {
                        id: id.into(),
                        title: Some(title.into()),
                        created_at: at.into(),
                        last_actor: "person".into(),
                        session_ids: sessions.iter().map(|s| s.to_string()).collect(),
                    },
                    body.into(),
                ),
            );
            self
        }
    }

    impl NoteSource for FakeNotes {
        fn note(&self, id: &str) -> Option<NoteFacts> {
            self.0.get(id).map(|(facts, _)| facts.clone())
        }
        fn all_notes(&self) -> Vec<NoteFacts> {
            let mut notes: Vec<_> = self.0.values().map(|(facts, _)| facts.clone()).collect();
            notes.sort_by(|a, b| a.id.cmp(&b.id));
            notes
        }
        fn markdown(&self, id: &str) -> Option<String> {
            self.0.get(id).map(|(_, body)| body.clone())
        }
    }

    pub fn add_session(db: &Connection, id: &str, harness: &str, title: &str, started: Option<&str>) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,depth,title,started_at) VALUES(?1,NULL,?2,'Chat','idle','estimated','direct',0,?3,?4)",
            params![id, harness, title, started],
        )
        .unwrap();
    }

    pub fn view(state: PullRequestState, checks: Vec<(&str, Option<CheckConclusion>)>) -> SessionPrView {
        let details: Vec<PullRequestCheck> = checks
            .iter()
            .map(|(name, conclusion)| PullRequestCheck {
                name: name.to_string(),
                status: CheckStatus::Completed,
                conclusion: *conclusion,
                log_url: format!("https://ci/{name}"),
                workflow: "ci".into(),
            })
            .collect();
        SessionPrView {
            number: 12,
            title: "Archive a chat".into(),
            url: "https://github.com/o/r/pull/12".into(),
            state,
            is_draft: false,
            mergeability: crate::github_surface::Mergeability::Unknown,
            head_branch: "feat/archive".into(),
            head_sha: "abc".into(),
            checks: crate::session_prs::rollup_for(&details),
            check_details: details,
            attribution: "manual".into(),
            attached_at: "2026-11-10T12:50:00Z".into(),
            fetched_at: Some("2026-11-11T08:44:00Z".into()),
            stale: false,
            error: None,
        }
    }

    pub fn attach_pr(db: &Connection, session_id: &str, url: &str, created: &str, view: Option<&SessionPrView>) {
        db.execute(
            "INSERT INTO session_pull_requests(session_id,workspace_id,repo_host,repo_owner,repo_name,number,url,title,head_branch,head_sha,attribution,snapshot,fetched_at,created_at)
             VALUES(?1,'w','github.com','o','r',?6,?2,'Archive a chat','feat/archive','abc','manual',?3,?4,?5)",
            params![
                session_id,
                url,
                view.map(|view| serde_json::to_string(view).unwrap()),
                view.and_then(|view| view.fetched_at.clone()),
                created,
                url.rsplit('/').next().and_then(|number| number.parse::<i64>().ok()).unwrap_or(12)
            ],
        )
        .unwrap();
    }

    fn pr(url: &str) -> TrailMemberRef {
        TrailMemberRef { kind: TrailMemberKind::PullRequest, reference: url.into() }
    }

    fn note(id: &str) -> TrailMemberRef {
        TrailMemberRef { kind: TrailMemberKind::Note, reference: id.into() }
    }

    const URL: &str = "https://github.com/o/r/pull/12";

    #[test]
    fn orders_events_across_sources_with_stable_ties() {
        let db = database();
        add_session(&db, "s1", "claude", "Design it", Some("2026-11-09T09:20:00Z"));
        add_session(&db, "s2", "codex", "Build it", Some("2026-11-10T11:34:00+00:00"));
        attach_pr(&db, "s2", URL, "2026-11-10T12:50:00Z", Some(&view(PullRequestState::Open, vec![("lint", Some(CheckConclusion::Success))])));
        let notes = FakeNotes::default().with("n1", "Plan", "2026-11-09T09:20:00Z", &[], "body");
        let trail = store::create(&db, "T", None, "2026-11-09T09:00:00Z").unwrap();
        for member in [session("s2"), pr(URL), session("s1"), note("n1")] {
            store::add_member(&db, &trail.id, &member, TrailAddedBy::Person, "2026-11-12T00:00:00Z").unwrap();
        }
        let detail = detail(&db, &notes, &trail).unwrap();
        let titles: Vec<_> = detail.events.iter().map(|event| (event.kind, event.title.as_str())).collect();
        assert_eq!(
            titles,
            vec![
                (TrailEventKind::Note, "Plan"),
                (TrailEventKind::Chat, "Design it"),
                (TrailEventKind::Chat, "Build it"),
                (TrailEventKind::PullRequest, "#12 Archive a chat"),
                (TrailEventKind::Checks, "Checks on #12"),
            ],
            "same instant: the note sorts before the chat; mixed offsets compare as instants"
        );
        assert_eq!(detail.events[1].detail.as_deref(), Some("Claude"));
        assert_eq!(detail.events[1].session_id.as_deref(), Some("s1"));
        assert_eq!(detail.events[3].session_id.as_deref(), Some("s2"), "a PR opens the chat holding it");
        assert_eq!(detail.events[4].detail.as_deref(), Some("1 of 1 passing"));
        let again = super::detail(&db, &notes, &trail).unwrap();
        assert_eq!(again.events, detail.events);
        assert_eq!(detail.branch.as_deref(), Some("feat/archive"));
        assert_eq!(detail.summary.harnesses, vec![
            TrailHarnessCount { harness: "claude".into(), chats: 1 },
            TrailHarnessCount { harness: "codex".into(), chats: 1 },
        ]);
        assert_eq!(detail.summary.last_event.as_ref().unwrap().title, "Checks on #12");
    }

    #[test]
    fn gone_records_render_as_removed() {
        let db = database();
        let trail = store::create(&db, "T", None, "t0").unwrap();
        for member in [session("gone"), note("gone"), pr("https://github.com/o/r/pull/9"), TrailMemberRef { kind: TrailMemberKind::Outgoing, reference: "gone".into() }] {
            store::add_member(&db, &trail.id, &member, TrailAddedBy::Person, "2026-11-12T00:00:00Z").unwrap();
        }
        let detail = detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(detail.events.len(), 4);
        assert!(detail.events.iter().all(|event| event.kind == TrailEventKind::Removed));
        assert_eq!(detail.summary.stage, 0);
        assert_eq!(detail.summary.state, TrailState::InProgress);
    }

    #[test]
    fn state_needs_you_shipped_in_progress() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", None);
        let trail = store::create(&db, "T", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "2026-11-12T00:00:00Z").unwrap();
        assert_eq!(summary(&db, &FakeNotes::default(), &trail).unwrap().state, TrailState::InProgress);

        attach_pr(&db, "s1", URL, "2026-11-12T01:00:00Z", Some(&view(PullRequestState::Open, vec![("vitest", Some(CheckConclusion::Failure)), ("lint", Some(CheckConclusion::Success))])));
        store::add_member(&db, &trail.id, &pr(URL), TrailAddedBy::Auto, "2026-11-12T01:00:00Z").unwrap();
        let detail = detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(detail.summary.state, TrailState::NeedsYou);
        assert_eq!(detail.summary.needs_you_reason.as_deref(), Some("vitest is failing on #12"));
        assert_eq!(detail.open.len(), 1);
        assert_eq!(detail.open[0].kind, TrailOpenKind::FailingCheck);
        assert_eq!(detail.open[0].url.as_deref(), Some("https://ci/vitest"));
        assert_eq!(detail.events.iter().find(|e| e.kind == TrailEventKind::Checks).unwrap().tone, TrailTone::Danger);

        db.execute("UPDATE session_pull_requests SET snapshot=?1", params![serde_json::to_string(&view(PullRequestState::Merged, vec![("lint", Some(CheckConclusion::Success))])).unwrap()]).unwrap();
        let detail = super::detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(detail.summary.state, TrailState::Shipped);
        assert_eq!(detail.summary.stage, 4);
        assert!(detail.open.is_empty());

        // A chat waiting on the person outranks nothing but still needs you.
        db.execute(
            "INSERT INTO attention_events(id,kind,severity,session_id,title,created_at) VALUES('a1','needs_you','info','s1','Bridge needs you','t')",
            [],
        )
        .unwrap();
        let detail = super::detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(detail.summary.state, TrailState::NeedsYou);
        assert_eq!(detail.summary.needs_you_reason.as_deref(), Some("“Build” is waiting for you"));
        assert_eq!(detail.open[0].kind, TrailOpenKind::ChatWaiting);
    }

    #[test]
    fn stage_is_the_highest_reached() {
        let db = database();
        let trail = store::create(&db, "T", None, "t0").unwrap();
        let notes = FakeNotes::default().with("n1", "Plan", "t1", &[], "");
        store::add_member(&db, &trail.id, &note("n1"), TrailAddedBy::Person, "t1").unwrap();
        assert_eq!(summary(&db, &notes, &trail).unwrap().stage, 1);
        add_session(&db, "s1", "codex", "Build", None);
        store::add_member(&db, &trail.id, &session("s1"), TrailAddedBy::Person, "t2").unwrap();
        assert_eq!(summary(&db, &notes, &trail).unwrap().stage, 2);
        attach_pr(&db, "s1", URL, "t3", None);
        store::add_member(&db, &trail.id, &pr(URL), TrailAddedBy::Person, "t3").unwrap();
        let summary = summary(&db, &notes, &trail).unwrap();
        assert_eq!(summary.stage, 3, "an unread PR is still a PR");
        assert_eq!(summary.state, TrailState::InProgress, "an unread PR never counts as shipped");
    }

    fn issue(url: &str) -> TrailMemberRef {
        TrailMemberRef { kind: TrailMemberKind::Issue, reference: url.into() }
    }

    #[test]
    fn issue_rows_and_ties() {
        use crate::trails::issues::{self, tests::snapshot};
        let db = database();
        let trail = store::create(&db, "T", None, "t0").unwrap();
        let mut open = snapshot(41, true, None, &[]);
        open.comment_count = 3;
        issues::save(&db, &open).unwrap();
        issues::save(&db, &snapshot(42, false, Some("COMPLETED"), &[])).unwrap();
        let mut dropped = snapshot(43, false, Some("NOT_PLANNED"), &[]);
        dropped.comment_count = 0;
        dropped.created_at = "2026-11-09T09:20:00Z".into();
        issues::save(&db, &dropped).unwrap();
        // A note at the same instant as issue 43: the issue sorts first.
        let notes = FakeNotes::default().with("n1", "Plan", "2026-11-09T09:20:00Z", &[], "");
        for member in [note("n1"), issue("https://github.com/o/r/issues/43"), issue("https://github.com/o/r/issues/41"), issue("https://github.com/o/r/issues/42"), issue("https://github.com/o/r/issues/44")] {
            store::add_member(&db, &trail.id, &member, TrailAddedBy::Person, "2026-11-12T00:00:00Z").unwrap();
        }
        let detail = detail(&db, &notes, &trail).unwrap();
        let rows: Vec<_> = detail.events.iter().map(|event| (event.kind, event.title.as_str(), event.detail.as_deref().unwrap_or(""), event.tone)).collect();
        assert_eq!(
            rows,
            vec![
                (TrailEventKind::Issue, "#41 Archived chats stay searchable", "open · 3 comments", TrailTone::Plain),
                (TrailEventKind::Issue, "#42 Archived chats stay searchable", "closed · 1 comment", TrailTone::Success),
                (TrailEventKind::Issue, "#43 Archived chats stay searchable", "closed as not planned", TrailTone::Plain),
                (TrailEventKind::Note, "Plan", "last edited by you", TrailTone::Plain),
                (TrailEventKind::Issue, "Issue #44", "not read from GitHub yet", TrailTone::Plain),
            ]
        );
        assert_eq!(detail.events[0].url.as_deref(), Some("https://github.com/o/r/issues/41"));
        assert_eq!(detail.events[4].url.as_deref(), Some("https://github.com/o/r/issues/44"), "an unread issue still links out");
    }

    #[test]
    fn an_issue_counts_as_plan_and_never_needs_you() {
        let db = database();
        let trail = store::create(&db, "T", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &issue("https://github.com/o/r/issues/41"), TrailAddedBy::Person, "t1").unwrap();
        let unread = detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!((unread.summary.stage, unread.summary.state), (1, TrailState::InProgress));
        crate::trails::issues::save(&db, &crate::trails::issues::tests::snapshot(41, true, None, &[])).unwrap();
        let read = detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!((read.summary.stage, read.summary.state), (1, TrailState::InProgress));
        assert!(read.open.is_empty());
    }

    #[test]
    fn sent_and_failed_messages_read_as_what_happened() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", None);
        let destination = bridge_protocol::messages::OutgoingDestination::SlackDm { user: "@mika".into() };
        let message = crate::outgoing::create_draft(&db, "s1", destination, "Could you review #12 today?", "ask", &[], "2026-11-12T00:00:00Z").unwrap();
        let trail = store::create(&db, "T", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Outgoing, reference: message.id.clone() }, TrailAddedBy::Auto, "t1").unwrap();
        db.execute(
            "UPDATE outgoing_messages SET state='sent', permalink='https://acme.slack.com/archives/D1/p1', sent_at='2026-11-12T01:00:00Z' WHERE id=?1",
            params![message.id],
        )
        .unwrap();
        let sent = detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(sent.summary.state, TrailState::InProgress, "a sent message waits on nobody");
        assert_eq!(sent.events[0].detail.as_deref(), Some("sent"));
        assert_eq!(sent.events[0].tone, TrailTone::Success);
        assert_eq!(sent.events[0].at, "2026-11-12T01:00:00Z");
        assert_eq!(sent.events[0].url.as_deref(), Some("https://acme.slack.com/archives/D1/p1"));
        assert!(sent.open.is_empty());
        db.execute("UPDATE outgoing_messages SET state='failed', last_error='channel_not_found' WHERE id=?1", params![message.id]).unwrap();
        let failed = super::detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(failed.summary.state, TrailState::NeedsYou);
        assert_eq!(failed.summary.needs_you_reason.as_deref(), Some("A message to Slack · @mika failed to send"));
        assert_eq!(failed.events[0].detail.as_deref(), Some("send failed: channel_not_found"));
        assert_eq!(failed.events[0].tone, TrailTone::Danger);
        assert!(failed.events[0].url.is_none());
        assert_eq!(failed.open[0].label, "Retry or discard the message to Slack · @mika");
    }

    #[test]
    fn a_draft_needs_you_and_quotes_its_body() {
        let db = database();
        add_session(&db, "s1", "codex", "Build", None);
        let destination = bridge_protocol::messages::OutgoingDestination::SlackDm { user: "@mika".into() };
        let created = crate::outgoing::create_draft(&db, "s1", destination, "Could you review #12 today?", "ask for review", &[], "2026-11-12T00:00:00Z");
        let id = match created {
            Ok(message) => message.id,
            Err(error) => panic!("{error}"),
        };
        let trail = store::create(&db, "T", None, "t0").unwrap();
        store::add_member(&db, &trail.id, &TrailMemberRef { kind: TrailMemberKind::Outgoing, reference: id }, TrailAddedBy::Auto, "t1").unwrap();
        let detail = detail(&db, &FakeNotes::default(), &trail).unwrap();
        assert_eq!(detail.summary.state, TrailState::NeedsYou);
        assert_eq!(detail.events[0].excerpt.as_deref(), Some("Could you review #12 today?"));
        assert_eq!(detail.events[0].tone, TrailTone::Warning);
        assert_eq!(detail.events[0].session_id.as_deref(), Some("s1"), "a message opens the chat that drafted it");
        assert_eq!(detail.open[0].kind, TrailOpenKind::UnsentDraft);
        let _ = CheckRollup::default();
    }
}
