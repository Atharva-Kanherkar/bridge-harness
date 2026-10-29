//! What a chat is called.
//!
//! Every session used to read `Orchestrator`, because that is the label the
//! orchestrator is created with and nothing ever replaced it. A title is resolved
//! in preference order:
//!
//! 1. a name the user chose;
//! 2. the harness's own title, where it keeps one — Claude Code writes an
//!    `ai-title` entry into its transcript once it has seen the first message,
//!    and a `custom-title` one when the user renames the session;
//! 3. a title Bridge generates on the chat's own harness ([`crate::title_jobs`]);
//! 4. a short topic label extracted from the first substantive user message.
//!
//! A write never lowers that rank (see [`TitleSource::rank`]). Headings cut
//! locally contain at most three words; a harness or model title keeps up to six,
//! because cutting "Paper and slate launch video" at three leaves "Paper and slate".
//!
//! Codex keeps no title of its own. A full rollout file carries `session_meta`,
//! `event_msg`, `response_item`, `world_state` and `turn_context` entries and none
//! of them names the conversation, so steps 3 and 4 are what title a Codex chat.
//! OpenCode does name sessions, but on 2.1.1 its titler answered the prompt
//! ("SQLite's WAL keeps growing when long-lived read…") rather than naming it, so
//! Bridge keeps creating OpenCode sessions with a fixed title and generates its own.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::BridgeError;

/// Labels a session is born with. A session still wearing one has no real title.
/// `Chat` is what a direct chat is born with (`sessions.rs`, `chat_defaults.rs`).
pub const PLACEHOLDER_TITLES: [&str; 4] = ["Orchestrator", "Bridge orchestrator", "New chat", "Chat"];

/// Longest heading worth keeping; past this a rail row truncates anyway.
pub const MAX_HEADING: usize = 60;
const MAX_TITLE_WORDS: usize = 3;
/// A harness or model title is a phrase, not a bag of topic words.
pub const MAX_NAMED_WORDS: usize = 6;

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
/// The transcript is looked up through the slug Claude derives from the cwd
/// first, and only then by scanning every project directory: the file is named
/// after the provider session id, which is unique, but a real machine has tens
/// of thousands of project directories and the slug rules are Claude's to change.
pub fn claude_transcript_title(
    projects_dir: &Path,
    provider_session_id: &str,
    cwd: Option<&str>,
) -> Option<HarnessTitle> {
    let transcript = find_transcript(projects_dir, provider_session_id, cwd)?;
    title_from_transcript_file(&transcript)
}

