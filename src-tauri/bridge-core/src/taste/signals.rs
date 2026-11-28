//! The taste layer's decision log: one append-only record of what a person
//! did with what agents produced. A review finding kept, edited or dropped; a
//! posted finding the author later fixed; a comment thread resolved or left
//! to go stale; a tool request allowed or declined.
//!
//! Suppression, the rule learner, rule scores and the replay eval all read
//! this one log rather than re-deriving outcomes from review rows, thread
//! state and chat events each in their own way. The log copies normalised
//! facts; the review tables stay the source of truth for reviews.
//!
//! Each subject (a finding, a permission request) has at most one live signal
//! per source. A changed decision appends a new signal and marks the earlier
//! one superseded, so history stays readable and the same decision written
//! twice (a backfill after a live write, a re-review seeing the same resolved
//! thread) adds nothing.

use std::sync::LazyLock;

use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use uuid::Uuid;

use crate::BridgeError;

pub const SOURCE_REVIEW_DECISION: &str = "review_decision";
pub const SOURCE_REVIEW_FIXED: &str = "review_fixed";
pub const SOURCE_REVIEW_THREAD: &str = "review_thread";
pub const SOURCE_PERMISSION: &str = "permission";

/// How long a signal is kept. A year of reviews is enough to learn from, and
/// old taste is stale taste.
pub const RETENTION_DAYS: i64 = 365;
/// Longest title or body kept on a signal.
const MAX_TEXT_CHARS: usize = 4000;
/// Longest summary of a tool request's input kept on a permission signal.
const MAX_REQUEST_CHARS: usize = 600;

/// Schema 106: the log, and everything already triaged copied into it.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS taste_signals(
            id TEXT PRIMARY KEY,
            at TEXT NOT NULL,
            scope_key TEXT NOT NULL,
            source TEXT NOT NULL CHECK(source IN ('review_decision','review_fixed','review_thread','permission')),
            verdict TEXT NOT NULL CHECK(verdict IN
                ('kept','edited','dropped','fixed','resolved','ignored','allowed','declined')),
            subject_id TEXT NOT NULL,
            harness TEXT,
            path TEXT,
            language TEXT,
            severity TEXT,
            category TEXT,
            title TEXT,
            body TEXT,
            edited_body TEXT,
            fingerprint TEXT,
            job_id TEXT,
            pr_number INTEGER,
            session_id TEXT,
            detail_json TEXT,
            superseded_by TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_taste_signals_live
            ON taste_signals(source, subject_id) WHERE superseded_by IS NULL;
        CREATE INDEX IF NOT EXISTS idx_taste_signals_scope ON taste_signals(scope_key, at);
        CREATE INDEX IF NOT EXISTS idx_taste_signals_fingerprint ON taste_signals(fingerprint);",
    )?;
    backfill(transaction)?;
    Ok(())
}

/// One signal as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub id: String,
    pub at: String,
    pub scope_key: String,
    pub source: String,
    pub verdict: String,
    /// A finding's id for review signals; `<session>:<event>` for a permission.
    pub subject_id: String,
    pub harness: Option<String>,
    pub path: Option<String>,
    pub language: Option<String>,
    pub severity: Option<String>,
    pub category: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub edited_body: Option<String>,
    pub fingerprint: Option<String>,
    pub job_id: Option<String>,
    pub pr_number: Option<u64>,
    pub session_id: Option<String>,
    pub detail: Option<Value>,
    pub superseded_by: Option<String>,
}

const SIGNAL_COLUMNS: &str = "id, at, scope_key, source, verdict, subject_id, harness, path, language, severity, \
     category, title, body, edited_body, fingerprint, job_id, pr_number, session_id, detail_json, superseded_by";

fn map_signal(row: &rusqlite::Row<'_>) -> rusqlite::Result<Signal> {
    Ok(Signal {
        id: row.get(0)?,
        at: row.get(1)?,
        scope_key: row.get(2)?,
        source: row.get(3)?,
        verdict: row.get(4)?,
        subject_id: row.get(5)?,
        harness: row.get(6)?,
        path: row.get(7)?,
        language: row.get(8)?,
        severity: row.get(9)?,
        category: row.get(10)?,
        title: row.get(11)?,
        body: row.get(12)?,
        edited_body: row.get(13)?,
        fingerprint: row.get(14)?,
        job_id: row.get(15)?,
        pr_number: row.get::<_, Option<i64>>(16)?.map(|number| number.max(0) as u64),
        session_id: row.get(17)?,
        detail: row.get::<_, Option<String>>(18)?.and_then(|raw| serde_json::from_str(&raw).ok()),
        superseded_by: row.get(19)?,
    })
}

