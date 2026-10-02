//! What a chat is called.
//!
//! Every session used to read `Orchestrator`, because that is the label the
//! orchestrator is created with and nothing ever replaced it. A title is resolved
//! in preference order:
//!
//! 1. the harness's own title, where it keeps one — Claude Code writes a
//!    `custom-title` entry into its transcript once it has seen enough of the
//!    conversation to name it;
//! 2. a short topic label extracted from the first substantive user message.
//!
//! Automatic names contain at most three words. User-chosen names are preserved.
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
const MAX_TITLE_WORDS: usize = 3;

/// Request scaffolding is not a topic: "can you please fix the" should never
/// occupy all three words of a chat's name. This is a local fallback, not an
/// additional model turn; an available harness title still takes precedence.
const TITLE_FILLER: &[&str] = &[
    "a", "an", "the", "i", "i'm", "im", "we", "you", "me", "my", "our", "your",
    "can", "could", "would", "will", "should", "please", "want", "wanted", "need",
    "like", "help", "to", "with", "in", "on", "at", "of", "for", "from", "by",
    "and", "or", "but", "so", "that", "this", "it", "its", "is", "are", "was",
    "be", "been", "have", "has", "do", "does", "some", "all", "just", "also",
    "fix", "review", "implement", "build", "add", "update", "make", "look", "check",
    "tell", "explain", "why", "how", "what", "first", "then", "every", "single",
];

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

/// What a pasted link is about. A raw URL fills a whole heading and names nothing
/// a reader recognises, so `github.com/o/kairo/pull/43` becomes `kairo PR #43`
/// (complete on its own), a repository its name, and any other link its host.
fn link_heading(word: &str) -> Option<(String, bool)> {
    // a link in prose often carries its brackets or the sentence's punctuation.
    let word = word
        .trim_start_matches(['(', '<', '[', '"', '\''])
        .trim_end_matches([')', '>', ']', '"', '\'', ',', '.', ';', ':', '!', '?']);
    let rest = word.strip_prefix("https://").or_else(|| word.strip_prefix("http://"))?;
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    // the query and fragment say nothing about what the page is.
    let path = rest.split(['?', '#']).next().unwrap_or(rest);
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let host = parts.next()?;
    if !host.eq_ignore_ascii_case("github.com") {
        // the last path segment usually says what the page is; an opaque id does not.
        let opaque = |tail: &str| tail.len() >= 8 && tail.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
        let tail = parts.last().filter(|tail| tail.len() <= 32 && tail.chars().any(char::is_alphabetic) && !tail.contains('=') && !opaque(tail));
        return Some((tail.map_or_else(|| host.to_owned(), |tail| format!("{host} {tail}")), false));
    }
    let (Some(_owner), Some(repo)) = (parts.next(), parts.next()) else {
        return Some((host.to_owned(), false));
    };
    let repo = repo.trim_end_matches(".git");
    let kind = parts.next().map(str::to_ascii_lowercase);
    let number = parts.next().filter(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    Some(match (kind.as_deref(), number) {
        (Some("pull" | "pulls"), Some(number)) => (format!("{repo} PR #{number}"), true),
        (Some("issues"), Some(number)) => (format!("{repo} issue #{number}"), true),
        _ => (repo.to_owned(), false),
    })
}

/// Extracts up to three topic words, excluding conversational request scaffolding.
pub fn heading_from_message(text: &str) -> Option<String> {
    let line = text
        .lines()
        .map(strip_furniture)
        .find(|line| !line.is_empty())?;

    // A link names itself by what it points at; identifiers keep their case.
    let (first, rest) = line.split_once(char::is_whitespace).unwrap_or((line.as_str(), ""));
    if let Some((link, complete)) = link_heading(first) {
        if complete {
            return Some(link);
        }
        let topic = std::iter::once(link).chain(topic_words(rest)).take(MAX_TITLE_WORDS).collect::<Vec<_>>().join(" ");
        return Some(shorten(&topic));
    }

    let topic = topic_words(&line).into_iter().take(MAX_TITLE_WORDS).collect::<Vec<_>>().join(" ");
    let trimmed = shorten(&topic);
    if trimmed.is_empty() {
        return None;
    }
    Some(capitalize(&trimmed))
}

/// Topic words of a line with request scaffolding removed.
fn topic_words(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|word| word.trim_matches(|c: char| !c.is_alphanumeric() && c != '/' && c != '_' && c != '-'))
        .filter(|word| !word.is_empty() && !TITLE_FILLER.contains(&word.to_lowercase().as_str()))
        .map(str::to_owned)
        .collect()
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
    let collapsed = value.split_whitespace().take(MAX_TITLE_WORDS).collect::<Vec<_>>().join(" ");
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

/// Where a stored title came from. Only a title Bridge derived is ever replaced:
/// the harness's own, and one the user chose, are final.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    /// Cut from the conversation by [`heading_from_message`].
    Derived,
    /// Read back from the harness.
    Provider,
}

