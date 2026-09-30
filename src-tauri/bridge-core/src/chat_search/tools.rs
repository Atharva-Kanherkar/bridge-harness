//! The three lookups the model may ask Bridge for, and the cards they print.
//!
//! These are not provider tools: the model has none. It names a lookup in its
//! reply, Bridge runs it here against the index, and the printed result goes
//! back as the next turn. Every output is truncated before it is counted, so
//! the model cannot overspend by asking for something large, and every string
//! passes the same secret sanitizer a user turn does before it can reach a
//! provider.
//!
//! Ids are shortened to eight characters so a card costs about 60 tokens.
//! [`Shown`] remembers every id printed, and an answer can only name one of
//! those: a chat the model was never shown cannot be suggested.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;

use super::parse::{self, ParsedQuery, Term};
use super::retrieve::{self, truncate_chars, Candidate};
use crate::{secret_interception, BridgeError};

pub const MAX_FIND_LIMIT: usize = 8;
pub const MAX_PEEK_WINDOWS: usize = 3;
pub const PEEK_WINDOW_CHARS: usize = 200;
pub const OUTLINE_CHARS: usize = 300;
pub const CARD_SNIPPET_CHARS: usize = 80;
const SHORT_ID: usize = 8;

/// Tokens a string costs the model, by the usual four-characters rule. Tool
/// outputs are Bridge's own truncated text, so this is the measure of what is
/// actually sent.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Redact before a string can reach a provider.
pub fn redact(text: &str) -> String {
    secret_interception::sanitize(text).text
}

/// Every chat id the model has been shown, by the short id it saw.
#[derive(Debug, Default)]
pub struct Shown {
    by_short: HashMap<String, String>,
    candidates: HashMap<String, Candidate>,
}

impl Shown {
    /// The short id for `candidate`, remembering it. Two chats that share a
    /// prefix get a longer one rather than a collision.
    pub fn register(&mut self, candidate: &Candidate) -> String {
        let id = &candidate.session_id;
        if let Some((short, _)) = self.by_short.iter().find(|(_, full)| *full == id) {
            return short.clone();
        }
        let mut length = SHORT_ID.min(id.len());
        let short = loop {
            let short: String = id.chars().take(length).collect();
            match self.by_short.get(&short) {
                Some(existing) if existing != id && length < id.len() => length += 1,
                _ => break short,
            }
        };
        self.by_short.insert(short.clone(), id.clone());
        self.candidates.insert(id.clone(), candidate.clone());
        short
    }