/// A scope's live signals, newest first.
pub fn live(db: &Connection, scope_key: &str, limit: u32) -> Result<Vec<Signal>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {SIGNAL_COLUMNS} FROM taste_signals
         WHERE scope_key=?1 AND superseded_by IS NULL
         ORDER BY at DESC, id DESC LIMIT ?2"
    ))?;
    let rows = statement.query_map(params![scope_key, i64::from(limit)], map_signal)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Every live signal of a scope, oldest first: the past a new review is
/// judged against.
pub fn live_in(db: &Connection, scope_key: &str) -> Result<Vec<Signal>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {SIGNAL_COLUMNS} FROM taste_signals
         WHERE scope_key=?1 AND superseded_by IS NULL ORDER BY at, id"
    ))?;
    let rows = statement.query_map(params![scope_key], map_signal)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Every signal ever written, superseded ones included, oldest first.
pub fn all(db: &Connection) -> Result<Vec<Signal>, BridgeError> {
    let mut statement = db.prepare(&format!("SELECT {SIGNAL_COLUMNS} FROM taste_signals ORDER BY at, id"))?;
    let rows = statement.query_map([], map_signal)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Every signal about one subject, superseded ones included, oldest first.
pub fn history(db: &Connection, source: &str, subject_id: &str) -> Result<Vec<Signal>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT {SIGNAL_COLUMNS} FROM taste_signals
         WHERE source=?1 AND subject_id=?2
         ORDER BY at, rowid"
    ))?;
    let rows = statement.query_map(params![source, subject_id], map_signal)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// What a producer knows; `append` adds the id, sanitises text and settles
/// supersession.
#[derive(Debug, Default)]
struct NewSignal {
    at: String,
    scope_key: String,
    source: &'static str,
    verdict: &'static str,
    subject_id: String,
    harness: Option<String>,
    path: Option<String>,
    severity: Option<String>,
    /// The finding's own category; guessed from its text when absent.
    category: Option<String>,
    title: Option<String>,
    body: Option<String>,
    edited_body: Option<String>,
    fingerprint: Option<String>,
    job_id: Option<String>,
    pr_number: Option<i64>,
    session_id: Option<String>,
    detail: Option<Value>,
}