/// Claude's project directory name for a cwd: every non-alphanumeric byte
/// becomes `-`. Very long paths are shortened by Claude, which the scan covers.
fn claude_project_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn find_transcript(projects_dir: &Path, provider_session_id: &str, cwd: Option<&str>) -> Option<PathBuf> {
    let file_name = format!("{provider_session_id}.jsonl");
    let direct = projects_dir.join(&file_name);
    if direct.is_file() {
        return Some(direct);
    }
    if let Some(cwd) = cwd.filter(|cwd| !cwd.is_empty()) {
        let slugged = projects_dir.join(claude_project_slug(cwd)).join(&file_name);
        if slugged.is_file() {
            return Some(slugged);
        }
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

/// Claude re-appends its title entries as a session goes on, so the newest one
/// sits near the end of the file (within 32 KiB on every transcript measured, up
/// to 117 MB). The tail is read first; the whole file only when the tail names
/// nothing and the file is small enough to stream on a turn boundary.
const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;
const TRANSCRIPT_FULL_SCAN_LIMIT: u64 = 32 * 1024 * 1024;

fn title_from_transcript_file(path: &Path) -> Option<HarnessTitle> {
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let offset = size.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let tail = String::from_utf8_lossy(&tail);
    // a tail that starts mid-file starts mid-line.
    let tail = if offset > 0 { tail.split_once('\n').map_or("", |(_, rest)| rest) } else { &tail };
    let mut titles = TranscriptTitles::default();
    tail.lines().for_each(|line| titles.observe(line));
    if titles.best().is_some() || offset == 0 || size > TRANSCRIPT_FULL_SCAN_LIMIT {
        return titles.best();
    }
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut titles = TranscriptTitles::default();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        titles.observe(&line);
    }
    titles.best()
}

/// Transcript entry types that name a session, in [`TranscriptTitles`] slot order.
const TITLE_ENTRY_TYPES: [&[u8]; 3] = [
    br#""type":"custom-title""#,
    br#""type":"ai-title""#,
    br#""type":"summary""#,
];

/// The last title of each kind a transcript carries.
#[derive(Default)]
struct TranscriptTitles {
    custom: Option<String>,
    ai: Option<String>,
    summary: Option<String>,
}

impl TranscriptTitles {
    fn observe(&mut self, line: &str) {
        // Only title entries are parsed; a transcript is mostly tool output, and
        // its type field leads every line Claude writes.
        let head = &line.as_bytes()[..line.len().min(48)];
        let kind = TITLE_ENTRY_TYPES.iter().position(|needle| contains(head, needle));
        let Some(slot) = kind else { return };
        let Ok(value) = serde_json::from_str::<Value>(line) else { return };
        let (field, target) = match slot {
            0 => ("customTitle", &mut self.custom),
            1 => ("aiTitle", &mut self.ai),
            _ => ("summary", &mut self.summary),
        };
        if let Some(title) = value.get(field).and_then(Value::as_str).map(str::trim).filter(|title| !title.is_empty()) {
            *target = Some(title.to_owned());
        }
    }

    /// A name the user gave the session beats one Claude generated, which beats
    /// the summary older transcripts carried. Within a kind the last one wins,
    /// because the transcript is append-only and a session can be renamed.
    fn best(&self) -> Option<HarnessTitle> {
        let (title, chosen) = match (&self.custom, &self.ai, &self.summary) {
            (Some(custom), _, _) => (custom, true),
            (None, Some(ai), _) => (ai, false),
            (None, None, Some(summary)) => (summary, false),
            _ => return None,
        };
        let title = bound(title, MAX_NAMED_WORDS);
        (!title.is_empty()).then_some(HarnessTitle { title, chosen })
    }
}

/// A title read from the harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessTitle {
    pub title: String,
    /// The user named the session in the harness (`/rename`), rather than the
    /// harness naming it.
    pub chosen: bool,
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Reads Claude's title from transcript text. See [`TranscriptTitles::best`].
pub fn title_from_transcript(contents: &str) -> Option<String> {
    let mut titles = TranscriptTitles::default();
    contents.lines().for_each(|line| titles.observe(line));
    titles.best().map(|best| best.title)
}

/// Words that say nothing about which chat this is, on top of [`TITLE_FILLER`].
/// Claude titles a Bridge chat "Continue work" or "Session start" when what it
/// saw last was a resume or a greeting.
const GENERIC_TITLE_WORDS: &[&str] = &[
    "bridge", "session", "sessions", "continue", "continued", "continuing", "work", "start", "started",
    "chat", "conversation", "new", "task", "request", "greeting", "hello",
];

/// Topic words of a title or a message, for [`grounded`]: alphanumeric runs of
/// three or more characters, or any run with a digit (a PR number), minus filler,
/// cut to a five-character stem so "review" meets "reviewing".
fn topic_stems(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 3 || word.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .filter(|word| !TITLE_FILLER.contains(&word.as_str()) && !GENERIC_TITLE_WORDS.contains(&word.as_str()))
        .map(|word| word.chars().take(5).collect())
}

/// Does a harness title share a topic word with what the person actually said?
///
/// Claude names a session from everything it was sent, and Bridge sends more than
/// the person typed: on a real store a chat about Dynamo bugs was titled "Bridge
/// session capabilities" (from the session context Bridge prepends), a PR review
/// "Bridge credential proxy verification", and real tasks "Continue work". A
/// title with nothing in common with the person's own words is not used.
pub fn grounded(title: &str, said: &str) -> bool {
    let said: std::collections::HashSet<String> = topic_stems(said).collect();
    topic_stems(title).any(|stem| said.contains(&stem))
}

/// What the person said, to ground a harness title in: their first and latest
/// few messages, which is where a chat's topic and its current subject live.
pub fn grounding_text(db: &Connection, session_id: &str) -> Result<String, BridgeError> {
    let mut said = String::new();
    for order in ["ASC", "DESC"] {
        let mut statement = db.prepare(&format!(
            "SELECT payload FROM session_entries WHERE session_id=?1 AND kind='user.message' ORDER BY rowid {order} LIMIT 6"
        ))?;
        let mut rows = statement.query(params![session_id])?;
        while let Some(row) = rows.next()? {
            let payload: String = row.get(0)?;
            let Ok(value) = serde_json::from_str::<Value>(&payload) else { continue };
            if let Some(text) = value.get("text").and_then(Value::as_str) {
                said.extend(text.chars().take(2_000));
                said.push('\n');
            }
        }
    }
    Ok(said)
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
    bound(value, MAX_TITLE_WORDS)
}

/// At most `words` words and [`MAX_HEADING`] characters.
pub fn bound(value: &str, words: usize) -> String {
    let collapsed = value.split_whitespace().take(words).collect::<Vec<_>>().join(" ");
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

/// Where a stored title came from. A user's title has no source: it is NULL,
/// as is every title from before this column existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleSource {
    /// Cut from the conversation by [`heading_from_message`].
    Derived,
    /// Generated on the chat's own harness by [`crate::title_jobs`].
    Generated,
    /// Read back from the harness.
    Provider,
}

/// A user's title, which nothing automatic replaces.
pub const USER_RANK: u8 = 4;

impl TitleSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TitleSource::Derived => "derived",
            TitleSource::Generated => "generated",
            TitleSource::Provider => "provider",
        }
    }

    fn parse(value: Option<&str>) -> Option<Self> {
        match value {
            Some("derived") => Some(TitleSource::Derived),
            Some("generated") => Some(TitleSource::Generated),
            Some("provider") => Some(TitleSource::Provider),
            _ => None,
        }
    }

    /// How much a title from this source is trusted. A write never lowers it.
    pub fn rank(self) -> u8 {
        match self {
            TitleSource::Derived => 1,
            TitleSource::Generated => 2,
            TitleSource::Provider => 3,
        }
    }
}