    /// Resolve an id the model wrote, short or full. `None` means it was
    /// never shown.
    pub fn resolve(&self, id: &str) -> Option<&Candidate> {
        let id = id.trim();
        let full = self.by_short.get(id).or_else(|| {
            self.candidates.contains_key(id).then(|| {
                self.by_short
                    .values()
                    .find(|full| full.as_str() == id)
                    .expect("a registered candidate has a short id")
            })
        })?;
        self.candidates.get(full)
    }

    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

fn day(at: &str) -> &str {
    at.get(..10).unwrap_or(at)
}

/// One entry's searchable text, whole. Redaction runs on this before any
/// window is cut: a window cut first could split a secret into a fragment no
/// detector recognises.
fn entry_body(db: &Connection, entry_id: &str) -> Result<Option<String>, BridgeError> {
    let sql = format!(
        "SELECT {} FROM session_entries WHERE id = ?1",
        crate::session_recall::searchable_body_sql("payload")
    );
    Ok(db.prepare_cached(&sql)?.query_row(params![entry_id], |row| row.get(0)).optional()?)
}

fn digest_body(db: &Connection, session_id: &str) -> Result<Option<String>, BridgeError> {
    Ok(db
        .prepare_cached("SELECT body FROM chat_digests WHERE session_id = ?1")?
        .query_row(params![session_id], |row| row.get(0))
        .optional()?)
}

/// `chars` characters of `body` around the first of `terms` it contains,
/// rarest term first; the opening of `body` when none is found.
pub fn window(body: &str, terms: &[String], chars: usize) -> String {
    let flat: Vec<char> = body.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect();
    if flat.len() <= chars {
        return flat.into_iter().collect();
    }
    let lower: Vec<char> = flat
        .iter()
        .map(|character| character.to_lowercase().next().unwrap_or(*character))
        .collect();
    let found = terms.iter().find_map(|term| {
        let needle: Vec<char> = term.to_lowercase().chars().collect();
        if needle.is_empty() || needle.len() > lower.len() {
            return None;
        }
        lower.windows(needle.len()).position(|slice| slice == needle.as_slice())
    });
    let keep = chars.saturating_sub(2).max(1);
    let start = found.map_or(0, |at| at.saturating_sub(keep / 3));
    let start = start.min(flat.len().saturating_sub(keep));
    let end = (start + keep).min(flat.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&flat[start..end]);
    if end < flat.len() {
        out.push('…');
    }
    out
}

/// The snippet a model may see for `candidate`: its best entry, else its
/// digest, redacted whole and then windowed.
pub fn model_snippet(db: &Connection, candidate: &Candidate, terms: &[String], chars: usize) -> String {
    let body = match &candidate.best_entry_id {
        Some(entry_id) => entry_body(db, entry_id),
        None => digest_body(db, &candidate.session_id),
    };
    match body {
        Ok(Some(body)) => window(&redact(&body), terms, chars),
        _ => String::new(),
    }
}

/// One ~60-token line: id, date, harness, title, and a short snippet.
pub fn card(db: &Connection, shown: &mut Shown, candidate: &Candidate, terms: &[String]) -> String {
    let short = shown.register(candidate);
    let mut flags = String::new();
    if candidate.archived {
        flags.push_str(" [archived]");
    } else if candidate.ended {
        flags.push_str(" [ended]");
    }
    let snippet = model_snippet(db, candidate, terms, CARD_SNIPPET_CHARS);
    format!(
        "{short} | {} | {} | {}{flags}{}",
        day(&candidate.last_active_at),
        candidate.harness,
        truncate_chars(&redact(&candidate.title), 80),
        if snippet.is_empty() { String::new() } else { format!(" | \"{snippet}\"") }
    )
}

/// A tool call as the model writes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ToolCall {
    FindChats {
        #[serde(default)]
        terms: Vec<String>,
        since: Option<String>,
        until: Option<String>,
        harness: Option<String>,
        limit: Option<usize>,
    },
    PeekChat {
        id: String,
        term: String,
        n: Option<usize>,
    },
    ChatOutline {
        id: String,
    },
}

impl ToolCall {
    pub fn name(&self) -> &'static str {
        match self {
            Self::FindChats { .. } => "find_chats",
            Self::PeekChat { .. } => "peek_chat",
            Self::ChatOutline { .. } => "chat_outline",
        }
    }
}

fn date(value: Option<&str>) -> Option<DateTime<Utc>> {
    let value = value?.trim();
    DateTime::parse_from_rfc3339(value)
        .map(|at| at.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|day| day.and_hms_opt(0, 0, 0))
                .map(|at| at.and_utc())
        })
}

