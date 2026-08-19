//! What a chat is called.
//!
//! Every session used to read `Orchestrator`, because that is the label the
//! orchestrator is created with and nothing ever replaced it. A title is resolved
//! in preference order:
//!
//! 1. the harness's own title, where it keeps one — Claude Code writes a
//!    `custom-title` entry into its transcript once it has seen enough of the
//!    conversation to name it;
//! 2. a heading cut from the session's first user message.
//!
//! Codex keeps no title of its own. A full rollout file carries `session_meta`,
//! `event_msg`, `response_item`, `world_state` and `turn_context` entries and none
//! of them names the conversation, so step 2 is what titles a Codex chat — and it
//! is also what titles a Claude chat for the turn or two before Claude writes its
//! own.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::BridgeError;

/// Labels a session is born with. A session still wearing one has no real title.
pub const PLACEHOLDER_TITLES: [&str; 3] = ["Orchestrator", "Bridge orchestrator", "New chat"];

/// Longest heading worth keeping; past this a rail row truncates anyway.
const MAX_HEADING: usize = 60;

/// Openers that say nothing about a conversation. A chat that starts "hello" is
/// better left unnamed until it says something, because "Hello" as a heading is
/// worse than no heading at all in a list of forty.
const LOW_SIGNAL: [&str; 16] = [
    "hi", "hii", "hiu", "hey", "heya", "hello", "yo", "sup", "hola", "test", "testing", "ping",
    "thanks", "thank you", "who are you", "hello who are you",
];

/// Words that open a pleasantry rather than a request. Paired with a short
/// message they name nothing: "hey man", "thanks!", "ok cool".
const PLEASANTRY_OPENERS: [&str; 12] = [
    "hi", "hii", "hiu", "hey", "heya", "hello", "yo", "sup", "thanks", "ok", "okay", "cool",
];

/// True for a greeting, a redacted secret, a bare file mention, or a fragment too
/// short to name a conversation after.
pub fn is_low_signal(text: &str) -> bool {
    let trimmed = text.trim();
    // A secret the interceptor replaced is never a title. Neither is a lone
    // @mention, which names a file rather than the conversation.
    if trimmed.starts_with("[secret:") || (trimmed.starts_with('@') && !trimmed.contains(char::is_whitespace)) {
        return true;
    }
    let normalized: String = text
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if normalized.chars().count() < 6 {
        return true;
    }
    if LOW_SIGNAL.contains(&normalized.as_str()) {
        return true;
    }
    // A short message that opens with a pleasantry is one: "hey man", "ok cool".
    // Terse instructions ("fix migration") are kept, which is why the opener has
    // to match rather than the length alone.
    let mut words = normalized.split_whitespace();
    let opener = words.next().unwrap_or_default();
    let count = normalized.split_whitespace().count();
    count < 4 && PLEASANTRY_OPENERS.contains(&opener)
}

/// True while a session has nothing better than the label it was created with.
pub fn needs_title(title: Option<&str>) -> bool {
    match title.map(str::trim) {
        None | Some("") => true,
        Some(value) => PLACEHOLDER_TITLES
            .iter()
            .any(|placeholder| value.eq_ignore_ascii_case(placeholder)),
    }
}

/// Cuts a heading out of a user message: its first meaningful line, stripped of
/// markdown furniture and shortened on a word boundary.
pub fn heading_from_message(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(strip_furniture)
        .find(|line| !line.is_empty())?;

    let trimmed = shorten(&line);
    if trimmed.is_empty() {
        return None;
    }
    Some(capitalize(&trimmed))
}

/// Claude Code's own title for a session, read from its transcript.
///
/// The transcript is looked up by scanning the project directories rather than by
/// rebuilding Claude's slug for the cwd: the file is named after the provider
/// session id, which is unique, and the slug rules are Claude's to change.
pub fn claude_transcript_title(projects_dir: &Path, provider_session_id: &str) -> Option<String> {
    let transcript = find_transcript(projects_dir, provider_session_id)?;
    let contents = std::fs::read_to_string(transcript).ok()?;
    title_from_transcript(&contents)
}