/// The rank of what a session is called now: 0 for a placeholder, then the
/// source's rank, and [`USER_RANK`] for a real title with no recorded source.
pub fn current_rank(title: Option<&str>, label: &str, source: Option<&str>) -> u8 {
    if needs_title(title.or(Some(label))) {
        return 0;
    }
    TitleSource::parse(source).map_or(USER_RANK, TitleSource::rank)
}

/// Harnesses that keep a title Bridge can read back.
pub fn harness_names_its_chats(harness: &str) -> bool {
    harness == "claude"
}

/// Everything a title decision needs, read in one cheap pass so the caller can let
/// go of the database before doing any provider I/O.
#[derive(Debug, Clone)]
pub struct TitlePlan {
    pub harness: String,
    pub provider_session_id: Option<String>,
    pub cwd: Option<String>,
    /// The message a heading would be cut from, when the session has nothing
    /// better than a heading.
    pub naming_message: Option<String>,
    /// The rank of the title stored now; see [`current_rank`].
    pub rank: u8,
    /// What the person said, for harnesses whose title is read back; see [`grounded`].
    pub grounding: String,
}

/// Reads what is needed to decide a title. Returns None when there is nothing to
/// do: the session is gone, its title is the user's, or it has a generated or
/// provider title and a harness with no title of its own to follow.
///
/// Cheap by construction — one row plus at most eight entries — because callers
/// hold a process-wide lock while they run it.
pub fn plan(db: &Connection, session_id: &str) -> Result<Option<TitlePlan>, BridgeError> {
    let row = db.query_row(
        "SELECT title,label,harness,provider_session_id,title_source,cwd FROM sessions WHERE id=?1",
        params![session_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        },
    );
    let Ok((title, label, harness, provider_session_id, source, cwd)) = row else {
        return Ok(None);
    };
    let rank = current_rank(title.as_deref(), &label, source.as_deref());
    if rank >= USER_RANK {
        return Ok(None);
    }
    // Past a heading only the harness's own title can improve on what is stored,
    // and Claude renames as a conversation moves on, so its title is followed.
    let follows_provider = harness_names_its_chats(&harness) && provider_session_id.is_some();
    if rank >= TitleSource::Generated.rank() && !follows_provider {
        return Ok(None);
    }
    let naming_message = if rank <= TitleSource::Derived.rank() {
        naming_message(db, session_id)?
    } else {
        None
    };
    let grounding = if follows_provider { grounding_text(db, session_id)? } else { String::new() };
    Ok(Some(TitlePlan { harness, provider_session_id, cwd, naming_message, rank, grounding }))
}

