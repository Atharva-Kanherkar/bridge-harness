//! Outgoing: messages a chat drafts for a person or a place.
//!
//! A third of chats end in text meant for someone else: a Slack reply, a
//! GitHub comment, a post. This module keeps each such text as a record of
//! its own rather than a paragraph in a transcript, so it can be checked
//! against the person's style rules, shown the way its destination shows it,
//! edited, copied, and (in a later slice) sent and answered.
//!
//! An agent creates a draft through `outgoing_draft` (see
//! `crate::outgoing_tools`); the person changes it through the protocol.
//! Every text change bumps `revision`, and a change or copy that names an
//! older revision is refused, so what the person acts on is always what they
//! last saw.

pub mod attachments;
pub mod lint;
pub mod render;

use bridge_protocol::messages::{
    OutgoingAttachment, OutgoingDestination, OutgoingFormat, OutgoingLintFinding, OutgoingMessage, OutgoingState,
    MAX_OUTGOING_BODY_CHARS,
};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde_json::Value;

use crate::BridgeError;

/// Drafts one chat may hold. A chat that needs more is looping.
pub const MAX_DRAFTS_PER_CHAT: i64 = 20;
pub const MAX_REASON_CHARS: usize = 300;

pub fn install_store(db: &Connection) -> Result<(), BridgeError> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS outgoing_messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            destination TEXT NOT NULL,
            body TEXT NOT NULL,
            rendered TEXT NOT NULL,
            lint TEXT NOT NULL DEFAULT '[]',
            attachments TEXT NOT NULL DEFAULT '[]',
            state TEXT NOT NULL,
            revision INTEGER NOT NULL DEFAULT 1,
            reason TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_outgoing_messages_session ON outgoing_messages(session_id, created_at);",
    )?;
    Ok(())
}

pub fn format_of(destination: &OutgoingDestination) -> OutgoingFormat {
    match destination {
        OutgoingDestination::SlackChannel { .. }
        | OutgoingDestination::SlackThread { .. }
        | OutgoingDestination::SlackDm { .. } => OutgoingFormat::SlackMarkdown,
        OutgoingDestination::GithubComment { .. } | OutgoingDestination::GithubReviewReply { .. } => {
            OutgoingFormat::Github
        }
        OutgoingDestination::Clipboard { format } => *format,
    }
}

/// The chip a card shows for a destination.
pub fn label(destination: &OutgoingDestination) -> String {
    match destination {
        OutgoingDestination::SlackChannel { channel } => format!("Slack \u{b7} {channel}"),
        OutgoingDestination::SlackThread { channel, .. } => format!("Slack \u{b7} {channel} \u{b7} thread"),
        OutgoingDestination::SlackDm { user } => format!("Slack \u{b7} {user}"),
        OutgoingDestination::GithubComment { repo, number } => format!("GitHub \u{b7} {repo}#{number}"),
        OutgoingDestination::GithubReviewReply { repo, number, .. } => {
            format!("GitHub \u{b7} {repo}#{number} \u{b7} reply")
        }
        OutgoingDestination::Clipboard { format } => format!(
            "Clipboard \u{b7} {}",
            match format {
                OutgoingFormat::Slack | OutgoingFormat::SlackMarkdown => "Slack",
                OutgoingFormat::Github => "GitHub",
                OutgoingFormat::Plain => "plain text",
            }
        ),
    }
}

pub const DESTINATION_HELP: &str = "to must be exactly one of {\"slack\": {\"channel\": \"#dev\"}}, {\"slack\": {\"channel\": \"#dev\", \"thread\": \"<permalink>\"}}, {\"slack\": {\"user\": \"@name\"}}, {\"github\": {\"repo\": \"owner/name\", \"number\": 123}} (optionally \"replyTo\": <review comment id>), or {\"clipboard\": \"slack\" | \"github\" | \"plain\"}";

fn named(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(Value::String(text)) if !text.trim().is_empty() && text.chars().count() <= 200 => {
            Ok(Some(text.trim().to_owned()))
        }
        Some(_) => Err(format!("{key} must be a non-empty name of at most 200 characters")),
    }
}

fn positive(object: &serde_json::Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(value) => value.as_u64().filter(|number| *number > 0).map(Some).ok_or_else(|| format!("{key} must be a positive whole number")),
    }
}