/// Write `signal` unless the subject's live signal already says the same.
/// Returns the new signal's id, or `None` when nothing changed.
fn append(db: &Connection, signal: NewSignal) -> Result<Option<String>, BridgeError> {
    let edited_body = signal.edited_body.as_deref().map(clean);
    let current: Option<(String, String, Option<String>)> = db
        .query_row(
            "SELECT id, verdict, edited_body FROM taste_signals
             WHERE source=?1 AND subject_id=?2 AND superseded_by IS NULL",
            params![signal.source, signal.subject_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((_, verdict, body)) = &current {
        if verdict == signal.verdict && *body == edited_body {
            return Ok(None);
        }
    }
    let id = Uuid::new_v4().to_string();
    let at = instant(&signal.at);
    let language = signal.path.as_deref().and_then(language_of).map(str::to_owned);
    let category = category_or_guess(
        signal.category.as_deref(),
        signal.title.as_deref().unwrap_or_default(),
        signal.body.as_deref().unwrap_or_default(),
    );
    if let Some((previous, ..)) = &current {
        db.execute("UPDATE taste_signals SET superseded_by=?1 WHERE id=?2", params![id, previous])?;
    }
    db.execute(
        "INSERT INTO taste_signals(id, at, scope_key, source, verdict, subject_id, harness, path, language,
             severity, category, title, body, edited_body, fingerprint, job_id, pr_number, session_id, detail_json)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
        params![
            id,
            at,
            signal.scope_key,
            signal.source,
            signal.verdict,
            signal.subject_id,
            signal.harness,
            signal.path,
            language,
            signal.severity,
            category,
            signal.title.as_deref().map(clean),
            signal.body.as_deref().map(clean),
            edited_body,
            signal.fingerprint,
            signal.job_id,
            signal.pr_number,
            signal.session_id,
            signal.detail.map(|detail| detail.to_string()),
        ],
    )?;
    Ok(Some(id))
}

/// One format for every instant in the log (UTC, milliseconds, `Z`), the
/// reviews store's, so string order is time order whoever wrote the signal.
/// Text that is not an RFC 3339 instant is kept as given.
pub(crate) fn instant(raw: &str) -> String {
    DateTime::parse_from_rfc3339(raw)
        .map(|at| crate::reviews::store::stamp(at.with_timezone(&Utc)))
        .unwrap_or_else(|_| raw.to_string())
}

/// Text as the log keeps it: secrets redacted, trimmed, at most
/// [`MAX_TEXT_CHARS`] characters.
fn clean(text: &str) -> String {
    let redacted = crate::secret_interception::sanitize(text.trim()).text;
    redacted.chars().take(MAX_TEXT_CHARS).collect()
}

/// A finding as the log needs it, with its job's repository and the harness
/// of the pass that raised it.
struct FindingFacts {
    category: Option<String>,
    hidden: bool,
    path: String,
    severity: String,
    title: String,
    body: String,
    fingerprint: String,
    job_id: String,
    workspace_id: String,
    repo_owner: String,
    repo_name: String,
    pr_number: i64,
    harness: Option<String>,
}

fn finding_facts(db: &Connection, finding_id: &str) -> Result<Option<FindingFacts>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT f.path, f.severity, f.title, f.body, f.fingerprint, f.job_id,
                    j.workspace_id, j.repo_owner, j.repo_name, j.pr_number, p.harness,
                    f.category, f.hidden_reason IS NOT NULL
             FROM review_findings f
             JOIN review_jobs j ON j.id = f.job_id
             LEFT JOIN review_passes p ON p.id = f.pass_id
             WHERE f.id = ?1",
            params![finding_id],
            |row| {
                Ok(FindingFacts {
                    path: row.get(0)?,
                    severity: row.get(1)?,
                    title: row.get(2)?,
                    body: row.get(3)?,
                    fingerprint: row.get(4)?,
                    job_id: row.get(5)?,
                    workspace_id: row.get(6)?,
                    repo_owner: row.get(7)?,
                    repo_name: row.get(8)?,
                    pr_number: row.get(9)?,
                    harness: row.get(10)?,
                    category: row.get(11)?,
                    hidden: row.get(12)?,
                })
            },
        )
        .optional()?)
}

/// A repository's scope, or its workspace's when the name is not one GitHub
/// would give.
pub(crate) fn repository_scope(owner: &str, repo: &str, workspace_id: &str) -> String {
    crate::memory_ledger::project_scope(owner, repo).unwrap_or_else(|_| format!("workspace:{workspace_id}"))
}

fn review_signal(facts: FindingFacts, finding_id: &str, source: &'static str, verdict: &'static str, at: &str) -> NewSignal {
    NewSignal {
        at: at.to_string(),
        scope_key: repository_scope(&facts.repo_owner, &facts.repo_name, &facts.workspace_id),
        source,
        verdict,
        subject_id: finding_id.to_string(),
        harness: facts.harness,
        category: facts.category,
        path: Some(facts.path),
        severity: Some(facts.severity),
        title: Some(facts.title),
        body: Some(facts.body),
        fingerprint: Some(facts.fingerprint),
        job_id: Some(facts.job_id),
        pr_number: Some(facts.pr_number),
        ..NewSignal::default()
    }
}

/// A person kept, edited or dropped a finding. `pending` (undecided) writes
/// nothing. Called in the transaction that stores the decision.
pub(crate) fn record_decision(
    db: &Connection,
    finding_id: &str,
    decision: &str,
    edited_body: Option<&str>,
    at: &str,
) -> Result<Option<String>, BridgeError> {
    let verdict = match decision {
        "kept" => "kept",
        "edited" => "edited",
        "dropped" => "dropped",
        _ => return Ok(None),
    };
    let Some(facts) = finding_facts(db, finding_id)? else { return Ok(None) };
    let hidden = facts.hidden;
    let mut signal = review_signal(facts, finding_id, SOURCE_REVIEW_DECISION, verdict, at);
    signal.edited_body = edited_body.filter(|_| verdict == "edited").map(str::to_owned);
    // A finding the taste layer hid and the person brought back and kept is
    // the strongest sign the hiding was wrong.
    if hidden {
        signal.detail = Some(serde_json::json!({"wasHidden": true}));
    }
    append(db, signal)
}