/// The harness's own title, where it keeps one and it can be trusted: a name the
/// user gave the session in the harness always, a generated one only when it is
/// [`grounded`] in what the person said. Walks Claude's project directories and
/// reads a transcript, so it must not run under the database lock.
pub fn provider_title(harness: &str, provider_session_id: Option<&str>, cwd: Option<&str>, grounding: &str) -> Option<String> {
    let found = match (harness, provider_session_id) {
        ("claude", Some(id)) => claude_projects_dir()
            .as_deref()
            .and_then(|dir| claude_transcript_title(dir, id, cwd)),
        // Codex keeps no title, and OpenCode's names the answer rather than the
        // question; both are titled by the title job instead.
        _ => None,
    }?;
    (found.chosen || grounded(&found.title, grounding)).then_some(found.title)
}

/// Settles on a title. Takes no database handle on purpose: this is the half that
/// walks Claude's project directories and reads a transcript, and it must not run
/// while the caller holds the database.
pub fn resolve(plan: &TitlePlan) -> Option<(String, TitleSource)> {
    if let Some(provider) = provider_title(&plan.harness, plan.provider_session_id.as_deref(), plan.cwd.as_deref(), &plan.grounding) {
        return Some((provider, TitleSource::Provider));
    }
    // Recompute derived names too, so old sentence-length titles can be repaired.
    plan.naming_message
        .as_deref()
        .and_then(heading_from_message)
        .map(|heading| (heading, TitleSource::Derived))
}

/// Writes a settled title unless it would lower the stored title's rank. Equal
/// rank is allowed, so a heading can be recomputed and Claude's title followed.
/// Returns whether anything changed. Cheap, so it is safe to hold the database
/// across it, and the read and the write share that one hold.
pub fn commit(
    db: &Connection,
    session_id: &str,
    title: &str,
    source: TitleSource,
) -> Result<bool, BridgeError> {
    let words = if source == TitleSource::Derived { MAX_TITLE_WORDS } else { MAX_NAMED_WORDS };
    let title = bound(title, words);
    if title.is_empty() {
        return Ok(false);
    }
    let current = db
        .query_row(
            "SELECT title,label,title_source FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<String>>(2)?)),
        )
        .optional()?;
    let Some((stored, label, stored_source)) = current else {
        return Ok(false);
    };
    if current_rank(stored.as_deref(), &label, stored_source.as_deref()) > source.rank() {
        return Ok(false);
    }
    let changed = db.execute(
        "UPDATE sessions SET title=?2,title_source=?3 WHERE id=?1 AND (title IS NOT ?2 OR title_source IS NOT ?3)",
        params![session_id, title, source.as_str()],
    )?;
    Ok(changed > 0)
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
    if !commit(db, session_id, &title, source)? {
        return Ok(None);
    }
    Ok(Some(bound(&title, if source == TitleSource::Derived { MAX_TITLE_WORDS } else { MAX_NAMED_WORDS })))
}