fn find_transcript(projects_dir: &Path, provider_session_id: &str) -> Option<PathBuf> {
    let file_name = format!("{provider_session_id}.jsonl");
    let direct = projects_dir.join(&file_name);
    if direct.is_file() {
        return Some(direct);
    }
    for entry in std::fs::read_dir(projects_dir).ok()?.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let candidate = entry.path().join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Reads the last title Claude wrote. A session can be renamed, and the transcript
/// is append-only, so the last entry wins.
pub fn title_from_transcript(contents: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in contents.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let entry_type = value.get("type").and_then(Value::as_str);
        let candidate = match entry_type {
            Some("custom-title") => value.get("customTitle").and_then(Value::as_str),
            // Older transcripts name a conversation with a summary entry instead.
            Some("summary") => value.get("summary").and_then(Value::as_str),
            _ => None,
        };
        if let Some(candidate) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
            found = Some(shorten(candidate));
        }
    }
    found.filter(|value| !value.is_empty())
}

fn strip_furniture(line: &str) -> String {
    let mut value = line.trim();
    // Markdown headings, quotes and bullets say nothing about the topic.
    value = value.trim_start_matches(['#', '>', '-', '*', '+', ' ']);
    value = value.trim_start_matches('/');
    value = value.trim_matches(['`', '"', '\'', ' ']);
    value.to_owned()
}

fn shorten(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_HEADING {
        return collapsed;
    }
    // Prefer ending on a sentence, then on a word, and only then mid-word.
    let head: String = collapsed.chars().take(MAX_HEADING).collect();
    if let Some(stop) = head.rfind(['.', '?', '!']) {
        let sentence = head[..stop].trim();
        if sentence.chars().count() >= MAX_HEADING / 3 {
            return sentence.to_owned();
        }
    }
    match head.rfind(' ') {
        Some(space) if space >= MAX_HEADING / 3 => format!("{}…", head[..space].trim_end()),
        _ => format!("{}…", head.trim_end()),
    }
}

fn capitalize(value: &str) -> String {
    let lowered = value.to_lowercase();
    if lowered.starts_with("http://") || lowered.starts_with("https://") {
        return value.to_owned();
    }
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => first.to_uppercase().collect::<String>() + chars.as_str(),
        _ => value.to_owned(),
    }
}

/// Where Claude Code keeps its transcripts. Overridable so a test — or a machine
/// with a relocated home — does not have to own `$HOME`.
pub fn claude_projects_dir() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(explicit).join("projects"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude").join("projects"))
}

/// The message a heading is cut from when the harness supplies no title: the first
/// substantive thing the user said.
///
/// Greetings are skipped rather than titled — a rail full of "Hello" is no better
/// than a rail full of "Orchestrator". If the opening turns are *all* greetings the
/// session stays unnamed until it says something worth naming.
pub fn naming_message(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT payload FROM session_entries WHERE session_id=?1 AND kind='user.message' ORDER BY rowid LIMIT 8",
    )?;
    let mut rows = statement.query(params![session_id])?;
    while let Some(row) = rows.next()? {
        let payload: String = row.get(0)?;
        let Ok(value) = serde_json::from_str::<Value>(&payload) else {
            continue;
        };
        let text = value
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| value.get("message").and_then(Value::as_str));
        let Some(text) = text.map(str::trim).filter(|text| !text.is_empty()) else {
            continue;
        };
        if !is_low_signal(text) {
            return Ok(Some(text.to_owned()));
        }
    }
    Ok(None)
}

