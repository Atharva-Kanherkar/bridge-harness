//! T0: turn a vague memory into terms and filters, with no model.
//!
//! "the one where we fixed the plugins stall, a few days ago, in codex"
//! becomes the terms `plugins stall fixed`, a harness filter `codex`, and a
//! window of one to seven days back. Everything is resolved against a
//! passed-in `now`, so every phrase is testable.
//!
//! Which terms survive is decided by the index, not by a word list: after
//! stopwords go, the rest are ranked by how many entries contain them and
//! only the rarest few are kept. A rare word is what tells chats apart; a
//! common one only widens every posting list the query has to walk.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc, Weekday};
use rusqlite::{params, Connection, OptionalExtension};

use crate::BridgeError;

/// The most terms a query keeps after ranking. More only adds cost.
pub const MAX_TERMS: usize = 6;
/// Unknown words at least this long retry as a prefix.
const PREFIX_FALLBACK_CHARS: usize = 4;

const HARNESS_WORDS: [&str; 5] = ["codex", "claude", "opencode", "cursor", "grok"];

/// Words that say nothing about which chat is meant. Deliberately short on
/// content words: "fixed" or "stall" are exactly what a user remembers.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "again", "ago", "all", "also", "am", "an", "and", "any", "are", "around",
    "as", "at", "be", "been", "before", "being", "but", "by", "can", "chat", "chats", "could",
    "conversation", "conversations", "did", "do", "does", "doing", "during", "find", "for",
    "from", "get", "got", "had", "has", "have", "he", "her", "here", "him", "his", "how", "i",
    "if", "in", "into", "is", "it", "its", "just", "last", "like", "maybe", "me", "mentioned", "my",
    "of", "on", "one", "or", "our", "remember", "said", "same", "she", "should", "so", "some",
    "something", "session", "sessions", "stuff", "talked", "than", "that", "the", "their",
    "them", "then", "there", "these", "they", "thing", "things", "think", "this", "those",
    "thread", "to", "us", "very", "was", "we", "were", "what", "when", "where", "which",
    "while", "who", "why", "will", "with", "would", "you", "your",
];

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september",
    "october", "november", "december",
];

/// One search term, already free of FTS syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Space-separated tokens; more than one only for a quoted phrase.
    pub text: String,
    pub phrase: bool,
    /// Match as a prefix: the last token of a query still being typed.
    pub prefix: bool,
}