/// Catch up placeholders and old automatic names to the concise-title policy.
/// Explicit user titles (no recorded automatic source) are never rewritten.
///
/// Deliberately does no file or network I/O. Opening the database must stay cheap
/// and side-effect free; the harness's own title, and a generated one, arrive
/// through the title job ([`crate::title_jobs`]) instead.
pub fn backfill_from_messages(db: &Connection) -> Result<usize, BridgeError> {
    let mut untitled: Vec<(String, Option<String>, bool)> = Vec::new();
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
        if commit(db, &id, &heading, TitleSource::Derived)? {
            named += 1;
        }
    }
    Ok(named)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> Connection {
        let db = Connection::open_in_memory().expect("db");
        db.execute_batch(
            "CREATE TABLE sessions(id TEXT PRIMARY KEY, title TEXT, label TEXT, harness TEXT, provider_session_id TEXT, title_source TEXT, cwd TEXT);
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
        assert_eq!(pending.rank, TitleSource::Derived.rank());
        assert_eq!(pending.provider_session_id.as_deref(), Some("prov-1"));

        // With a provider title available, it wins.
        let named = resolve(&TitlePlan { harness: "claude".into(), provider_session_id: None, cwd: None, naming_message: None, rank: 1, grounding: String::new() });
        assert_eq!(named, None, "no provider title and nothing new to derive");
        assert!(commit(&db, "s1", "Sidebar redesign", TitleSource::Provider).expect("commit"));

        // Once it is the provider's, only the provider is consulted again.
        let after = plan(&db, "s1").expect("plan").expect("claude is followed");
        assert_eq!(after.rank, TitleSource::Provider.rank());
        assert_eq!(after.naming_message, None);
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
            cwd: None,
            naming_message: Some("fix the rail grouping".into()),
            rank: 1,
            grounding: String::new(),
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
        assert_eq!(plan(&db, "s1").expect("plan").expect("replaceable").rank, TitleSource::Derived.rank());
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
    fn provider_titles_keep_up_to_six_words() {
        // A harness title is a phrase; three words would cut it mid-thought.
        assert_eq!(title_from_transcript(r#"{"type":"custom-title","customTitle":"Mission Control drag and drop fixes"}"#), Some("Mission Control drag and drop fixes".into()));
        assert_eq!(title_from_transcript(r#"{"type":"ai-title","aiTitle":"Fix the Mission Control drag and drop regression today"}"#), Some("Fix the Mission Control drag and".into()));
        let db = seeded();
        session(&db, "s1", None, "codex");
        commit(&db, "s1", "Paper and slate launch video", TitleSource::Generated).unwrap();
        let title: String = db.query_row("SELECT title FROM sessions WHERE id='s1'", [], |row| row.get(0)).unwrap();
        assert_eq!(title, "Paper and slate launch video");
        // Codex has no title of its own to follow, so a generated one is settled.
        assert!(plan(&db, "s1").unwrap().is_none());
        // Headings cut locally stay at three words.
        assert_eq!(heading_from_message("Paper and slate launch video").as_deref(), Some("Paper slate launch"));
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
        assert_eq!(backfill_from_messages(&db).unwrap(), 1);
        let title = |id: &str| db.query_row("SELECT title FROM sessions WHERE id=?1", [id], |row| row.get::<_, String>(0)).unwrap();
        assert_eq!(title("derived"), "Mission Control dragging");
        assert_eq!(title("provider"), "Mission Control drag and drop fixes");
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
                // Read by path: looking each one up by id would scan every
                // project directory once per transcript.
                if let Some(HarnessTitle { title, .. }) = title_from_transcript_file(&path) {
                    named += 1;
                    println!("{id:<40} {title}");
                    assert!(!title.is_empty());
                    assert!(title.chars().count() <= MAX_HEADING + 1);
                    assert!(title.split_whitespace().count() <= MAX_NAMED_WORDS);
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
            claude_transcript_title(&root, "session-42", None).map(|found| found.title),
            Some("Found by id".into())
        );
        assert_eq!(claude_transcript_title(&root, "missing", None), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chat_is_a_placeholder_label() {
        // Every direct chat is born "Chat", and was never titled because of it.
        assert!(needs_title(Some("Chat")));
        assert!(needs_title(Some("chat")));
        let db = seeded();
        db.execute("INSERT INTO sessions(id,title,label,harness) VALUES('d1',NULL,'Chat','codex')", []).unwrap();
        said(&db, "d1", "fix the model profile migration");
        assert_eq!(refresh(&db, "d1").unwrap(), Some("Model profile migration".into()));
        db.execute("INSERT INTO sessions(id,title,label,harness) VALUES('d2',NULL,'Chat','claude')", []).unwrap();
        said(&db, "d2", "freeze AxonHub output_format drop");
        assert_eq!(backfill_from_messages(&db).unwrap(), 1);
        let title: String = db.query_row("SELECT title FROM sessions WHERE id='d2'", [], |row| row.get(0)).unwrap();
        assert_eq!(title, "Freeze AxonHub output_format");
    }

    #[test]
    fn claudes_ai_title_names_a_chat() {
        let transcript = concat!(
            r#"{"type":"user","message":{"role":"user"}}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"Slack inbox connector","sessionId":"abc"}"#, "\n",
        );
        assert_eq!(title_from_transcript(transcript), Some("Slack inbox connector".into()));
    }

    #[test]
    fn a_custom_title_beats_a_later_ai_title() {
        let transcript = concat!(
            r#"{"type":"custom-title","customTitle":"Named by hand","sessionId":"abc"}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"Named by Claude","sessionId":"abc"}"#, "\n",
        );
        assert_eq!(title_from_transcript(transcript), Some("Named by hand".into()));
    }

    #[test]
    fn the_last_ai_title_wins() {
        let transcript = concat!(
            r#"{"type":"ai-title","aiTitle":"Early guess","sessionId":"abc"}"#, "\n",
            r#"{"type":"summary","summary":"Old summary"}"#, "\n",
            r#"{"type":"ai-title","aiTitle":"Settled topic","sessionId":"abc"}"#, "\n",
        );
        assert_eq!(title_from_transcript(transcript), Some("Settled topic".into()));
    }

    #[test]
    fn a_title_entry_type_inside_a_message_does_not_name_a_chat() {
        // Tool output quoting a transcript is not a title entry: the type has to lead.
        let quoted = r#"{"type":"user","message":{"content":"{\"type\":\"ai-title\",\"aiTitle\":\"Quoted\"}"}}"#;
        assert_eq!(title_from_transcript(quoted), None);
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("bridge-titles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_title_in_the_tail_of_a_large_transcript_is_found() {
        let root = temp_root("tail");
        let project = root.join("-tmp-big");
        std::fs::create_dir_all(&project).unwrap();
        let filler = format!("{}\n", r#"{"type":"assistant","message":{"content":"x"}}"#).repeat(30_000);
        let body = format!(
            "{}\n{filler}{}\n{filler}",
            r#"{"type":"ai-title","aiTitle":"Stale early title"}"#,
            r#"{"type":"ai-title","aiTitle":"Current topic"}"#,
        );
        assert!(body.len() as u64 > 2 * TRANSCRIPT_TAIL_BYTES);
        std::fs::write(project.join("big.jsonl"), &body).unwrap();
        // Out of the tail, but the file is small enough to stream.
        assert_eq!(claude_transcript_title(&root, "big", None).map(|found| found.title), Some("Current topic".into()));
        let near_end = format!("{filler}{}\n{}", r#"{"type":"ai-title","aiTitle":"Near the end"}"#, &filler[..4_000]);
        std::fs::write(project.join("tail.jsonl"), near_end).unwrap();
        assert_eq!(claude_transcript_title(&root, "tail", None).map(|found| found.title), Some("Near the end".into()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_transcript_is_found_through_its_cwd_slug() {
        let root = temp_root("slug");
        let cwd = "/Users/me/Library/Application Support/dev.bridge.deck/chats/abc";
        let project = root.join("-Users-me-Library-Application-Support-dev-bridge-deck-chats-abc");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("s-1.jsonl"), r#"{"type":"ai-title","aiTitle":"Through the slug"}"#).unwrap();
        assert_eq!(claude_project_slug(cwd), "-Users-me-Library-Application-Support-dev-bridge-deck-chats-abc");
        assert_eq!(claude_transcript_title(&root, "s-1", Some(cwd)).map(|found| found.title), Some("Through the slug".into()));
        let _ = std::fs::remove_dir_all(&root);
    }

    fn stored(db: &Connection, id: &str) -> (Option<String>, Option<String>) {
        db.query_row("SELECT title,title_source FROM sessions WHERE id=?1", [id], |row| Ok((row.get(0)?, row.get(1)?))).unwrap()
    }

    #[test]
    fn a_generated_title_is_not_replaced_by_a_derived_one() {
        let db = seeded();
        session(&db, "s1", None, "codex");
        said(&db, "s1", "pls isnt showijng the titles");
        assert!(commit(&db, "s1", "Missing chat titles", TitleSource::Generated).unwrap());
        assert!(!commit(&db, "s1", "Pls isnt showijng", TitleSource::Derived).unwrap());
        assert_eq!(refresh(&db, "s1").unwrap(), None);
        assert_eq!(backfill_from_messages(&db).unwrap(), 0);
        assert_eq!(stored(&db, "s1"), (Some("Missing chat titles".into()), Some("generated".into())));
    }

    #[test]
    fn a_provider_title_replaces_a_generated_one() {
        let db = seeded();
        session(&db, "s1", None, "claude");
        assert!(commit(&db, "s1", "Missing chat titles", TitleSource::Generated).unwrap());
        assert!(commit(&db, "s1", "Chat titles backfill and display", TitleSource::Provider).unwrap());
        assert!(!commit(&db, "s1", "Something generated later", TitleSource::Generated).unwrap());
        assert_eq!(stored(&db, "s1"), (Some("Chat titles backfill and display".into()), Some("provider".into())));
    }

    #[test]
    fn a_provider_title_follows_claudes_newer_title() {
        let root = temp_root("follow");
        let project = root.join("p");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("prov-9.jsonl");
        std::fs::write(&path, r#"{"type":"ai-title","aiTitle":"First topic"}"#).unwrap();
        let db = seeded();
        db.execute("INSERT INTO sessions(id,title,label,harness,provider_session_id) VALUES('s1',NULL,'Chat','claude','prov-9')", []).unwrap();
        let name = |db: &Connection| {
            let plan = plan(db, "s1").unwrap().unwrap();
            let title = claude_transcript_title(&root, plan.provider_session_id.as_deref().unwrap(), None).unwrap();
            commit(db, "s1", &title.title, TitleSource::Provider).unwrap()
        };
        assert!(name(&db));
        std::fs::write(&path, "{\"type\":\"ai-title\",\"aiTitle\":\"First topic\"}\n{\"type\":\"ai-title\",\"aiTitle\":\"Where it ended up\"}").unwrap();
        assert!(name(&db));
        assert_eq!(stored(&db, "s1"), (Some("Where it ended up".into()), Some("provider".into())));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_user_title_is_never_replaced() {
        let db = seeded();
        session(&db, "s1", Some("Chosen by hand"), "claude");
        for source in [TitleSource::Derived, TitleSource::Generated, TitleSource::Provider] {
            assert!(!commit(&db, "s1", "Automatic name", source).unwrap());
        }
        assert_eq!(stored(&db, "s1"), (Some("Chosen by hand".into()), None));
        assert_eq!(current_rank(Some("Chosen by hand"), "Chat", None), USER_RANK);
    }

    #[test]
    fn a_harness_title_must_share_a_topic_word_with_what_was_said() {
        // Seen on a real store: Claude named these chats after Bridge's own
        // session context or a resume, not after what the person asked.
        assert!(!grounded("Bridge session capabilities", "We have found a lot of bugs in dyanamo in the past weeks"));
        assert!(!grounded("Bridge credential proxy verification", "Please review trhis PR https://github.com/o/kairo/pull/65"));
        assert!(!grounded("Continue work", "I want you to create two issues, one for chat defaults"));
        assert!(!grounded("Session start", "hi"));
        assert!(!grounded("Make a PR", "read agents.md and then find a bug in switchyard"));
        // And these are what the person was talking about.
        assert!(grounded("LINE WORKS account suspension", "why was our line works account suspended?"));
        assert!(grounded("Issue 836 review", "look at issue 836 and document it"));
        assert!(grounded("Chat titles backfill and display", "make a backfill queue job so past chats have ttitles"));
        assert!(grounded("Kairo PR review", "review https://github.com/o/kairo/pull/65"));
    }

    #[test]
    fn a_name_the_user_chose_in_claude_needs_no_grounding() {
        let mut titles = TranscriptTitles::default();
        titles.observe(r#"{"type":"custom-title","customTitle":"Friday sync","sessionId":"a"}"#);
        assert_eq!(titles.best(), Some(HarnessTitle { title: "Friday sync".into(), chosen: true }));
        let mut titles = TranscriptTitles::default();
        titles.observe(r#"{"type":"ai-title","aiTitle":"Continue work","sessionId":"a"}"#);
        assert_eq!(titles.best(), Some(HarnessTitle { title: "Continue work".into(), chosen: false }));
    }

    #[test]
    fn grounding_reads_the_first_and_latest_messages() {
        let db = seeded();
        session(&db, "s1", None, "claude");
        said(&db, "s1", "look at the dynamo router");
        for _ in 0..10 { said(&db, "s1", "ok"); }
        said(&db, "s1", "now the sentry alerts");
        let text = grounding_text(&db, "s1").unwrap();
        assert!(text.contains("dynamo router") && text.contains("sentry alerts"));
    }
}