/// Run one tool call. The output is printed text, already redacted and
/// truncated to its per-tool bound; the caller then clips it to the budget.
pub fn run(
    db: &Connection,
    shown: &mut Shown,
    call: &ToolCall,
    base: &ParsedQuery,
    now: DateTime<Utc>,
) -> Result<String, BridgeError> {
    match call {
        ToolCall::FindChats {
            terms,
            since,
            until,
            harness,
            limit,
        } => {
            // The model's words go through the same parser a user's do, so a
            // synonym it supplies is still just quoted FTS terms.
            let joined = terms.join(" ");
            let mut parsed = parse::parse(&joined, now);
            parsed.open_ended = false;
            for term in &mut parsed.terms {
                term.prefix = false;
            }
            parsed.since = date(since.as_deref()).or(base.since);
            parsed.until = date(until.as_deref()).or(base.until);
            parsed.harness = harness
                .as_deref()
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty())
                .or_else(|| base.harness.clone());
            let ranked: Vec<Term> = parse::rank_terms(db, &parsed)?;
            let limit = limit.unwrap_or(MAX_FIND_LIMIT).clamp(1, MAX_FIND_LIMIT);
            let found = retrieve::retrieve(db, &parsed, &ranked, limit, now)?;
            if found.candidates.is_empty() {
                return Ok("find_chats: no chats matched.".into());
            }
            let words: Vec<String> = ranked.iter().map(|term| term.text.clone()).collect();
            let mut out = format!("find_chats ({}):", found.candidates.len());
            for candidate in &found.candidates {
                out.push('\n');
                out.push_str(&card(db, shown, candidate, &words));
            }
            Ok(out)
        }
        ToolCall::PeekChat { id, term, n } => {
            let Some(candidate) = shown.resolve(id).cloned() else {
                return Ok(format!("peek_chat: {id} is not a chat you were shown."));
            };
            let words = parse::parse(term, now).terms;
            if words.is_empty() {
                return Ok("peek_chat: give a word to look for.".into());
            }
            let expression = words
                .iter()
                .map(|term| Term { prefix: false, ..term.clone() }.to_match())
                .collect::<Vec<_>>()
                .join(" OR ");
            let limit = n.unwrap_or(MAX_PEEK_WINDOWS).clamp(1, MAX_PEEK_WINDOWS);
            let mut statement = db.prepare_cached(
                "SELECT entry_id FROM session_entry_fts
                 WHERE session_entry_fts.session_id = ?1 AND session_entry_fts MATCH ?2
                 ORDER BY rank LIMIT ?3",
            )?;
            let entry_ids: Vec<String> = statement
                .query_map(params![candidate.session_id, expression, limit as i64], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            let needles: Vec<String> = words.iter().map(|term| term.text.clone()).collect();
            let short = shown.register(&candidate);
            let mut windows = Vec::new();
            for entry_id in entry_ids {
                if let Some(body) = entry_body(db, &entry_id)? {
                    windows.push(window(&redact(&body), &needles, PEEK_WINDOW_CHARS));
                }
            }
            if windows.is_empty() {
                return Ok(format!("peek_chat {short}: no match for that word."));
            }
            let mut out = format!("peek_chat {short}:");
            for (index, window) in windows.iter().enumerate() {
                out.push_str(&format!("\n{}. {window}", index + 1));
            }
            Ok(out)
        }
        ToolCall::ChatOutline { id } => {
            let Some(candidate) = shown.resolve(id).cloned() else {
                return Ok(format!("chat_outline: {id} is not a chat you were shown."));
            };
            let (first, summary, entries): (Option<String>, Option<String>, i64) = db.query_row(
                "SELECT
                    (SELECT json_extract(payload, '$.text') FROM session_entries
                      WHERE session_id = ?1 AND kind = 'user.message'
                        AND context_visibility IN ('eligible','visible')
                      ORDER BY sequence LIMIT 1),
                    (SELECT coalesce(json_extract(payload, '$.summary'), json_extract(payload, '$.text'))
                      FROM session_entries
                      WHERE session_id = ?1 AND kind IN ('compaction','checkpoint','branch.summary')
                      ORDER BY sequence DESC LIMIT 1),
                    (SELECT count(*) FROM session_entries WHERE session_id = ?1)",
                params![candidate.session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let short = shown.register(&candidate);
            let mut out = format!(
                "chat_outline {short} | {} | {entries} entries",
                truncate_chars(&redact(&candidate.title), 80)
            );
            if let Some(first) = first.filter(|text| !text.trim().is_empty()) {
                out.push_str(&format!("\nfirst: {}", truncate_chars(&redact(&first), OUTLINE_CHARS)));
            }
            if let Some(summary) = summary.filter(|text| !text.trim().is_empty()) {
                out.push_str(&format!("\nsummary: {}", truncate_chars(&redact(&summary), OUTLINE_CHARS)));
            }
            Ok(out)
        }
    }
}

/// Clip `text` to `tokens` by the same estimate the budget counts with.
pub fn clip_to_tokens(text: &str, tokens: usize) -> String {
    if estimate_tokens(text) <= tokens {
        return text.to_owned();
    }
    let keep = tokens.saturating_mul(4).saturating_sub(1);
    format!("{}…", text.chars().take(keep).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str) -> Candidate {
        Candidate {
            session_id: id.into(),
            title: "Title".into(),
            harness: "codex".into(),
            workspace_id: None,
            workspace_title: None,
            last_active_at: "2026-09-27T10:00:00+00:00".into(),
            started_at: None,
            best_entry_id: None,
            entry_matches: 1,
            digest_match: false,
            snippet: "a snippet".into(),
            score: 1.0,
            archived: false,
            ended: false,
        }
    }

    #[test]
    fn short_ids_resolve_and_never_collide() {
        let mut shown = Shown::default();
        let first = shown.register(&candidate("abcdefgh-1111"));
        let second = shown.register(&candidate("abcdefgh-2222"));
        assert_eq!(first, "abcdefgh");
        assert_ne!(first, second);
        assert_eq!(shown.resolve(&first).unwrap().session_id, "abcdefgh-1111");
        assert_eq!(shown.resolve(&second).unwrap().session_id, "abcdefgh-2222");
        assert_eq!(shown.resolve("abcdefgh-2222").unwrap().session_id, "abcdefgh-2222");
        assert!(shown.resolve("zzzzzzzz").is_none());
        assert_eq!(shown.register(&candidate("abcdefgh-1111")), first, "re-registering is stable");
    }

    #[test]
    fn a_card_is_about_sixty_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title)
             VALUES('0123456789abcdef',NULL,'codex','Chat','idle','estimated','direct',?1)",
            params!["t".repeat(200)],
        )
        .unwrap();
        crate::store::append_session_entry(&db, "0123456789abcdef", None, "user.message", &serde_json::json!({"text": "s".repeat(4000)}), None, "eligible", None).unwrap();
        let mut shown = Shown::default();
        let mut long = candidate("0123456789abcdef");
        long.title = "t".repeat(200);
        let line = card(&db, &mut shown, &long, &[]);
        assert!(estimate_tokens(&line) <= 60, "{} tokens: {line}", estimate_tokens(&line));
    }

    #[test]
    fn a_window_is_centred_on_the_rarest_term_found() {
        let body = format!("{} needle {}", "a ".repeat(300), "b ".repeat(300));
        let cut = window(&body, &["missing".into(), "needle".into()], 60);
        assert!(cut.contains("needle"), "{cut}");
        assert!(cut.starts_with('…') && cut.ends_with('…'));
        assert!(cut.chars().count() <= 60);
        assert_eq!(window("short body", &["x".into()], 60), "short body");
        assert!(window(&"z".repeat(500), &[], 60).chars().count() <= 60);
    }

    #[test]
    fn clipping_respects_the_estimate() {
        let clipped = clip_to_tokens(&"x".repeat(1000), 10);
        assert!(estimate_tokens(&clipped) <= 10);
        assert_eq!(clip_to_tokens("short", 10), "short");
    }

    #[test]
    fn tool_calls_parse_from_the_model_shape() {
        let call: ToolCall = serde_json::from_str(r#"{"tool":"find_chats","terms":["stall","hang"],"limit":20}"#).unwrap();
        assert_eq!(call.name(), "find_chats");
        let call: ToolCall = serde_json::from_str(r#"{"tool":"peek_chat","id":"abc","term":"stall"}"#).unwrap();
        assert_eq!(call.name(), "peek_chat");
        let call: ToolCall = serde_json::from_str(r#"{"tool":"chat_outline","id":"abc"}"#).unwrap();
        assert_eq!(call.name(), "chat_outline");
        assert!(serde_json::from_str::<ToolCall>(r#"{"tool":"rm_rf","id":"abc"}"#).is_err());
    }
}