/// A later review found that the PR's new commits fixed a kept or posted
/// finding: the strongest sign the finding was right.
pub(crate) fn record_fixed(db: &Connection, finding_id: &str, fixed_in_sha: &str, at: &str) -> Result<Option<String>, BridgeError> {
    let Some(facts) = finding_facts(db, finding_id)? else { return Ok(None) };
    let mut signal = review_signal(facts, finding_id, SOURCE_REVIEW_FIXED, "fixed", at);
    signal.detail = Some(serde_json::json!({"fixedInSha": fixed_in_sha}));
    append(db, signal)
}

/// How a posted finding's comment thread stands on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadOutcome {
    /// Someone resolved the thread.
    Resolved,
    /// The code moved on (the thread is outdated) with no reply and no fix:
    /// a weak sign the finding did not matter.
    Ignored,
}

pub(crate) fn record_thread(db: &Connection, finding_id: &str, outcome: ThreadOutcome, at: &str) -> Result<Option<String>, BridgeError> {
    let Some(facts) = finding_facts(db, finding_id)? else { return Ok(None) };
    let verdict = match outcome {
        ThreadOutcome::Resolved => "resolved",
        ThreadOutcome::Ignored => "ignored",
    };
    append(db, review_signal(facts, finding_id, SOURCE_REVIEW_THREAD, verdict, at))
}

/// What a thread on a posted finding says, if anything worth a signal: a
/// resolved thread, or an outdated one nobody answered on a finding that was
/// neither fixed nor dropped.
pub fn thread_outcome(resolved: bool, outdated: bool, replies: usize, fixed: bool, dropped: bool) -> Option<ThreadOutcome> {
    if resolved {
        Some(ThreadOutcome::Resolved)
    } else if outdated && replies == 0 && !fixed && !dropped {
        Some(ThreadOutcome::Ignored)
    } else {
        None
    }
}

/// A person answered a tool request in a chat. `accept` and
/// `acceptForSession` are allowed, `decline` declined; a cancel (stopping the
/// turn) says nothing about the request and writes nothing.
pub(crate) fn record_permission(
    db: &Connection,
    session_id: &str,
    event_id: i64,
    harness: &str,
    request: &Value,
    decision: &str,
    at: &str,
) -> Result<Option<String>, BridgeError> {
    let verdict = match decision {
        "accept" | "acceptForSession" => "allowed",
        "decline" => "declined",
        _ => return Ok(None),
    };
    let tool = request.get("tool_name").and_then(Value::as_str).map(str::to_owned);
    let input = request.get("tool_input").cloned().unwrap_or(Value::Null);
    let path = ["file_path", "path", "filePath"]
        .iter()
        .find_map(|key| input.get(key).and_then(Value::as_str))
        .map(str::to_owned);
    let summary = input
        .get("command")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| (!input.is_null()).then(|| input.to_string()))
        .map(|text| text.chars().take(MAX_REQUEST_CHARS).collect::<String>());
    append(
        db,
        NewSignal {
            at: at.to_string(),
            scope_key: session_scope(db, session_id)?,
            source: SOURCE_PERMISSION,
            verdict,
            subject_id: format!("{session_id}:{event_id}"),
            harness: Some(harness.to_string()),
            path,
            title: tool,
            body: summary,
            session_id: Some(session_id.to_string()),
            detail: Some(serde_json::json!({"decision": decision, "forSession": decision == "acceptForSession"})),
            ..NewSignal::default()
        },
    )
}