impl TitleSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TitleSource::Derived => "derived",
            TitleSource::Provider => "provider",
        }
    }

    fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            Some("derived") => Some(TitleSource::Derived),
            Some("provider") => Some(TitleSource::Provider),
            _ => None,
        }
    }
}

/// Everything a title decision needs, read in one cheap pass so the caller can let
/// go of the database before doing any provider I/O.
#[derive(Debug, Clone)]
pub struct TitlePlan {
    pub harness: String,
    pub provider_session_id: Option<String>,
    /// The message a heading would be cut from, when the session is still unnamed.
    pub naming_message: Option<String>,
    /// True when the only thing stored is a heading Bridge derived itself.
    pub replaceable: bool,
}

/// Reads what is needed to decide a title. Returns None when there is nothing to
/// do: the session is gone, or its title is the provider's or the user's.
///
/// Cheap by construction — one row plus at most eight entries — because callers
/// hold a process-wide lock while they run it.
pub fn plan(db: &Connection, session_id: &str) -> Result<Option<TitlePlan>, BridgeError> {
    let row = db.query_row(
        "SELECT title,label,harness,provider_session_id,title_source FROM sessions WHERE id=?1",
        params![session_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        },
    );
    let Ok((title, label, harness, provider_session_id, source)) = row else {
        return Ok(None);
    };

    let unnamed = needs_title(title.as_deref().or(Some(label.as_str())));
    let source = TitleSource::parse(source.as_deref());
    match (unnamed, source) {
        // Named by the harness, or by the user before this column existed.
        (false, Some(TitleSource::Provider) | None) => return Ok(None),
        // A heading we cut ourselves; the harness may have named it since.
        (false, Some(TitleSource::Derived)) => {
            return Ok(Some(TitlePlan {
                harness,
                provider_session_id,
                naming_message: naming_message(db, session_id)?,
                replaceable: true,
            }))
        }
        _ => {}
    }

    Ok(Some(TitlePlan {
        harness,
        provider_session_id,
        naming_message: naming_message(db, session_id)?,
        replaceable: false,
    }))
}

/// Settles on a title. Takes no database handle on purpose: this is the half that
/// walks Claude's project directories and reads a transcript, and it must not run
/// while the caller holds the database.
pub fn resolve(plan: &TitlePlan) -> Option<(String, TitleSource)> {
    let provider = match (plan.harness.as_str(), plan.provider_session_id.as_deref()) {
        ("claude", Some(id)) => claude_projects_dir()
            .as_deref()
            .and_then(|dir| claude_transcript_title(dir, id)),
        // Codex keeps no title, and OpenCode's lives behind its HTTP API rather
        // than on disk, so both fall through to the conversation.
        _ => None,
    };
    if let Some(provider) = provider {
        return Some((provider, TitleSource::Provider));
    }
    // Recompute derived names too, so old sentence-length titles can be repaired.
    plan.naming_message
        .as_deref()
        .and_then(heading_from_message)
        .map(|heading| (heading, TitleSource::Derived))
}

/// Writes a settled title. Cheap, so it is safe to hold the database across it.
pub fn commit(
    db: &Connection,
    session_id: &str,
    title: &str,
    source: TitleSource,
) -> Result<(), BridgeError> {
    let title = shorten(title);
    db.execute(
        "UPDATE sessions SET title=?2,title_source=?3 WHERE id=?1 AND (title IS NOT ?2 OR title_source IS NOT ?3)",
        params![session_id, title, source.as_str()],
    )?;
    Ok(())
}