fn only_keys(object: &serde_json::Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("unknown field {key}; {DESTINATION_HELP}")),
        None => Ok(()),
    }
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |part: Option<&str>| {
        part.is_some_and(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// The destination an agent named in `to`.
pub fn parse_destination(to: &Value) -> Result<OutgoingDestination, String> {
    let object = to.as_object().filter(|object| object.len() == 1).ok_or_else(|| DESTINATION_HELP.to_owned())?;
    let (family, inner) = object.iter().next().expect("one entry");
    match family.as_str() {
        "slack" => {
            let inner = inner.as_object().ok_or_else(|| DESTINATION_HELP.to_owned())?;
            only_keys(inner, &["channel", "thread", "user"])?;
            match (named(inner, "channel")?, named(inner, "thread")?, named(inner, "user")?) {
                (Some(channel), None, None) => Ok(OutgoingDestination::SlackChannel { channel }),
                (Some(channel), Some(thread), None) => Ok(OutgoingDestination::SlackThread { channel, thread }),
                (None, None, Some(user)) => Ok(OutgoingDestination::SlackDm { user }),
                _ => Err(format!("a Slack destination names a channel (optionally with a thread) or a user; {DESTINATION_HELP}")),
            }
        }
        "github" => {
            let inner = inner.as_object().ok_or_else(|| DESTINATION_HELP.to_owned())?;
            only_keys(inner, &["repo", "number", "replyTo"])?;
            let repo = named(inner, "repo")?.filter(|repo| valid_repo(repo)).ok_or_else(|| "repo must be owner/name".to_owned())?;
            let number = positive(inner, "number")?.ok_or_else(|| "number must be a positive whole number".to_owned())?;
            Ok(match positive(inner, "replyTo")? {
                Some(comment_id) => OutgoingDestination::GithubReviewReply { repo, number, comment_id },
                None => OutgoingDestination::GithubComment { repo, number },
            })
        }
        "clipboard" => match inner.as_str() {
            Some("slack") => Ok(OutgoingDestination::Clipboard { format: OutgoingFormat::Slack }),
            Some("github") => Ok(OutgoingDestination::Clipboard { format: OutgoingFormat::Github }),
            Some("plain") => Ok(OutgoingDestination::Clipboard { format: OutgoingFormat::Plain }),
            _ => Err(DESTINATION_HELP.to_owned()),
        },
        _ => Err(DESTINATION_HELP.to_owned()),
    }
}

/// A body is between 1 and `MAX_OUTGOING_BODY_CHARS` characters once trimmed.
pub fn check_body(body: &str) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return Err("the message is empty".into());
    }
    let count = body.chars().count();
    if count > MAX_OUTGOING_BODY_CHARS {
        return Err(format!("the message is {count} characters; the most is {MAX_OUTGOING_BODY_CHARS}"));
    }
    Ok(body.to_owned())
}

fn state_name(state: OutgoingState) -> &'static str {
    match state {
        OutgoingState::Draft => "draft",
        OutgoingState::Copied => "copied",
        OutgoingState::Discarded => "discarded",
    }
}

fn parse_state(text: &str) -> OutgoingState {
    match text {
        "copied" => OutgoingState::Copied,
        "discarded" => OutgoingState::Discarded,
        _ => OutgoingState::Draft,
    }
}

const COLUMNS: &str = "id,session_id,destination,body,rendered,lint,state,revision,reason,created_at,updated_at,attachments";

fn from_row(row: &Row<'_>) -> rusqlite::Result<OutgoingMessage> {
    let destination: OutgoingDestination = serde_json::from_str(&row.get::<_, String>(2)?)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(error)))?;
    let lint: Vec<OutgoingLintFinding> = serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or_default();
    Ok(OutgoingMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        label: label(&destination),
        format: format_of(&destination),
        destination,
        body: row.get(3)?,
        rendered: row.get(4)?,
        lint,
        state: parse_state(&row.get::<_, String>(6)?),
        revision: row.get(7)?,
        reason: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        attachments: serde_json::from_str(&row.get::<_, String>(11)?).unwrap_or_default(),
    })
}

pub fn get(db: &Connection, id: &str) -> Result<Option<OutgoingMessage>, BridgeError> {
    Ok(db
        .query_row(&format!("SELECT {COLUMNS} FROM outgoing_messages WHERE id=?1"), params![id], from_row)
        .optional()?)
}