/// The scope a chat's decisions belong to: its repository when a review has
/// named it, otherwise its workspace, otherwise the account (a chat with no
/// project).
fn session_scope(db: &Connection, session_id: &str) -> Result<String, BridgeError> {
    let workspace: Option<String> = db
        .query_row("SELECT workspace_id FROM sessions WHERE id=?1", params![session_id], |row| row.get(0))
        .optional()?
        .flatten();
    let Some(workspace) = workspace else {
        return Ok(crate::memory_ledger::account_memory_scope().to_string());
    };
    let repository: Option<(String, String)> = db
        .query_row(
            "SELECT repo_owner, repo_name FROM review_jobs WHERE workspace_id=?1 ORDER BY created_at DESC LIMIT 1",
            params![workspace],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    Ok(match repository {
        Some((owner, repo)) => repository_scope(&owner, &repo, &workspace),
        None => format!("workspace:{workspace}"),
    })
}

/// Copy every decision and fix already stored into the log. Safe to run
/// again: a subject whose live signal already says the same is skipped.
pub(crate) fn backfill(db: &Connection) -> Result<usize, BridgeError> {
    let decided: Vec<(String, String, Option<String>, String)> = {
        let mut statement = db.prepare(
            "SELECT id, decision, edited_body, COALESCE(decided_at, created_at) FROM review_findings
             WHERE decision IN ('kept','edited','dropped') ORDER BY COALESCE(decided_at, created_at), id",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let fixed: Vec<(String, String, String)> = {
        let mut statement = db.prepare(
            "SELECT id, fixed_in_sha, COALESCE(decided_at, created_at) FROM review_findings
             WHERE fixed_in_sha IS NOT NULL ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut written = 0;
    for (id, decision, edited_body, at) in decided {
        written += usize::from(record_decision(db, &id, &decision, edited_body.as_deref(), &at)?.is_some());
    }
    for (id, sha, at) in fixed {
        written += usize::from(record_fixed(db, &id, &sha, &at)?.is_some());
    }
    Ok(written)
}

/// Drop signals older than [`RETENTION_DAYS`]. Evidence links to them stay,
/// so a rule keeps the count of what made it.
pub fn sweep(db: &Connection, now: DateTime<Utc>) -> Result<usize, BridgeError> {
    let cutoff = crate::reviews::store::stamp(now - Duration::days(RETENTION_DAYS));
    Ok(db.execute("DELETE FROM taste_signals WHERE at < ?1", params![cutoff])?)
}

/// The language a path is written in, from its extension.
pub fn language_of(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "rs" => "rust",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "rb" => "ruby",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "cs" => "csharp",
        "php" => "php",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "shell",
        "md" | "mdx" => "markdown",
        "json" => "json",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "css" | "scss" => "css",
        "html" => "html",
        _ => return None,
    })
}

/// The categories a finding can have.
pub const CATEGORIES: [&str; 9] =
    ["security", "correctness", "error-handling", "tests", "performance", "naming", "docs", "style", "other"];

/// A finding's own category when it is one of [`CATEGORIES`], else a guess
/// from its text.
pub fn category_or_guess(given: Option<&str>, title: &str, body: &str) -> &'static str {
    given
        .map(str::trim)
        .and_then(|given| CATEGORIES.iter().find(|known| known.eq_ignore_ascii_case(given)))
        .copied()
        .unwrap_or_else(|| category_of(title, body))
}

/// What a finding is about, as one of a fixed set of slugs, read from its
/// title first and its body only when the title says nothing: a finding
/// titled "No tests" that mentions the bug they would catch is about tests.
pub fn category_of(title: &str, body: &str) -> &'static str {
    classify(title).or_else(|| classify(body)).unwrap_or("other")
}

static PATTERNS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        (
            "security",
            r"\b(security|secrets?|credentials?|passwords?|api[- ]?keys?|hard-?coded (keys?|secrets?|passwords?|tokens?)|injection|xss|csrf|ssrf|traversal|vulnerab\w*|auth[nz]?|authenticat\w*|authoriz\w*|permissions?|sanitiz\w*|escap\w*)\b",
        ),
        (
            "correctness",
            r"\b(bugs?|off[- ]by[- ]one|incorrect\w*|wrong|race|overflow\w*|underflow\w*|bounds?|out[- ]of[- ]range|logic|regress\w*|crash\w*|deadlock\w*|null)\b",
        ),
        ("error-handling", r"\b(unwrap\w*|panic\w*|expect|errors?|unhandled|swallow\w*|exceptions?|fallible)\b"),
        ("tests", r"\b(tests?|testing|tested|coverage|assert\w*|fixtures?)\b"),
        ("performance", r"\b(performance|slow\w*|allocat\w*|quadratic|n\+1|inefficien\w*|latency|clones?|copies)\b"),
        ("naming", r"\b(naming|names?|rename\w*|snake_case|camel_?case|pascal_?case|conventions?)\b"),
        ("docs", r"\b(docs?|documentation|doc ?comments?|docstrings?|readme|comments?)\b"),
        ("style", r"\b(style|format\w*|readab\w*|idiom\w*|simplif\w*|nits?|cleanup|verbose|lint\w*)\b"),
    ]
    .into_iter()
    // Fixed patterns, checked by the category tests; one that failed to
    // compile would only leave its category unguessed.
    .filter_map(|(slug, pattern)| Regex::new(&format!("(?i){pattern}")).ok().map(|pattern| (slug, pattern)))
    .collect()
});

fn classify(text: &str) -> Option<&'static str> {
    PATTERNS.iter().find(|(_, pattern)| pattern.is_match(text)).map(|(slug, _)| *slug)
}

#[cfg(test)]
#[path = "signals_tests.rs"]
mod tests;