/// Gives a session a real title if it still wears a placeholder. Returns the title
/// it settled on, or None when there is nothing to go on yet.
///
/// Called when a turn completes: Claude needs a turn or two before it names a
/// conversation, and a session with no messages has nothing to name it after.
pub fn refresh(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    let row = db.query_row(
        "SELECT title,label,harness,provider_session_id FROM sessions WHERE id=?1",
        params![session_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    );
    let Ok((title, label, harness, provider_session_id)) = row else {
        return Ok(None);
    };
    // A title the user or the provider already set is not ours to overwrite.
    if !needs_title(title.as_deref().or(Some(label.as_str()))) {
        return Ok(None);
    }

    let provider = match (harness.as_str(), provider_session_id.as_deref()) {
        ("claude", Some(id)) => claude_projects_dir()
            .as_deref()
            .and_then(|dir| claude_transcript_title(dir, id)),
        // Codex keeps no title, and OpenCode's lives behind its HTTP API rather
        // than on disk, so both fall through to the first message.
        _ => None,
    };

    let resolved = match provider {
        Some(title) => Some(title),
        None => naming_message(db, session_id)?
            .as_deref()
            .and_then(heading_from_message),
    };

    let Some(resolved) = resolved else {
        return Ok(None);
    };
    db.execute(
        "UPDATE sessions SET title=?2 WHERE id=?1",
        params![session_id, resolved],
    )?;
    Ok(Some(resolved))
}

/// One-time catch-up for chats that predate titles: gives every placeholder
/// session a heading cut from its first message.
///
/// Deliberately does no file or network I/O. Opening the database must stay cheap
/// and side-effect free, and the harness's own title is picked up on that session's
/// next completed turn anyway.
pub fn backfill_from_messages(db: &Connection) -> Result<usize, BridgeError> {
    let mut untitled: Vec<String> = Vec::new();
    {
        let mut statement = db.prepare("SELECT id,title,label FROM sessions")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let label: String = row.get(2)?;
            if needs_title(title.as_deref().or(Some(label.as_str()))) {
                untitled.push(id);
            }
        }
    }

    let mut named = 0usize;
    for id in untitled {
        let Some(heading) = naming_message(db, &id)?
            .as_deref()
            .and_then(heading_from_message)
        else {
            continue;
        };
        db.execute(
            "UPDATE sessions SET title=?2 WHERE id=?1",
            params![id, heading],
        )?;
        named += 1;
    }
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Connection {
        let db = Connection::open_in_memory().expect("db");
        db.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, title TEXT, label TEXT, harness TEXT, provider_session_id TEXT);
             CREATE TABLE session_entries(rowid INTEGER PRIMARY KEY, session_id TEXT, kind TEXT, payload TEXT);",
        )
        .expect("schema");
        db
    }

    fn session(db: &Connection, id: &str, title: Option<&str>, harness: &str) {
        db.execute(
            "INSERT INTO sessions(id,title,label,harness,provider_session_id) VALUES(?1,?2,'Orchestrator',?3,NULL)",
            params![id, title, harness],
        )
        .expect("session");
    }

    fn said(db: &Connection, id: &str, text: &str) {
        db.execute(
            "INSERT INTO session_entries(session_id,kind,payload) VALUES(?1,'user.message',?2)",
            params![id, serde_json::json!({"text": text}).to_string()],
        )
        .expect("entry");
    }

    #[test]
    fn a_codex_session_is_titled_from_its_first_message() {
        let db = seeded();
        session(&db, "s1", None, "codex");
        said(&db, "s1", "fix the model profile migration");
        said(&db, "s1", "and then look at the rail");
        assert_eq!(
            refresh(&db, "s1").expect("refresh"),
            Some("Fix the model profile migration".into())
        );
        let stored: Option<String> = db
            .query_row("SELECT title FROM sessions WHERE id='s1'", [], |row| row.get(0))
            .expect("title");
        assert_eq!(stored.as_deref(), Some("Fix the model profile migration"));
    }

    #[test]
    fn a_greeting_does_not_name_a_chat() {
        for opener in [
            "hi", "Hello", "hey!", "  yo  ", "Who are you?", "thanks", "test",
            // Seen in the real database once this shipped:
            "Hey man", "Hey cutie", "ok cool", "@bun.lock",
            "[secret:sec_b7b4971ac7104f2daa64768e43f8ab82]",
        ] {
            assert!(is_low_signal(opener), "{opener} should be low signal");
        }
        for real in [
            "fix the rail grouping",
            "Who owns the worker lease?",
            "review PR 115",
            // Terse but a real instruction, so it stays.
            "fix migration",
            "What model are you?",
            "@bun.lock is out of date, refresh it",
        ] {
            assert!(!is_low_signal(real), "{real} should name a chat");
        }
    }

    #[test]
    fn a_chat_that_opens_with_a_greeting_is_named_by_its_next_message() {
        let db = seeded();
        session(&db, "s1", None, "claude");
        said(&db, "s1", "hello");
        said(&db, "s1", "can you fix the rail grouping");
        assert_eq!(
            refresh(&db, "s1").expect("refresh"),
            Some("Can you fix the rail grouping".into())
        );
    }

    #[test]
    fn a_chat_that_only_ever_greets_stays_unnamed() {
        let db = seeded();
        session(&db, "s1", None, "codex");
        said(&db, "s1", "hi");
        said(&db, "s1", "hello?");
        // Better an honest placeholder than a rail full of "Hello".
        assert_eq!(refresh(&db, "s1").expect("refresh"), None);
    }

    #[test]
    fn a_url_keeps_its_scheme_lowercase() {
        // Capitalising the first letter turned a pasted link into "Https://…".
        assert_eq!(
            heading_from_message("https://github.com/Atharva-Kanherkar/bridge-harness/issues/1"),
            Some("https://github.com/Atharva-Kanherkar/bridge-harness/issues/1".into())
        );
        // Still shortened when it is genuinely too long, scheme intact.
        let long = heading_from_message(
            "https://github.com/Atharva-Kanherkar/bridge-harness/pull/202/files#diff-abcdef",
        )
        .expect("heading");
        assert!(long.starts_with("https://"), "{long}");
        assert!(long.ends_with('…'), "{long}");
    }

    #[test]
    fn a_session_with_a_real_title_is_left_alone() {
        let db = seeded();
        session(&db, "s1", Some("Chosen by hand"), "codex");
        said(&db, "s1", "something else entirely");
        assert_eq!(refresh(&db, "s1").expect("refresh"), None);
    }

    #[test]
    fn a_session_that_has_said_nothing_yet_keeps_its_placeholder() {
        let db = seeded();
        session(&db, "s1", None, "codex");
        assert_eq!(refresh(&db, "s1").expect("refresh"), None);
    }

    #[test]
    fn an_empty_first_message_falls_through_to_the_next_one() {
        let db = seeded();
        session(&db, "s1", None, "codex");
        said(&db, "s1", "   ");
        said(&db, "s1", "review PR 115 adversarially");
        assert_eq!(
            refresh(&db, "s1").expect("refresh"),
            Some("Review PR 115 adversarially".into())
        );
    }

    #[test]
    fn the_backfill_names_old_chats_and_leaves_named_ones_alone() {
        let db = seeded();
        session(&db, "old-1", None, "codex");
        said(&db, "old-1", "freeze AxonHub output_format drop");
        session(&db, "old-2", Some("Orchestrator"), "opencode");
        said(&db, "old-2", "review the referral jobs page");
        session(&db, "named", Some("Chosen by hand"), "claude");
        said(&db, "named", "something else");
        // Nothing said yet, so nothing to name it after.
        session(&db, "silent", None, "codex");

        assert_eq!(backfill_from_messages(&db).expect("backfill"), 2);

        let titles: Vec<(String, Option<String>)> = db
            .prepare("SELECT id,title FROM sessions ORDER BY id")
            .expect("prepare")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .flatten()
            .collect();
        assert_eq!(
            titles,
            vec![
                ("named".to_string(), Some("Chosen by hand".to_string())),
                ("old-1".to_string(), Some("Freeze AxonHub output_format drop".to_string())),
                ("old-2".to_string(), Some("Review the referral jobs page".to_string())),
                ("silent".to_string(), None),
            ]
        );

        // Running twice renames nothing further.
        assert_eq!(backfill_from_messages(&db).expect("backfill"), 0);
    }

    #[test]
    fn an_unknown_session_is_not_an_error() {
        let db = seeded();
        assert_eq!(refresh(&db, "nope").expect("refresh"), None);
    }

    #[test]
    fn a_placeholder_label_still_needs_a_title() {
        assert!(needs_title(None));
        assert!(needs_title(Some("")));
        assert!(needs_title(Some("   ")));
        assert!(needs_title(Some("Orchestrator")));
        assert!(needs_title(Some("bridge orchestrator")));
        assert!(!needs_title(Some("Fix the model profile migration")));
    }

    #[test]
    fn a_heading_is_the_first_meaningful_line_capitalised() {
        assert_eq!(
            heading_from_message("fix the model profile migration"),
            Some("Fix the model profile migration".into())
        );
        assert_eq!(
            heading_from_message("\n\n  ## Review PR 115 adversarially\nmore detail below"),
            Some("Review PR 115 adversarially".into())
        );
        assert_eq!(
            heading_from_message("/review-checkpoint"),
            Some("Review-checkpoint".into())
        );
    }

    #[test]
    fn a_heading_collapses_whitespace_and_drops_quoting() {
        assert_eq!(
            heading_from_message("> `freeze   AxonHub output_format`"),
            Some("Freeze AxonHub output_format".into())
        );
    }

    #[test]
    fn a_long_message_is_cut_on_a_boundary() {
        let heading = heading_from_message(
            "Please look at the sidebar rail and tell me why every single chat in the list is labelled the same",
        )
        .expect("heading");
        assert!(heading.chars().count() <= MAX_HEADING + 1, "{heading}");
        assert!(heading.ends_with('…'), "{heading}");
        // Cut between words, never mid-word.
        assert!(!heading.trim_end_matches('…').ends_with(' '));
        assert!(heading.starts_with("Please look at the sidebar rail"));
    }

    #[test]
    fn a_long_first_sentence_ends_at_the_sentence() {
        let heading = heading_from_message("Fix the migration first. Then look at the rail grouping and the headers")
            .expect("heading");
        assert_eq!(heading, "Fix the migration first");
    }

    #[test]
    fn an_empty_message_has_no_heading() {
        assert_eq!(heading_from_message(""), None);
        assert_eq!(heading_from_message("\n\n   \n"), None);
        assert_eq!(heading_from_message("###"), None);
    }

    #[test]
    fn claudes_own_title_is_read_from_its_transcript() {
        let transcript = concat!(
            r#"{"type":"custom-title","customTitle":"Sidebar redesign","sessionId":"abc"}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user"}}"#,
            "\n",
        );
        assert_eq!(
            title_from_transcript(transcript),
            Some("Sidebar redesign".into())
        );
    }

    #[test]
    fn the_last_title_wins_because_a_chat_can_be_renamed() {
        let transcript = concat!(
            r#"{"type":"custom-title","customTitle":"First name"}"#,
            "\n",
            r#"{"type":"custom-title","customTitle":"Second name"}"#,
            "\n",
        );
        assert_eq!(title_from_transcript(transcript), Some("Second name".into()));
    }

    #[test]
    fn a_summary_entry_names_an_older_transcript() {
        let transcript = r#"{"type":"summary","summary":"Worktree isolation review","leafUuid":"x"}"#;
        assert_eq!(
            title_from_transcript(transcript),
            Some("Worktree isolation review".into())
        );
    }

    #[test]
    fn a_transcript_with_no_title_yields_none() {
        // Claude writes its title a turn or two in; before that there is nothing.
        let transcript = concat!(
            r#"{"type":"queue-operation","op":"append"}"#,
            "\n",
            r#"{"type":"user","message":{"role":"user"}}"#,
            "\n",
            "not json at all\n",
        );
        assert_eq!(title_from_transcript(transcript), None);
    }

    /// Reads the developer's real Claude directory. Ignored by default — it depends
    /// on a machine's own history — but it is what proves the parser matches what
    /// Claude Code actually writes, rather than what this file assumes it writes.
    /// `cargo test -p bridge-core session_titles::tests::real -- --ignored --nocapture`
    #[test]
    #[ignore = "reads the developer's ~/.claude directory"]
    fn real_claude_transcripts_yield_titles() {
        let dir = claude_projects_dir().expect("home");
        let mut named = 0usize;
        let mut total = 0usize;
        for project in std::fs::read_dir(&dir).expect("projects").flatten() {
            if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            for entry in std::fs::read_dir(project.path()).expect("transcripts").flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "jsonl") {
                    continue;
                }
                total += 1;
                let id = path.file_stem().unwrap().to_string_lossy().to_string();
                if let Some(title) = claude_transcript_title(&dir, &id) {
                    named += 1;
                    println!("{id:<40} {title}");
                    assert!(!title.is_empty());
                    assert!(title.chars().count() <= MAX_HEADING + 1);
                }
            }
        }
        println!("{named} of {total} transcripts carry a title");
        assert!(total > 0, "no transcripts to read");
    }

    #[test]
    fn a_transcript_is_found_by_session_id_under_any_project() {
        let root = std::env::temp_dir().join(format!("bridge-titles-{}", std::process::id()));
        let project = root.join("-Users-atharva-Documents-harness");
        std::fs::create_dir_all(&project).expect("project dir");
        std::fs::write(
            project.join("session-42.jsonl"),
            r#"{"type":"custom-title","customTitle":"Found by id"}"#,
        )
        .expect("transcript");

        assert_eq!(
            claude_transcript_title(&root, "session-42"),
            Some("Found by id".into())
        );
        assert_eq!(claude_transcript_title(&root, "missing"), None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
