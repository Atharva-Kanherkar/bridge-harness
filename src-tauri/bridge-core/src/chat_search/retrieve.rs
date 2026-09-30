//! T1: which chats the index thinks you mean, with no model.
//!
//! Two bounded MATCH queries, both across every chat: one over entries
//! (`session_entry_fts`, whose rows already carry a `session_id`, so going
//! global needed no reindex) and one over the per-chat digests. Each costs the
//! posting lists of the chosen terms plus a `LIMIT`, independent of how many
//! chats exist. The two ranked lists are fused per chat by reciprocal rank,
//! with a small bonus for several matching entries and a smaller one for
//! recency, which only breaks ties.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use super::index::HIDDEN_KINDS_SQL;
use super::parse::{ParsedQuery, Term};
use crate::BridgeError;

/// Entry hits read per query. Bounds the group step whatever the corpus size.
pub const ENTRY_LIMIT: i64 = 200;
/// Digest hits read per query.
pub const DIGEST_LIMIT: i64 = 50;
/// Fewer chats than this under AND relaxes the query to OR.
pub const RELAX_BELOW: usize = 4;
/// The reciprocal-rank constant from the RRF paper; large enough that rank 1
/// and rank 2 are close, which is the point.
const RRF_K: f64 = 60.0;
/// A digest match says the chat is *about* the terms, which one passing entry
/// does not.
const DIGEST_WEIGHT: f64 = 2.0;
const ENTRY_WEIGHT: f64 = 1.0;
const MULTI_HIT_WEIGHT: f64 = 0.25;
/// After relaxing to OR, a chat that matched every term under the AND is
/// worth more than one that matched a single common word; rank fusion alone
/// cannot see that, since it only reads positions.
const COVERAGE_WEIGHT: f64 = 2.0;
const RECENCY_WEIGHT: f64 = 0.1;
const RECENCY_DAYS: f64 = 30.0;
/// Top score over second must reach this for the index to call itself sure.
pub const CONFIDENCE_RATIO: f64 = 1.5;
pub const SNIPPET_CHARS: usize = 160;

/// One chat the index ranked, with everything a card or the model needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub session_id: String,
    pub title: String,
    pub harness: String,
    pub workspace_id: Option<String>,
    pub workspace_title: Option<String>,
    pub last_active_at: String,
    pub started_at: Option<String>,
    /// The chat's best-ranked matching entry, so a model-bound snippet can be
    /// cut from a redacted whole body rather than from an FTS fragment.
    pub best_entry_id: Option<String>,
    pub entry_matches: u32,
    pub digest_match: bool,
    /// Matched every searched term, in one entry or in its digest. After an
    /// OR relaxation only some candidates have.
    pub covered_all: bool,
    pub snippet: String,
    pub score: f64,
    pub archived: bool,
    pub ended: bool,
}

impl Candidate {
    /// Entry matches plus one for a digest match: a chat whose title says
    /// the words counts as a match even when no single message does.
    pub fn match_count(&self) -> u32 {
        self.entry_matches + u32::from(self.digest_match)
    }