/// A chat's drafts, oldest first, discarded ones included.
pub fn list(db: &Connection, session_id: &str) -> Result<Vec<OutgoingMessage>, BridgeError> {
    let mut statement =
        db.prepare(&format!("SELECT {COLUMNS} FROM outgoing_messages WHERE session_id=?1 ORDER BY created_at, id"))?;
    let items = statement.query_map(params![session_id], from_row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(items)
}

pub fn count_for_session(db: &Connection, session_id: &str) -> Result<i64, BridgeError> {
    Ok(db.query_row("SELECT COUNT(*) FROM outgoing_messages WHERE session_id=?1", params![session_id], |row| row.get(0))?)
}

/// What a body looks like after the agent-side pass: every fixable rule
/// applied and recorded, then whatever only the person can resolve.
fn fixed_text(body: &str, format: OutgoingFormat) -> (String, Vec<OutgoingLintFinding>) {
    let (text, mut findings) = lint::fix(body);
    findings.extend(lint::check(&text, format));
    (text, findings)
}

fn write(db: &Connection, item: &OutgoingMessage) -> Result<(), BridgeError> {
    db.execute(
        "INSERT INTO outgoing_messages(id,session_id,destination,body,rendered,lint,state,revision,reason,created_at,updated_at,attachments)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
         ON CONFLICT(id) DO UPDATE SET body=excluded.body, rendered=excluded.rendered, lint=excluded.lint,
           state=excluded.state, revision=excluded.revision, updated_at=excluded.updated_at, attachments=excluded.attachments",
        params![
            item.id,
            item.session_id,
            serde_json::to_string(&item.destination).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            item.body,
            item.rendered,
            serde_json::to_string(&item.lint).map_err(|error| BridgeError::Invalid(error.to_string()))?,
            state_name(item.state),
            item.revision,
            item.reason,
            item.created_at,
            item.updated_at,
            serde_json::to_string(&item.attachments).map_err(|error| BridgeError::Invalid(error.to_string()))?,
        ],
    )?;
    Ok(())
}

/// GitHub's API cannot attach an upload to a comment; the image has to be
/// hosted and linked in the body instead.
fn attachments_for(destination: &OutgoingDestination, paths: &[String]) -> Result<Vec<OutgoingAttachment>, BridgeError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    if matches!(destination, OutgoingDestination::GithubComment { .. } | OutgoingDestination::GithubReviewReply { .. }) {
        return Err(BridgeError::Invalid(
            "GitHub comments cannot carry uploaded images; link a hosted image in the body instead".into(),
        ));
    }
    attachments::validate(paths).map_err(BridgeError::Invalid)
}

/// Store an agent's draft with every fixable rule already applied.
pub fn create_draft(
    db: &Connection,
    session_id: &str,
    destination: OutgoingDestination,
    body: &str,
    reason: &str,
    attachment_paths: &[String],
    now: &str,
) -> Result<OutgoingMessage, BridgeError> {
    let body = check_body(body).map_err(BridgeError::Invalid)?;
    let attachments = attachments_for(&destination, attachment_paths)?;
    let format = format_of(&destination);
    let (body, lint) = fixed_text(&body, format);
    let item = OutgoingMessage {
        id: uuid::Uuid::now_v7().to_string(),
        session_id: session_id.to_owned(),
        label: label(&destination),
        format,
        destination,
        rendered: render::render(&body, format),
        body,
        lint,
        attachments,
        state: OutgoingState::Draft,
        revision: 1,
        reason: reason.trim().to_owned(),
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
    };
    write(db, &item)?;
    Ok(item)
}

fn current(db: &Connection, id: &str, revision: Option<u32>) -> Result<OutgoingMessage, BridgeError> {
    let item = get(db, id)?.ok_or_else(|| BridgeError::Invalid("that draft no longer exists".into()))?;
    if item.state == OutgoingState::Discarded {
        return Err(BridgeError::Invalid("this draft was discarded".into()));
    }
    if revision.is_some_and(|revision| revision != item.revision) {
        return Err(BridgeError::Invalid("this draft changed since you opened it; reload it and try again".into()));
    }
    Ok(item)
}

/// The person's change: a new body, a Fix, or both. Their own words are
/// checked but not rewritten unless they ask for the fixes.
pub fn update(
    db: &Connection,
    id: &str,
    revision: u32,
    body: Option<&str>,
    apply_fixes: bool,
    attachment_paths: Option<&[String]>,
    now: &str,
) -> Result<OutgoingMessage, BridgeError> {
    let mut item = current(db, id, Some(revision))?;
    if body.is_none() && !apply_fixes && attachment_paths.is_none() {
        return Err(BridgeError::Invalid("nothing to change".into()));
    }
    if let Some(paths) = attachment_paths {
        item.attachments = attachments_for(&item.destination, paths)?;
    }
    let text = match body {
        Some(body) => check_body(body).map_err(BridgeError::Invalid)?,
        None => item.body.clone(),
    };
    let (text, lint) = if apply_fixes { fixed_text(&text, item.format) } else { (text.clone(), lint::check(&text, item.format)) };
    item.rendered = render::render(&text, item.format);
    item.body = text;
    item.lint = lint;
    item.state = OutgoingState::Draft;
    item.revision += 1;
    item.updated_at = now.to_owned();
    write(db, &item)?;
    Ok(item)
}