impl Term {
    fn word(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            phrase: false,
            prefix: false,
        }
    }

    /// The FTS5 expression for this term. Every token is quoted, so nothing a
    /// user typed can become an operator.
    pub fn to_match(&self) -> String {
        let quoted = format!("\"{}\"", self.text.replace('"', ""));
        if self.prefix {
            format!("{quoted}*")
        } else {
            quoted
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedQuery {
    /// Candidate terms in the order typed, before ranking.
    pub terms: Vec<Term>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub harness: Option<String>,
    /// Whether the query ended mid-word, so its last token may be a prefix.
    pub open_ended: bool,
}

impl ParsedQuery {
    pub fn has_filter(&self) -> bool {
        self.since.is_some() || self.until.is_some() || self.harness.is_some()
    }
}

/// Split the way the FTS tokenizer does (`unicode61`): anything that is not
/// alphanumeric separates tokens.
fn tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub fn parse(raw: &str, now: DateTime<Utc>) -> ParsedQuery {
    let mut parsed = ParsedQuery {
        open_ended: raw
            .chars()
            .last()
            .is_some_and(char::is_alphanumeric),
        ..ParsedQuery::default()
    };

    // Quoted phrases first, so their words are never read as time or filler.
    let mut rest = String::new();
    let mut phrases = Vec::new();
    let mut inside = false;
    let mut current = String::new();
    for character in raw.chars() {
        if character == '"' {
            if inside {
                let words = tokens(&current);
                if !words.is_empty() {
                    phrases.push(Term {
                        text: words.join(" "),
                        phrase: true,
                        prefix: false,
                    });
                }
                current.clear();
            }
            inside = !inside;
            rest.push(' ');
        } else if inside {
            current.push(character);
        } else {
            rest.push(character);
        }
    }
    // An unclosed quote is ordinary text.
    rest.push(' ');
    rest.push_str(&current);
    if inside {
        parsed.open_ended = false;
    }

    let words = tokens(&rest);
    let mut consumed = vec![false; words.len()];
    if let Some((since, until)) = time_window(&words, &mut consumed, now) {
        parsed.since = since;
        parsed.until = until;
    }
    for (index, word) in words.iter().enumerate() {
        if consumed[index] {
            continue;
        }
        if HARNESS_WORDS.contains(&word.as_str()) && parsed.harness.is_none() {
            parsed.harness = Some(word.clone());
            consumed[index] = true;
        }
    }

    let last_index = words.len().checked_sub(1);
    for (index, word) in words.iter().enumerate() {
        if consumed[index] || STOPWORDS.contains(&word.as_str()) {
            continue;
        }
        if matches!(word.as_str(), "and" | "or" | "not" | "near") {
            continue;
        }
        if parsed.terms.iter().any(|term| term.text == *word) {
            continue;
        }
        let mut term = Term::word(word);
        // Only a still-open final token is a prefix candidate; ranking later
        // decides whether it needs to be one.
        term.prefix = parsed.open_ended && Some(index) == last_index;
        parsed.terms.push(term);
    }
    parsed.terms.extend(phrases);
    parsed
}

fn number(word: &str) -> Option<i64> {
    match word {
        "a" | "an" | "one" => Some(1),
        "two" | "couple" => Some(2),
        "three" | "few" => Some(3),
        "four" => Some(4),
        "five" => Some(5),
        "six" => Some(6),
        "seven" => Some(7),
        "eight" => Some(8),
        "nine" => Some(9),
        "ten" => Some(10),
        other => other.parse().ok().filter(|value| (1..=365).contains(value)),
    }
}

fn start_of_day(at: DateTime<Utc>) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(at.year(), at.month(), at.day(), 0, 0, 0)
        .single()
        .unwrap_or(at)
}

fn start_of_month(year: i32, month: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .expect("the first of a month exists")
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

type Window = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

/// Find the first time phrase and mark its words consumed. One phrase per
/// query: two windows would have to be intersected or unioned, and neither is
/// what "last week, or yesterday" means often enough to guess.
fn time_window(words: &[String], consumed: &mut [bool], now: DateTime<Utc>) -> Option<Window> {
    let today = start_of_day(now);
    let word = |index: usize| words.get(index).map(String::as_str);
    for index in 0..words.len() {
        let mark = |consumed: &mut [bool], from: usize, to: usize| {
            for slot in consumed.iter_mut().take(to + 1).skip(from) {
                *slot = true;
            }
        };
        match word(index)? {
            "today" => {
                mark(consumed, index, index);
                return Some((Some(today), Some(now)));
            }
            "yesterday" => {
                mark(consumed, index, index);
                return Some((Some(today - Duration::days(1)), Some(today)));
            }
            "recently" | "lately" | "recent" => {
                mark(consumed, index, index);
                return Some((Some(now - Duration::days(14)), Some(now)));
            }
            "last" | "past" | "previous" | "this" => {
                let qualifier = word(index).unwrap_or_default();
                match word(index + 1) {
                    Some("week") => {
                        mark(consumed, index, index + 1);
                        let days = if qualifier == "this" { 7 } else { 14 };
                        return Some((Some(now - Duration::days(days)), Some(now)));
                    }
                    Some("month") => {
                        mark(consumed, index, index + 1);
                        let (year, month) = (now.year(), now.month());
                        if qualifier == "this" {
                            return Some((Some(start_of_month(year, month)), Some(now)));
                        }
                        let (previous_year, previous_month) =
                            if month == 1 { (year - 1, 12) } else { (year, month - 1) };
                        return Some((
                            Some(start_of_month(previous_year, previous_month)),
                            Some(start_of_month(year, month)),
                        ));
                    }
                    Some(day) if weekday(day).is_some() => {
                        mark(consumed, index, index + 1);
                        return Some(weekday_window(weekday(day)?, now));
                    }
                    _ => {}
                }
            }
            "ago" => {
                // "<n> <unit> ago", "a few days ago", "a couple of weeks ago".
                let Some(unit) = index.checked_sub(1).and_then(word) else {
                    continue;
                };
                let unit_days = match unit {
                    "day" | "days" => 1,
                    "week" | "weeks" => 7,
                    "month" | "months" => 30,
                    _ => continue,
                };
                let mut start = index - 1;
                let mut count = None;
                if let Some(before) = start.checked_sub(1) {
                    let mut probe = before;
                    if word(probe) == Some("of") {
                        probe = probe.checked_sub(1).unwrap_or(probe);
                    }
                    if let Some(value) = word(probe).and_then(number) {
                        count = Some((value, word(probe) == Some("few")));
                        start = probe;
                        if matches!(word(probe), Some("few") | Some("couple")) {
                            if let Some(article) = probe.checked_sub(1) {
                                if word(article) == Some("a") {
                                    start = article;
                                }
                            }
                        }
                    }
                }
                let (value, vague) = count.unwrap_or((1, false));
                mark(consumed, start, index);
                if vague {
                    return Some((
                        Some(now - Duration::days(7 * unit_days)),
                        Some(now - Duration::days(unit_days)),
                    ));
                }
                return Some((
                    Some(now - Duration::days((value + 1) * unit_days)),
                    Some(now - Duration::days((value - 1) * unit_days)),
                ));
            }
            month_word => {
                let Some(month_index) = MONTHS.iter().position(|month| *month == month_word) else {
                    if let Some(day) = weekday(month_word) {
                        if index > 0 && word(index - 1) == Some("on") {
                            mark(consumed, index - 1, index);
                            return Some(weekday_window(day, now));
                        }
                    }
                    continue;
                };
                // "may" is also a verb: only a preposition makes it a month.
                let introduced = index > 0 && matches!(word(index - 1), Some("in") | Some("during") | Some("since"));
                if month_word == "may" && !introduced {
                    continue;
                }
                let month = month_index as u32 + 1;
                let year = if month <= now.month() { now.year() } else { now.year() - 1 };
                let (end_year, end_month) = next_month(year, month);
                mark(consumed, if introduced { index - 1 } else { index }, index);
                return Some((Some(start_of_month(year, month)), Some(start_of_month(end_year, end_month).min(now))));
            }
        }
    }
    None
}

fn weekday(word: &str) -> Option<Weekday> {
    match word {
        "monday" => Some(Weekday::Mon),
        "tuesday" => Some(Weekday::Tue),
        "wednesday" => Some(Weekday::Wed),
        "thursday" => Some(Weekday::Thu),
        "friday" => Some(Weekday::Fri),
        "saturday" => Some(Weekday::Sat),
        "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

/// The most recent past occurrence of `day`, as a whole day.
fn weekday_window(day: Weekday, now: DateTime<Utc>) -> Window {
    let today = start_of_day(now);
    let back = (7 + now.weekday().num_days_from_monday() as i64 - day.num_days_from_monday() as i64) % 7;
    let back = if back == 0 { 7 } else { back };
    let start = today - Duration::days(back);
    (Some(start), Some(start + Duration::days(1)))
}

/// How many indexed rows (entries plus digests) contain `term`, from the
/// FTS vocabularies. A prefix term counts every vocabulary word it covers.
fn document_frequency(db: &Connection, term: &Term) -> Result<i64, BridgeError> {
    let mut total = 0;
    for vocab in ["session_entry_vocab", "chat_digest_vocab"] {
        let count: Option<i64> = if term.prefix {
            let upper = format!("{}\u{10FFFF}", term.text);
            db.query_row(
                &format!("SELECT sum(doc) FROM {vocab} WHERE term >= ?1 AND term < ?2"),
                params![term.text, upper],
                |row| row.get(0),
            )
            .optional()?
            .flatten()
        } else {
            db.query_row(
                &format!("SELECT doc FROM {vocab} WHERE term = ?1"),
                params![term.text],
                |row| row.get(0),
            )
            .optional()?
        };
        total += count.unwrap_or(0);
    }
    Ok(total)
}

/// Keep the rarest terms the index actually contains.
///
/// A term no entry contains can only empty an AND and add nothing to an OR,
/// so it is dropped; the model stage is where a word the user remembers but
/// never typed into a chat gets a synonym. Phrases are kept as typed: they
/// are the user saying exactly which words matter.
pub fn rank_terms(db: &Connection, parsed: &ParsedQuery) -> Result<Vec<Term>, BridgeError> {
    Ok(rank_terms_counted(db, parsed)?.0)
}

/// [`rank_terms`], plus how many typed words the index has never seen. An
/// unknown word means the index is answering part of the question, which the
/// confidence gate must know.
pub fn rank_terms_counted(db: &Connection, parsed: &ParsedQuery) -> Result<(Vec<Term>, usize), BridgeError> {
    let mut unknown = 0;
    let mut ranked: Vec<(i64, Term)> = Vec::new();
    for term in &parsed.terms {
        if term.phrase {
            ranked.push((0, term.clone()));
            continue;
        }
        let exact = Term {
            prefix: false,
            ..term.clone()
        };
        let exact_count = document_frequency(db, &exact)?;
        if exact_count > 0 {
            // A finished word stays exact even at the end of the query.
            ranked.push((exact_count, exact));
            continue;
        }
        // A fragment still being typed, or a word the index only holds in
        // another form ("subagent" for "subagents", "restart" for
        // "restarts"): match it as a prefix. The unicode61 tokenizer does no
        // stemming, and a prefix is the cheap half of it. Short words stay
        // exact, since "ab*" matches too much to mean anything.
        let chars = term.text.chars().count();
        if (term.prefix && chars >= 2) || chars >= PREFIX_FALLBACK_CHARS {
            let prefixed = Term {
                prefix: true,
                ..term.clone()
            };
            let prefix_count = document_frequency(db, &prefixed)?;
            if prefix_count > 0 {
                ranked.push((prefix_count, prefixed));
                continue;
            }
        }
        unknown += 1;
    }
    ranked.sort_by_key(|(count, _)| *count);
    ranked.truncate(MAX_TERMS);
    Ok((ranked.into_iter().map(|(_, term)| term).collect(), unknown))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    // A Wednesday.
    const NOW: &str = "2026-09-30T15:00:00Z";

    fn window(query: &str) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
        let parsed = parse(query, at(NOW));
        (parsed.since, parsed.until)
    }

    fn words(query: &str) -> Vec<String> {
        parse(query, at(NOW))
            .terms
            .into_iter()
            .map(|term| term.text)
            .collect()
    }

    #[test]
    fn yesterday_is_the_whole_previous_day() {
        assert_eq!(
            window("the migration yesterday"),
            (Some(at("2026-09-29T00:00:00Z")), Some(at("2026-09-30T00:00:00Z")))
        );
        assert_eq!(words("the migration yesterday"), vec!["migration"]);
    }

    #[test]
    fn today_runs_from_midnight_to_now() {
        assert_eq!(window("today"), (Some(at("2026-09-30T00:00:00Z")), Some(at(NOW))));
    }

    #[test]
    fn n_days_ago_has_a_day_of_slack_each_side() {
        assert_eq!(
            window("stall 3 days ago"),
            (Some(at("2026-09-26T15:00:00Z")), Some(at("2026-09-28T15:00:00Z")))
        );
        assert_eq!(window("stall three days ago"), window("stall 3 days ago"));
        assert_eq!(words("stall 3 days ago"), vec!["stall"]);
    }

    #[test]
    fn a_few_days_ago_is_one_to_seven_days_back() {
        assert_eq!(
            window("fixed the plugins stall, a few days ago"),
            (Some(at("2026-09-23T15:00:00Z")), Some(at("2026-09-29T15:00:00Z")))
        );
        assert_eq!(words("fixed the plugins stall, a few days ago"), vec!["fixed", "plugins", "stall"]);
    }

    #[test]
    fn weeks_and_this_week() {
        assert_eq!(window("last week"), (Some(at("2026-09-16T15:00:00Z")), Some(at(NOW))));
        assert_eq!(window("this week"), (Some(at("2026-09-23T15:00:00Z")), Some(at(NOW))));
        assert_eq!(
            window("2 weeks ago"),
            (Some(at("2026-09-09T15:00:00Z")), Some(at("2026-09-23T15:00:00Z")))
        );
    }

    #[test]
    fn last_month_is_the_previous_calendar_month() {
        assert_eq!(
            window("last month"),
            (Some(at("2026-08-01T00:00:00Z")), Some(at("2026-09-01T00:00:00Z")))
        );
        let january = parse("last month", at("2026-01-10T00:00:00Z"));
        assert_eq!(january.since, Some(at("2025-12-01T00:00:00Z")));
    }

    #[test]
    fn a_month_name_is_its_most_recent_occurrence() {
        assert_eq!(
            window("the auth refactor in march"),
            (Some(at("2026-03-01T00:00:00Z")), Some(at("2026-04-01T00:00:00Z")))
        );
        assert_eq!(words("the auth refactor in march"), vec!["auth", "refactor"]);
        // December has not come yet this year, so it is last year's.
        assert_eq!(window("december").0, Some(at("2025-12-01T00:00:00Z")));
        // "may" is a verb unless a preposition makes it a month.
        assert_eq!(window("it may stall"), (None, None));
        assert_eq!(window("in may").0, Some(at("2026-05-01T00:00:00Z")));
    }

    #[test]
    fn recently_is_two_weeks() {
        assert_eq!(window("recently"), (Some(at("2026-09-16T15:00:00Z")), Some(at(NOW))));
    }

    #[test]
    fn a_weekday_is_its_last_occurrence() {
        assert_eq!(
            window("on monday"),
            (Some(at("2026-09-28T00:00:00Z")), Some(at("2026-09-29T00:00:00Z")))
        );
        // Today is Wednesday: "last wednesday" is a week back, not today.
        assert_eq!(window("last wednesday").0, Some(at("2026-09-23T00:00:00Z")));
    }

    #[test]
    fn harness_words_become_a_filter() {
        let parsed = parse("codex sandbox bug", at(NOW));
        assert_eq!(parsed.harness.as_deref(), Some("codex"));
        assert_eq!(words("codex sandbox bug"), vec!["sandbox", "bug"]);
    }

    #[test]
    fn quoted_strings_are_phrases() {
        let parsed = parse("the \"catalog stall\" fix", at(NOW));
        let phrase = parsed.terms.iter().find(|term| term.phrase).unwrap();
        assert_eq!(phrase.text, "catalog stall");
        assert_eq!(phrase.to_match(), "\"catalog stall\"");
        assert_eq!(words("the \"catalog stall\" fix"), vec!["fix", "catalog stall"]);
    }

    #[test]
    fn stopwords_and_filler_are_removed() {
        assert_eq!(
            words("the one where we talked about something with the router"),
            vec!["router"]
        );
    }

    #[test]
    fn fts_operator_soup_is_inert() {
        let parsed = parse("\" OR NEAR(secret) * AND col:body (x", at(NOW));
        for term in &parsed.terms {
            let expression = term.to_match();
            assert!(expression.starts_with('"'), "{expression}");
            assert!(!term.text.contains('"'));
            assert!(!term.text.contains(':'));
            assert!(!term.text.contains('('));
            assert!(!term.text.contains('*'));
        }
        assert!(parsed.terms.iter().all(|term| !matches!(term.text.as_str(), "or" | "and" | "near" | "not")));
    }

    #[test]
    fn only_a_still_open_last_token_is_a_prefix_candidate() {
        let open = parse("plugins sta", at(NOW));
        assert!(open.terms.last().unwrap().prefix);
        assert!(!open.terms[0].prefix);
        let closed = parse("plugins sta ", at(NOW));
        assert!(closed.terms.iter().all(|term| !term.prefix));
    }

    fn vocab_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn chat_with(db: &Connection, id: &str, texts: &[&str]) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind)
             VALUES(?1,NULL,'codex','Chat','idle','estimated','direct')",
            params![id],
        )
        .unwrap();
        for text in texts {
            crate::store::append_session_entry(
                db, id, None, "user.message", &serde_json::json!({"text": text}), None, "eligible", None,
            )
            .unwrap();
        }
    }

    #[test]
    fn ranking_keeps_the_rarest_known_terms() {
        let (_dir, db) = vocab_db();
        for index in 0..6 {
            chat_with(&db, &format!("c{index}"), &["the plugin build is slow"]);
        }
        chat_with(&db, "rare", &["plugin catalog stall"]);
        let parsed = parse("plugin catalog zzzunknown", at(NOW));
        let ranked = rank_terms(&db, &parsed).unwrap();
        let texts: Vec<_> = ranked.iter().map(|term| term.text.as_str()).collect();
        assert_eq!(texts, vec!["catalog", "plugin"], "rarest first, unknown dropped");
        assert_eq!(rank_terms_counted(&db, &parsed).unwrap().1, 1);
    }

    #[test]
    fn a_finished_last_word_stays_exact_and_a_fragment_becomes_a_prefix() {
        let (_dir, db) = vocab_db();
        chat_with(&db, "a", &["catalog stall"]);
        let exact = rank_terms(&db, &parse("stall", at(NOW))).unwrap();
        assert_eq!(exact[0].to_match(), "\"stall\"");
        let fragment = rank_terms(&db, &parse("catalog sta", at(NOW))).unwrap();
        assert!(fragment.iter().any(|term| term.to_match() == "\"sta\"*"));
    }

    #[test]
    fn an_unknown_word_retries_as_a_prefix_of_its_plural() {
        let (_dir, db) = vocab_db();
        chat_with(&db, "a", &["fanning out subagents hit the limit"]);
        let (ranked, unknown) = rank_terms_counted(&db, &parse("subagent limit ", at(NOW))).unwrap();
        assert!(ranked.iter().any(|term| term.to_match() == "\"subagent\"*"), "{ranked:?}");
        assert_eq!(unknown, 0);
        let (_, unknown) = rank_terms_counted(&db, &parse("zzz limit ", at(NOW))).unwrap();
        assert_eq!(unknown, 1, "a short unknown word stays unknown");
    }

    #[test]
    fn at_most_six_terms_survive() {
        let (_dir, db) = vocab_db();
        chat_with(&db, "a", &["alpha bravo charlie delta echo foxtrot golf hotel"]);
        let ranked = rank_terms(&db, &parse("alpha bravo charlie delta echo foxtrot golf hotel ", at(NOW))).unwrap();
        assert_eq!(ranked.len(), MAX_TERMS);
    }
}