/// Plan, resolve and commit against one connection. Convenient for tests and for
/// callers that are not holding a shared lock; the live turn path runs the three
/// phases itself so the database is free during the provider read.
pub fn refresh(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    let Some(plan) = plan(db, session_id)? else {
        return Ok(None);
    };
    let Some((title, source)) = resolve(&plan) else {
        return Ok(None);
    };
    commit(db, session_id, &title, source)?;
    Ok(Some(title))
}

/// Catch up placeholders and old automatic names to the concise-title policy.
/// Explicit user titles (no recorded automatic source) are never rewritten.
///
/// Deliberately does no file or network I/O. Opening the database must stay cheap
/// and side-effect free, and the harness's own title is picked up on that session's
/// next completed turn anyway.
pub fn backfill_from_messages(db: &Connection) -> Result<usize, BridgeError> {
    let mut untitled: Vec<(String, Option<String>, bool)> = Vec::new();
    let mut provider_titles = Vec::new();
    {
        let mut statement = db.prepare("SELECT id,title,label,title_source FROM sessions")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let label: String = row.get(2)?;
            let source: Option<String> = row.get(3)?;
            if needs_title(title.as_deref().or(Some(label.as_str()))) || source.as_deref() == Some("derived") {
                // An old derived name can still be shortened if its original
                // message has been pruned or carries no usable topic words.
                untitled.push((id, title, source.as_deref() == Some("derived")));
            } else if source.as_deref() == Some("provider") {
                if let Some(title) = title {
                    let concise = shorten(&title);
                    if concise != title { provider_titles.push((id, concise)); }
                }
            }
        }
    }

    let mut named = 0usize;
    for (id, previous, derived) in untitled {
        let Some(heading) = naming_message(db, &id)?
            .as_deref()
            .and_then(heading_from_message)
            .or_else(|| derived.then(|| previous.as_deref().map(shorten)).flatten())
        else {
            continue;
        };
        if previous.as_deref() == Some(heading.as_str()) { continue; }
        commit(db, &id, &heading, TitleSource::Derived)?;
        named += 1;
    }
    for (id, title) in provider_titles {
        commit(db, &id, &title, TitleSource::Provider)?;
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
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, title TEXT, label TEXT, harness TEXT, provider_session_id TEXT, title_source TEXT);
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
            Some("Model profile migration".into())
        );
        let stored: Option<String> = db
            .query_row("SELECT title FROM sessions WHERE id='s1'", [], |row| row.get(0))
            .expect("title");
        assert_eq!(stored.as_deref(), Some("Model profile migration"));
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
            Some("Rail grouping".into())
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
    fn a_link_heading_is_never_capitalised() {
        // Capitalising the first letter once turned a pasted link into "Https://…";
        // repository names are identifiers, so they keep their case too.
        assert_eq!(
            heading_from_message("https://github.com/Atharva-Kanherkar/bridge-harness/issues/1"),
            Some("bridge-harness issue #1".into())
        );
        assert_eq!(
            heading_from_message("https://github.com/Atharva-Kanherkar/bridge-harness/pull/202/files#diff-abcdef"),
            Some("bridge-harness PR #202".into())
        );
        assert_eq!(heading_from_message("http://localhost:1420/ blank page"), Some("localhost:1420 blank page".into()));
    }

    #[test]
    fn claudes_own_title_replaces_a_heading_bridge_derived() {
        // The provider names a conversation a turn or two in; a heading cut from
        // the first message is a stand-in until then, not the final answer.
        let db = seeded();
        db.execute(
            "INSERT INTO sessions(id,title,label,harness,provider_session_id,title_source) VALUES('s1','Can you fix the rail grouping','Orchestrator','claude','prov-1','derived')",
            [],
        )
        .expect("session");
        let pending = plan(&db, "s1").expect("plan").expect("still replaceable");
        assert!(pending.replaceable);
        assert_eq!(pending.provider_session_id.as_deref(), Some("prov-1"));

        // With a provider title available, it wins.
        let named = resolve(&TitlePlan { harness: "claude".into(), provider_session_id: None, naming_message: None, replaceable: true });
        assert_eq!(named, None, "no provider title and nothing new to derive");
        commit(&db, "s1", "Sidebar redesign", TitleSource::Provider).expect("commit");

        // And once it is the provider's, it is final.
        assert!(plan(&db, "s1").expect("plan").is_none());
    }

    #[test]
    fn a_title_from_before_this_column_is_treated_as_the_users() {
        let db = seeded();
        db.execute(
            "INSERT INTO sessions(id,title,label,harness,provider_session_id,title_source) VALUES('s1','Chosen by hand','Orchestrator','claude','prov-1',NULL)",
            [],
        )
        .expect("session");
        assert!(plan(&db, "s1").expect("plan").is_none());
    }

    #[test]
    fn an_old_derived_title_can_be_replaced_by_a_concise_topic() {
        let pending = TitlePlan {
            harness: "codex".into(),
            provider_session_id: Some("prov-1".into()),
            naming_message: Some("fix the rail grouping".into()),
            replaceable: true,
        };
        assert_eq!(resolve(&pending), Some(("Rail grouping".into(), TitleSource::Derived)));
    }

    #[test]
    fn the_backfill_stamps_what_it_derived() {
        let db = seeded();
        session(&db, "s1", None, "codex");
        said(&db, "s1", "freeze AxonHub output_format drop");
        assert_eq!(backfill_from_messages(&db).expect("backfill"), 1);
        let source: Option<String> = db
            .query_row("SELECT title_source FROM sessions WHERE id='s1'", [], |row| row.get(0))
            .expect("source");
        assert_eq!(source.as_deref(), Some("derived"));
        // Stamped derived, so the harness can still improve on it.
        assert!(plan(&db, "s1").expect("plan").expect("replaceable").replaceable);
    }

    #[test]
    fn a_session_with_a_real_title_is_left_alone() {
        let db = seeded();
        session(&db, "s1", Some("Chosen by hand"), "codex");
        said(&db, "s1", "something else entirely");
        assert_eq!(refresh(&db, "s1").expect("refresh"), None);
    }

    #[test]
    fn automatic_titles_are_one_to_three_words() {
        for message in [
            "Can you please fix the Mission Control dragging behavior?",
            "I would like you to add persistent chat pinning",
            "Please implement OAuth callback validation and error handling",
            "检查 中文 会话 标题 长度",
        ] {
            let title = heading_from_message(message).unwrap();
            assert!((1..=3).contains(&title.split_whitespace().count()), "{title}");
        }
        assert_eq!(heading_from_message("Can you please fix the Mission Control dragging behavior?"), Some("Mission Control dragging".into()));
        assert_eq!(heading_from_message("I would like you to add persistent chat pinning"), Some("Persistent chat pinning".into()));
    }

    #[test]
    fn pasted_links_are_named_by_what_they_point_at() {
        assert_eq!(heading_from_message("https://github.com/Atharva-Kanherkar/kairo/pull/43 reviewe this please"), Some("kairo PR #43".into()));
        assert_eq!(heading_from_message("https://github.com/Atharva-Kanherkar/kairo/pull/27#issuecomment-1 fix"), Some("kairo PR #27".into()));
        assert_eq!(heading_from_message("https://github.com/org/bridge-harness/issues/12"), Some("bridge-harness issue #12".into()));
        assert_eq!(heading_from_message("https://GitHub.com/org/kairo/PULL/9"), Some("kairo PR #9".into()));
        assert_eq!(heading_from_message("https://github.com/org fix the thing"), Some("github.com thing".into()));
        assert_eq!(heading_from_message("https://github.com/Atharva-Kanherkar/kairo Read the repo"), Some("kairo Read repo".into()));
        assert_eq!(heading_from_message("https://github.com/Atharva-Kanherkar/kairo.git"), Some("kairo".into()));
        assert_eq!(heading_from_message("https://www.vercel.com/team/deployments are failing"), Some("vercel.com deployments failing".into()));
        assert_eq!(heading_from_message("https://example.com/runs/8f3a9c2e7d1b4a6f9e0c3b5d7a1f2e4c broke"), Some("example.com broke".into()));
        // punctuation and brackets around the link, and a query or fragment on it.
        assert_eq!(heading_from_message("https://github.com/o/kairo/pull/43, is it safe?"), Some("kairo PR #43".into()));
        assert_eq!(heading_from_message("https://github.com/o/kairo/pull/43."), Some("kairo PR #43".into()));
        assert_eq!(heading_from_message("(https://github.com/o/kairo/issues/7) keeps failing"), Some("kairo issue #7".into()));
        assert_eq!(heading_from_message("<https://github.com/o/kairo>"), Some("kairo".into()));
        assert_eq!(heading_from_message("https://docs.rs/serde/latest/serde/#derive"), Some("docs.rs serde".into()));
        assert_eq!(heading_from_message("https://vercel.com/team/deployments?tab=logs"), Some("vercel.com deployments".into()));
        // a link later in the message is just a word; the topic still leads.
        assert_eq!(heading_from_message("Mission Control drag https://example.com"), Some("Mission Control drag".into()));
    }

    #[test]
    fn provider_titles_are_bounded_and_still_preferred() {
        assert_eq!(title_from_transcript(r#"{"type":"custom-title","customTitle":"Mission Control drag and drop fixes"}"#), Some("Mission Control drag".into()));
        let db = seeded();
        session(&db, "s1", None, "claude");
        commit(&db, "s1", "Mission Control drag and drop fixes", TitleSource::Provider).unwrap();
        assert!(plan(&db, "s1").unwrap().is_none());
        let title: String = db.query_row("SELECT title FROM sessions WHERE id='s1'", [], |row| row.get(0)).unwrap();
        assert_eq!(title, "Mission Control drag");
    }

    #[test]
    fn backfill_repairs_automatic_names_but_preserves_user_names_and_is_idempotent() {
        let db = seeded();
        session(&db, "derived", Some("Can you please fix the Mission Control dragging behavior?"), "codex");
        said(&db, "derived", "Can you please fix the Mission Control dragging behavior?");
        session(&db, "provider", Some("Mission Control drag and drop fixes"), "claude");
        session(&db, "manual", Some("My deliberately long personal chat name"), "codex");
        db.execute("UPDATE sessions SET title_source='derived' WHERE id='derived'", []).unwrap();
        db.execute("UPDATE sessions SET title_source='provider' WHERE id='provider'", []).unwrap();
        assert_eq!(backfill_from_messages(&db).unwrap(), 2);
        let title = |id: &str| db.query_row("SELECT title FROM sessions WHERE id=?1", [id], |row| row.get::<_, String>(0)).unwrap();
        assert_eq!(title("derived"), "Mission Control dragging");
        assert_eq!(title("provider"), "Mission Control drag");
        assert_eq!(title("manual"), "My deliberately long personal chat name");
        assert_eq!(backfill_from_messages(&db).unwrap(), 0);
    }

    #[test]
    fn backfill_shortens_a_derived_title_even_without_its_original_message() {
        let db = seeded();
        session(&db, "s1", Some("Mission Control drag and drop"), "codex");
        db.execute("UPDATE sessions SET title_source='derived' WHERE id='s1'", []).unwrap();
        assert_eq!(backfill_from_messages(&db).unwrap(), 1);
        let title: String = db.query_row("SELECT title FROM sessions WHERE id='s1'", [], |row| row.get(0)).unwrap();
        assert_eq!(title, "Mission Control drag");
        assert_eq!(backfill_from_messages(&db).unwrap(), 0);
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
            Some("PR 115 adversarially".into())
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
                ("old-1".to_string(), Some("Freeze AxonHub output_format".to_string())),
                ("old-2".to_string(), Some("Referral jobs page".to_string())),
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
            Some("Model profile migration".into())
        );
        assert_eq!(
            heading_from_message("\n\n  ## Review PR 115 adversarially\nmore detail below"),
            Some("PR 115 adversarially".into())
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
    fn a_long_request_becomes_three_topic_words_instead_of_a_prompt_prefix() {
        let heading = heading_from_message(
            "Please look at the sidebar rail and tell me why every single chat in the list is labelled the same",
        )
        .expect("heading");
        assert_eq!(heading, "Sidebar rail chat");
        assert_eq!(heading.split_whitespace().count(), 3);
    }

    #[test]
    fn a_multiple_sentence_request_still_has_at_most_three_topic_words() {
        let heading = heading_from_message("Fix the migration first. Then look at the rail grouping and the headers")
            .expect("heading");
        assert_eq!(heading, "Migration rail grouping");
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