/// The text to put on the clipboard. Copying changes the state, not the
/// text, so the revision stays.
pub fn copy(db: &Connection, id: &str, revision: u32, now: &str) -> Result<(String, OutgoingMessage), BridgeError> {
    let mut item = current(db, id, Some(revision))?;
    item.state = OutgoingState::Copied;
    item.updated_at = now.to_owned();
    write(db, &item)?;
    Ok((item.rendered.clone(), item))
}

/// One attached image as `(mime, base64)`, checked again before it is read.
pub fn read_attachment(db: &Connection, id: &str, index: u32) -> Result<(String, String), BridgeError> {
    let item = get(db, id)?.ok_or_else(|| BridgeError::Invalid("that draft no longer exists".into()))?;
    let attachment = item
        .attachments
        .get(index as usize)
        .ok_or_else(|| BridgeError::Invalid("that draft has no such image".into()))?;
    attachments::read(attachment).map_err(BridgeError::Invalid)
}

/// Put a draft away. Discarding twice is not an error.
pub fn discard(db: &Connection, id: &str, now: &str) -> Result<OutgoingMessage, BridgeError> {
    let mut item = get(db, id)?.ok_or_else(|| BridgeError::Invalid("that draft no longer exists".into()))?;
    if item.state != OutgoingState::Discarded {
        item.state = OutgoingState::Discarded;
        item.updated_at = now.to_owned();
        write(db, &item)?;
    }
    Ok(item)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn store() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        install_store(&db).unwrap();
        db
    }

    fn slack() -> OutgoingDestination {
        OutgoingDestination::SlackChannel { channel: "#dev".into() }
    }

    #[test]
    fn destination_parses_every_accepted_shape_and_refuses_the_rest() {
        let cases = [
            (json!({"slack": {"channel": "#dev"}}), "Slack \u{b7} #dev"),
            (json!({"slack": {"channel": "#dev", "thread": "https://x.slack.com/p1"}}), "Slack \u{b7} #dev \u{b7} thread"),
            (json!({"slack": {"user": "@nina"}}), "Slack \u{b7} @nina"),
            (json!({"github": {"repo": "o/r", "number": 12}}), "GitHub \u{b7} o/r#12"),
            (json!({"github": {"repo": "o/r", "number": 12, "replyTo": 9}}), "GitHub \u{b7} o/r#12 \u{b7} reply"),
            (json!({"clipboard": "plain"}), "Clipboard \u{b7} plain text"),
        ];
        for (to, expected) in cases {
            assert_eq!(label(&parse_destination(&to).unwrap()), expected, "{to}");
        }
        for bad in [
            json!({"email": "a@b.c"}),
            json!({"slack": {"channel": "#a"}, "github": {"repo": "o/r", "number": 1}}),
            json!({"slack": {"channel": "#a", "user": "@b"}}),
            json!({"slack": {"thread": "t"}}),
            json!({"slack": {"channel": "  "}}),
            json!({"slack": {"channel": "#a", "ts": "1"}}),
            json!({"github": {"repo": "nope", "number": 1}}),
            json!({"github": {"repo": "o/r", "number": 0}}),
            json!({"clipboard": "html"}),
            json!("slack"),
        ] {
            assert!(parse_destination(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn agent_draft_applies_fixes_and_records_them() {
        let db = store();
        let item = create_draft(&db, "chat", slack(), "Deploy is done \u{2014} **all** green", "tell the team", &[], "t1").unwrap();
        assert_eq!(item.body, "Deploy is done, **all** green");
        // Slack's connector takes the Markdown as written.
        assert_eq!(item.rendered, "Deploy is done, **all** green");
        assert_eq!(item.lint.len(), 1);
        assert!(item.lint[0].fixed);
        assert_eq!((item.state, item.revision, item.format), (OutgoingState::Draft, 1, OutgoingFormat::SlackMarkdown));
        assert_eq!(get(&db, &item.id).unwrap().unwrap(), item);
    }

    #[test]
    fn stale_revision_is_refused_and_current_one_bumps() {
        let db = store();
        let item = create_draft(&db, "chat", slack(), "hello", "r", &[], "t1").unwrap();
        let edited = update(&db, &item.id, 1, Some("hello \u{2014} team"), false, None, "t2").unwrap();
        assert_eq!(edited.revision, 2);
        // The person's own words are checked, not rewritten.
        assert_eq!(edited.body, "hello \u{2014} team");
        assert!(edited.lint.iter().any(|finding| finding.fixable && !finding.fixed));
        assert!(update(&db, &item.id, 1, Some("again"), false, None, "t3").unwrap_err().to_string().contains("changed since"));
        let fixed = update(&db, &item.id, 2, None, true, None, "t4").unwrap();
        assert_eq!((fixed.body.as_str(), fixed.revision), ("hello, team", 3));
        assert!(update(&db, &item.id, 3, None, false, None, "t5").is_err(), "nothing to change");
        assert!(update(&db, &item.id, 3, Some("   "), false, None, "t5").is_err(), "empty");
    }

    #[test]
    fn copy_returns_rendered_text_and_marks_copied() {
        let db = store();
        // Pasted into Slack by hand, the text has to be mrkdwn already.
        let clipboard = OutgoingDestination::Clipboard { format: OutgoingFormat::Slack };
        let item = create_draft(&db, "chat", clipboard, "See [run](https://ci/1)", "r", &[], "t1").unwrap();
        let (text, copied) = copy(&db, &item.id, 1, "t2").unwrap();
        assert_eq!(text, "See <https://ci/1|run>");
        assert_eq!((copied.state, copied.revision), (OutgoingState::Copied, 1));
        assert!(copy(&db, &item.id, 7, "t3").is_err());
    }

    #[test]
    fn edit_after_copy_returns_to_draft() {
        let db = store();
        let item = create_draft(&db, "chat", slack(), "one", "r", &[], "t1").unwrap();
        copy(&db, &item.id, 1, "t2").unwrap();
        let edited = update(&db, &item.id, 1, Some("two"), false, None, "t3").unwrap();
        assert_eq!((edited.state, edited.revision), (OutgoingState::Draft, 2));
    }

    #[test]
    fn discarded_items_refuse_edits_and_copies() {
        let db = store();
        let item = create_draft(&db, "chat", slack(), "one", "r", &[], "t1").unwrap();
        assert_eq!(discard(&db, &item.id, "t2").unwrap().state, OutgoingState::Discarded);
        assert_eq!(discard(&db, &item.id, "t3").unwrap().updated_at, "t2", "idempotent");
        assert!(update(&db, &item.id, 1, Some("x"), false, None, "t4").unwrap_err().to_string().contains("discarded"));
        assert!(copy(&db, &item.id, 1, "t4").is_err());
    }

    #[test]
    fn images_attach_to_slack_and_clipboard_drafts_but_not_github() {
        let db = store();
        let dir = tempfile::tempdir().unwrap();
        let png = attachments::tests::write(dir.path(), "chart.png", attachments::tests::PNG);
        let item = create_draft(&db, "chat", slack(), "Here is the chart", "share it", &[png.clone()], "t1").unwrap();
        assert_eq!(item.attachments.len(), 1);
        assert_eq!(get(&db, &item.id).unwrap().unwrap().attachments, item.attachments);
        let (mime, _) = read_attachment(&db, &item.id, 0).unwrap();
        assert_eq!(mime, "image/png");
        assert!(read_attachment(&db, &item.id, 1).is_err());

        let github = OutgoingDestination::GithubComment { repo: "o/r".into(), number: 1 };
        let error = create_draft(&db, "chat", github, "see", "r", &[png], "t2").unwrap_err().to_string();
        assert!(error.contains("GitHub comments cannot carry"), "{error}");
        assert_eq!(count_for_session(&db, "chat").unwrap(), 1, "nothing stored for the refused draft");

        // Removing an image is an edit: a new revision with the shorter list.
        let removed = update(&db, &item.id, 1, None, false, Some(&[]), "t3").unwrap();
        assert_eq!((removed.attachments.len(), removed.revision), (0, 2));
    }

    #[test]
    fn list_is_per_chat_and_oldest_first() {
        let db = store();
        let first = create_draft(&db, "a", slack(), "1", "r", &[], "t1").unwrap();
        create_draft(&db, "b", slack(), "2", "r", &[], "t2").unwrap();
        let third = create_draft(&db, "a", slack(), "3", "r", &[], "t3").unwrap();
        discard(&db, &third.id, "t4").unwrap();
        let ids: Vec<String> = list(&db, "a").unwrap().into_iter().map(|item| item.id).collect();
        assert_eq!(ids, vec![first.id, third.id]);
        assert_eq!(count_for_session(&db, "a").unwrap(), 2);
    }
}