    pub fn why(&self) -> String {
        let messages = match self.entry_matches {
            0 => None,
            1 => Some("1 message matches".to_owned()),
            count => Some(format!("{count} messages match")),
        };
        match (self.digest_match, messages) {
            (true, Some(messages)) => format!("topic and {messages}"),
            (true, None) => "topic matches".to_owned(),
            (false, Some(messages)) => messages,
            (false, None) => "active in that window".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Retrieval {
    pub candidates: Vec<Candidate>,
    pub terms: Vec<Term>,
    /// Whether the AND query found too little and OR was used.
    pub relaxed: bool,
    /// Typed words the index has never seen, set by the caller from
    /// [`super::parse::rank_terms_counted`].
    pub unknown_terms: usize,
}

impl Retrieval {
    /// The gate that decides whether a model could add anything.
    pub fn confident(&self) -> bool {
        // An answer to part of the question is not a clear answer: a word
        // the index never saw, or a winner that matched only some terms,
        // leaves the rest to the model.
        if self.unknown_terms > 0 || self.candidates.first().is_some_and(|first| !first.covered_all) {
            return false;
        }
        match self.candidates.as_slice() {
            [] => false,
            [_] => true,
            [first, second, ..] => {
                second.score <= 0.0
                    || (first.score / second.score >= CONFIDENCE_RATIO && first.match_count() >= 2)
            }
        }
    }
}

/// SQL filter shared by both queries: top-level, visible chats in the
/// harness and window asked for. `?2` harness, `?3` since, `?4` until.
fn chat_filter() -> String {
    format!(
        "s.parent_session_id IS NULL
         AND coalesce(s.kind, 'direct') NOT IN ({HIDDEN_KINDS_SQL})
         AND (?2 IS NULL OR s.harness = ?2)"
    )
}

struct EntryHit {
    session_id: String,
    entry_id: String,
    strength: f64,
    snippet: String,
}

struct DigestHit {
    session_id: String,
    snippet: String,
}

fn match_expression(terms: &[Term], joiner: &str) -> String {
    terms
        .iter()
        .map(Term::to_match)
        .collect::<Vec<_>>()
        .join(joiner)
}

fn rfc3339(at: Option<DateTime<Utc>>) -> Option<String> {
    at.map(|value| value.to_rfc3339())
}

fn entry_hits(
    db: &Connection,
    expression: &str,
    parsed: &ParsedQuery,
) -> Result<Vec<EntryHit>, BridgeError> {
    // The window applies to the matching entry's own time: "the stall, last
    // week" means the stall was discussed last week, whatever the chat did
    // afterwards.
    // Without a window the entry row is not needed, and skipping the join
    // saves one primary-key lookup per posting.
    let windowed = parsed.since.is_some() || parsed.until.is_some();
    let sql = format!(
        "SELECT f.session_id, f.entry_id, bm25(session_entry_fts), snippet(session_entry_fts, 3, '', '', '…', 24)
         FROM session_entry_fts f
         JOIN sessions s ON s.id = f.session_id
         {entry_join}
         WHERE session_entry_fts MATCH ?1
           AND {filter}
           AND {window}
         ORDER BY bm25(session_entry_fts)
         LIMIT ?5",
        filter = chat_filter(),
        entry_join = if windowed { "JOIN session_entries e ON e.id = f.entry_id" } else { "" },
        window = if windowed {
            "(?3 IS NULL OR e.created_at >= ?3) AND (?4 IS NULL OR e.created_at < ?4)"
        } else {
            "?3 IS NULL AND ?4 IS NULL"
        },
    );
    let mut statement = db.prepare_cached(&sql)?;
    let rows = statement.query_map(
        params![
            expression,
            parsed.harness,
            rfc3339(parsed.since),
            rfc3339(parsed.until),
            ENTRY_LIMIT
        ],
        |row| {
            Ok(EntryHit {
                session_id: row.get(0)?,
                entry_id: row.get(1)?,
                // bm25 is negative, better is lower.
                strength: -row.get::<_, f64>(2)?,
                snippet: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            })
        },
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// When a chat was last active: its newest entry, else when it started.
const LAST_ACTIVE_SQL: &str = "coalesce(
    (SELECT e.created_at FROM session_entries e WHERE e.session_id = s.id ORDER BY e.sequence DESC LIMIT 1),
    s.started_at, '')";

fn digest_hits(
    db: &Connection,
    expression: &str,
    parsed: &ParsedQuery,
) -> Result<Vec<DigestHit>, BridgeError> {
    // A digest has no single time, so the window asks whether the chat was
    // alive during it: started before it ended, last active after it began.
    let sql = format!(
        "SELECT d.session_id, snippet(chat_digest_fts, 0, '', '', '…', 24)
         FROM chat_digest_fts
         JOIN chat_digests d ON d.id = chat_digest_fts.rowid
         JOIN sessions s ON s.id = d.session_id
         WHERE chat_digest_fts MATCH ?1
           AND {filter}
           AND (?3 IS NULL OR {LAST_ACTIVE_SQL} >= ?3)
           AND (?4 IS NULL OR coalesce(s.started_at, '') < ?4)
         ORDER BY bm25(chat_digest_fts)
         LIMIT ?5",
        filter = chat_filter()
    );
    let mut statement = db.prepare_cached(&sql)?;
    let rows = statement.query_map(
        params![
            expression,
            parsed.harness,
            rfc3339(parsed.since),
            rfc3339(parsed.until),
            DIGEST_LIMIT
        ],
        |row| {
            Ok(DigestHit {
                session_id: row.get(0)?,
                snippet: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            })
        },
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Chats active in the window, newest first: the answer to a query that was
/// only a time or a harness ("yesterday in codex").
fn recent_chats(db: &Connection, parsed: &ParsedQuery, limit: usize) -> Result<Vec<String>, BridgeError> {
    let sql = format!(
        "SELECT s.id FROM sessions s
         WHERE {filter}
           AND (?3 IS NULL OR {LAST_ACTIVE_SQL} >= ?3)
           AND (?4 IS NULL OR coalesce(s.started_at, '') < ?4)
         ORDER BY {LAST_ACTIVE_SQL} DESC
         LIMIT ?5",
        filter = chat_filter()
    );
    // `?1` (the MATCH in the other queries) is unused here; binding it keeps
    // one numbering for the shared filter.
    let mut statement = db.prepare(&sql)?;
    let rows = statement.query_map(
        params![
            Option::<String>::None,
            parsed.harness,
            rfc3339(parsed.since),
            rfc3339(parsed.until),
            limit as i64
        ],
        |row| row.get(0),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Card fields for one chat. `None` when the chat is gone or not searchable.
pub fn describe(db: &Connection, session_id: &str) -> Result<Option<Candidate>, BridgeError> {
    let sql = format!(
        "SELECT s.id, coalesce(nullif(s.title, ''), s.label), s.harness, s.workspace_id, w.title,
                {LAST_ACTIVE_SQL}, s.started_at, s.archived_at IS NOT NULL, s.ended_at IS NOT NULL
         FROM sessions s LEFT JOIN workspaces w ON w.id = s.workspace_id
         WHERE s.id = ?1 AND s.parent_session_id IS NULL
           AND coalesce(s.kind, 'direct') NOT IN ({HIDDEN_KINDS_SQL})"
    );
    Ok(db
        .prepare_cached(&sql)?
        .query_row(params![session_id], |row| {
            Ok(Candidate {
                session_id: row.get(0)?,
                title: row.get(1)?,
                harness: row.get(2)?,
                workspace_id: row.get(3)?,
                workspace_title: row.get(4)?,
                last_active_at: row.get(5)?,
                started_at: row.get(6)?,
                best_entry_id: None,
                entry_matches: 0,
                digest_match: false,
                covered_all: false,
                snippet: String::new(),
                score: 0.0,
                archived: row.get(7)?,
                ended: row.get(8)?,
            })
        })
        .optional()?)
}

pub fn truncate_chars(text: &str, limit: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= limit {
        flat
    } else {
        format!("{}…", flat.chars().take(limit.saturating_sub(1)).collect::<String>())
    }
}

fn age_days(last_active_at: &str, now: DateTime<Utc>) -> f64 {
    DateTime::parse_from_rfc3339(last_active_at)
        .map(|at| (now - at.with_timezone(&Utc)).num_seconds().max(0) as f64 / 86_400.0)
        .unwrap_or(365.0)
}

/// Rank chats for `terms` (already ranked by the parser) under `parsed`'s
/// filters. Returns at most `limit` candidates.
pub fn retrieve(
    db: &Connection,
    parsed: &ParsedQuery,
    terms: &[Term],
    limit: usize,
    now: DateTime<Utc>,
) -> Result<Retrieval, BridgeError> {
    if terms.is_empty() {
        let mut candidates = Vec::new();
        if parsed.has_filter() {
            for id in recent_chats(db, parsed, limit)? {
                if let Some(mut candidate) = describe(db, &id)? {
                    candidate.covered_all = true;
                    candidate.score = RECENCY_WEIGHT * (-age_days(&candidate.last_active_at, now) / RECENCY_DAYS).exp() / (RRF_K + 1.0);
                    candidates.push(candidate);
                }
            }
        }
        return Ok(Retrieval {
            candidates,
            terms: Vec::new(),
            relaxed: false,
            unknown_terms: 0,
        });
    }

    let and = match_expression(terms, " AND ");
    let mut entries = entry_hits(db, &and, parsed)?;
    let mut digests = digest_hits(db, &and, parsed)?;
    let mut relaxed = false;
    let chats = |entries: &[EntryHit], digests: &[DigestHit]| {
        let mut ids: Vec<String> = entries.iter().map(|hit| hit.session_id.clone()).collect();
        ids.extend(digests.iter().map(|hit| hit.session_id.clone()));
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let mut covered: HashSet<String> = chats(&entries, &digests).into_iter().collect();
    if terms.len() > 1 && covered.len() < RELAX_BELOW {
        // Rare terms still dominate an OR through their IDF in bm25, so the
        // relaxed list stays led by what the AND would have found.
        let or = match_expression(terms, " OR ");
        entries = entry_hits(db, &or, parsed)?;
        digests = digest_hits(db, &or, parsed)?;
        relaxed = true;
        // A chat can say every term without any one row saying them all:
        // the product name in its title, the symptom in a message. Coverage
        // is a property of the chat, so it is checked term by term. Each
        // check keeps the same row limit, so a very common term can only
        // under-report coverage, which errs toward asking the model.
        let mut per_term: Option<HashSet<String>> = None;
        for term in terms {
            let expression = term.to_match();
            let mut sessions: HashSet<String> = entry_hits(db, &expression, parsed)?
                .into_iter()
                .map(|hit| hit.session_id)
                .collect();
            sessions.extend(digest_hits(db, &expression, parsed)?.into_iter().map(|hit| hit.session_id));
            per_term = Some(match per_term {
                None => sessions,
                Some(previous) => previous.intersection(&sessions).cloned().collect(),
            });
        }
        covered.extend(per_term.unwrap_or_default());
    }

    // Group entry hits by chat, preserving bm25 order for the rank.
    let mut entry_rank: Vec<String> = Vec::new();
    let mut per_chat: HashMap<String, (Vec<f64>, String, String)> = HashMap::new();
    for hit in &entries {
        let slot = per_chat.entry(hit.session_id.clone()).or_insert_with(|| {
            entry_rank.push(hit.session_id.clone());
            (Vec::new(), hit.snippet.clone(), hit.entry_id.clone())
        });
        slot.0.push(hit.strength);
    }
    let strongest = entries
        .iter()
        .map(|hit| hit.strength)
        .fold(f64::MIN, f64::max)
        .max(f64::EPSILON);

    let mut scores: HashMap<String, f64> = HashMap::new();
    for (rank, id) in entry_rank.iter().enumerate() {
        *scores.entry(id.clone()).or_default() += ENTRY_WEIGHT / (RRF_K + rank as f64 + 1.0);
        let strengths = &per_chat[id].0;
        let top: f64 = strengths.iter().take(3).map(|strength| strength / strongest).sum();
        *scores.entry(id.clone()).or_default() += MULTI_HIT_WEIGHT * top / (RRF_K + 1.0);
    }
    if relaxed {
        for id in &covered {
            *scores.entry(id.clone()).or_default() += COVERAGE_WEIGHT / (RRF_K + 1.0);
        }
    }
    let mut digest_snippets: HashMap<String, String> = HashMap::new();
    for (rank, hit) in digests.iter().enumerate() {
        *scores.entry(hit.session_id.clone()).or_default() += DIGEST_WEIGHT / (RRF_K + rank as f64 + 1.0);
        digest_snippets.insert(hit.session_id.clone(), hit.snippet.clone());
    }

    let mut candidates = Vec::new();
    for (id, fused) in scores {
        let Some(mut candidate) = describe(db, &id)? else {
            continue;
        };
        let recency = RECENCY_WEIGHT * (-age_days(&candidate.last_active_at, now) / RECENCY_DAYS).exp() / (RRF_K + 1.0);
        candidate.score = fused + recency;
        candidate.entry_matches = per_chat.get(&id).map_or(0, |(strengths, _, _)| strengths.len() as u32);
        candidate.best_entry_id = per_chat.get(&id).map(|(_, _, entry_id)| entry_id.clone());
        candidate.digest_match = digest_snippets.contains_key(&id);
        candidate.covered_all = !relaxed || covered.contains(&id);
        let snippet = per_chat
            .get(&id)
            .map(|(_, snippet, _)| snippet.clone())
            .filter(|snippet| !snippet.trim().is_empty())
            .or_else(|| digest_snippets.get(&id).cloned())
            .unwrap_or_default();
        candidate.snippet = truncate_chars(&snippet, SNIPPET_CHARS);
        candidates.push(candidate);
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.last_active_at.cmp(&left.last_active_at))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    candidates.truncate(limit);
    Ok(Retrieval {
        candidates,
        terms: terms.to_vec(),
        relaxed,
        unknown_terms: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::super::parse::{parse, rank_terms_counted};
    use super::*;
    use crate::store::{self, append_session_entry};
    use serde_json::json;

    const NOW: &str = "2026-09-30T15:00:00Z";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(NOW).unwrap().with_timezone(&Utc)
    }

    fn fresh() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db = store::open(&dir.path().join("bridge.db")).unwrap();
        (dir, db)
    }

    fn chat(db: &Connection, id: &str, harness: &str, title: &str) {
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title,started_at)
             VALUES(?1,NULL,?2,'Chat','idle','estimated','direct',?3,'2026-01-01T00:00:00+00:00')",
            params![id, harness, title],
        )
        .unwrap();
    }

    fn say(db: &Connection, id: &str, kind: &str, text: &str) {
        append_session_entry(db, id, None, kind, &json!({"text": text}), None, "eligible", None).unwrap();
    }

    fn say_at(db: &Connection, id: &str, text: &str, at: &str) {
        say(db, id, "user.message", text);
        db.execute(
            "UPDATE session_entries SET created_at=?2 WHERE session_id=?1 AND sequence=(SELECT max(sequence) FROM session_entries WHERE session_id=?1)",
            params![id, at],
        )
        .unwrap();
    }

    fn search(db: &Connection, query: &str) -> Retrieval {
        let parsed = parse(query, now());
        let (terms, unknown) = rank_terms_counted(db, &parsed).unwrap();
        let mut retrieval = retrieve(db, &parsed, &terms, 4, now()).unwrap();
        retrieval.unknown_terms = unknown;
        retrieval
    }

    fn ids(retrieval: &Retrieval) -> Vec<&str> {
        retrieval.candidates.iter().map(|candidate| candidate.session_id.as_str()).collect()
    }

    #[test]
    fn finds_chats_across_sessions_with_one_query() {
        let (_dir, db) = fresh();
        chat(&db, "a", "codex", "Auth work");
        chat(&db, "b", "claude", "Release notes");
        chat(&db, "c", "codex", "Unrelated");
        say(&db, "a", "user.message", "rotate the refresh token");
        say(&db, "b", "assistant.message", "the refresh token rotation shipped");
        say(&db, "c", "user.message", "paint the bikeshed");
        let found = search(&db, "refresh token");
        let mut found_ids = ids(&found);
        found_ids.sort_unstable();
        assert_eq!(found_ids, vec!["a", "b"]);
    }

    #[test]
    fn digest_title_match_outranks_single_passing_mention() {
        let (_dir, db) = fresh();
        chat(&db, "topic", "codex", "Plugins catalog stall");
        say(&db, "topic", "user.message", "the marketplace hangs on open");
        say(&db, "topic", "assistant.message", "the catalog stall comes from an unbounded CLI call");
        chat(&db, "passing", "codex", "Weekly notes");
        say(&db, "passing", "user.message", "weekly sync agenda");
        say(&db, "passing", "user.message", "unrelated, but the catalog stall again maybe");
        let found = search(&db, "catalog stall");
        assert_eq!(ids(&found)[0], "topic");
    }

    #[test]
    fn relaxes_to_or_when_and_is_too_narrow() {
        let (_dir, db) = fresh();
        chat(&db, "a", "codex", "One");
        chat(&db, "b", "codex", "Two");
        say(&db, "a", "user.message", "sidecar crash");
        say(&db, "b", "user.message", "sidecar restart loop");
        let found = search(&db, "sidecar crash loop");
        assert!(found.relaxed);
        let mut found_ids = ids(&found);
        found_ids.sort_unstable();
        assert_eq!(found_ids, vec!["a", "b"]);
    }

    #[test]
    fn time_and_harness_filters_apply_in_sql() {
        let (_dir, db) = fresh();
        chat(&db, "old", "codex", "Old");
        chat(&db, "new", "codex", "New");
        chat(&db, "other", "claude", "Other");
        say_at(&db, "old", "migration plan", "2026-08-01T10:00:00+00:00");
        say_at(&db, "new", "migration plan", "2026-09-27T10:00:00+00:00");
        say_at(&db, "other", "migration plan", "2026-09-27T11:00:00+00:00");
        // Digest matches are dated by the chat's span; start them in the window.
        db.execute("UPDATE sessions SET started_at='2026-09-27T00:00:00+00:00' WHERE id IN ('new','other')", []).unwrap();
        db.execute("UPDATE sessions SET started_at='2026-08-01T00:00:00+00:00' WHERE id='old'", []).unwrap();
        assert_eq!(ids(&search(&db, "migration a few days ago in codex")), vec!["new"]);
        let everything = search(&db, "migration");
        assert_eq!(everything.candidates.len(), 3);
    }

    #[test]
    fn hidden_kinds_and_workers_are_never_hits() {
        let (_dir, db) = fresh();
        chat(&db, "visible", "codex", "Visible");
        say(&db, "visible", "user.message", "zeppelin");
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title,parent_session_id,depth)
             VALUES('worker',NULL,'codex','Worker','idle','estimated','direct','zeppelin','visible',1)",
            [],
        )
        .unwrap();
        say(&db, "worker", "worker.result", "zeppelin");
        db.execute(
            "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind,title)
             VALUES('brief',NULL,'claude','Brief','idle','reported','briefing','zeppelin')",
            [],
        )
        .unwrap();
        say(&db, "brief", "assistant.message", "zeppelin");
        assert_eq!(ids(&search(&db, "zeppelin")), vec!["visible"]);
    }

    #[test]
    fn archived_and_ended_chats_are_found_and_marked() {
        let (_dir, db) = fresh();
        chat(&db, "gone", "codex", "Gone");
        say(&db, "gone", "user.message", "quokka");
        db.execute("UPDATE sessions SET archived_at='2026-09-01T00:00:00Z', ended_at='2026-09-01T00:00:00Z' WHERE id='gone'", []).unwrap();
        let found = search(&db, "quokka");
        assert_eq!(ids(&found), vec!["gone"]);
        assert!(found.candidates[0].archived);
        assert!(found.candidates[0].ended);
    }

    #[test]
    fn non_indexed_entry_kinds_never_match() {
        let (_dir, db) = fresh();
        chat(&db, "s", "codex", "Chat title");
        append_session_entry(&db, "s", None, "tool.call", &json!({"text":"narwhal"}), None, "eligible", None).unwrap();
        append_session_entry(&db, "s", None, "usage.updated", &json!({"text":"narwhal"}), None, "hidden", None).unwrap();
        assert!(search(&db, "narwhal").candidates.is_empty());
    }

    #[test]
    fn gate_is_confident_for_a_clear_winner_and_not_for_a_tie() {
        let (_dir, db) = fresh();
        chat(&db, "winner", "codex", "Pelican migration");
        say(&db, "winner", "user.message", "pelican migration step one");
        say(&db, "winner", "user.message", "pelican migration step two");
        chat(&db, "also", "codex", "Notes");
        say(&db, "also", "user.message", "an aside about a pelican");
        let clear = search(&db, "pelican migration");
        assert_eq!(ids(&clear)[0], "winner");
        assert!(clear.confident(), "{:?}", clear.candidates.iter().map(|c| c.score).collect::<Vec<_>>());

        let (_dir, db) = fresh();
        chat(&db, "x", "codex", "Heron sync");
        chat(&db, "y", "codex", "Heron sync");
        say(&db, "x", "user.message", "heron sync");
        say(&db, "y", "user.message", "heron sync");
        assert!(!search(&db, "heron sync").confident());

        let (_dir, db) = fresh();
        chat(&db, "only", "codex", "Solo");
        say(&db, "only", "user.message", "axolotl");
        assert!(search(&db, "axolotl").confident(), "exactly one hit is clear");
        assert!(!search(&db, "nothing-matches-this").confident());
        // One hit on one of three typed words is a guess, not a clear answer.
        assert!(!search(&db, "axolotl printout zzzunknown").confident(), "an unknown word leaves the rest unanswered");

        let (_dir, db) = fresh();
        chat(&db, "w", "codex", "One");
        chat(&db, "p", "codex", "Two");
        say(&db, "w", "user.message", "wombat");
        say(&db, "p", "user.message", "printout");
        let partial = search(&db, "wombat printout");
        assert!(partial.relaxed);
        assert!(partial.candidates.iter().all(|candidate| !candidate.covered_all));
        assert!(!partial.confident(), "a winner that matched one of two words is not clear");
    }

    #[test]
    fn a_time_only_query_lists_chats_active_then() {
        let (_dir, db) = fresh();
        chat(&db, "y", "codex", "Yesterday's chat");
        say_at(&db, "y", "anything", "2026-09-29T09:00:00+00:00");
        chat(&db, "old", "codex", "Old chat");
        say_at(&db, "old", "anything", "2026-06-01T09:00:00+00:00");
        let found = search(&db, "yesterday");
        assert_eq!(ids(&found), vec!["y"]);
        assert_eq!(found.candidates[0].why(), "active in that window");
        assert!(search(&db, "the").candidates.is_empty(), "no terms and no filter is empty");
    }

    #[test]
    fn search_session_entries_is_unchanged() {
        let (_dir, db) = fresh();
        chat(&db, "a", "codex", "A");
        chat(&db, "b", "codex", "B");
        say(&db, "a", "user.message", "platypus");
        say(&db, "b", "user.message", "platypus");
        let scoped = crate::session_recall::search(&db, "a", "platypus", None).unwrap();
        assert_eq!(scoped.hits.len(), 1, "per-session recall stays per session");
        assert_eq!(search(&db, "platypus").candidates.len(), 2);
    }
}
