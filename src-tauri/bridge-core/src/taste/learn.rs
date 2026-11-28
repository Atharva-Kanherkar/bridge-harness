//! Learn review rules from a person's decisions (#1201).
//!
//! Signals of one repository are grouped by the suppressor's notion of alike.
//! A group qualifies when at least [`MIN_GROUP`] signals from at least
//! [`MIN_PRS`] pull requests point the same way: dropped (a rule to stop
//! raising it) or kept, edited and fixed (a rule to raise it, or to write code
//! that way). The memory model drafts at most one plain-language rule per
//! group; each draft must use the evidence's own words, must not repeat a
//! rule the repository already has (a repeat lends it the evidence instead),
//! and lands as a `taste` proposal with its evidence linked, for the person to
//! approve. Nothing becomes active without them.
//!
//! At most [`MAX_NEW_PROPOSALS`] proposals per repository every
//! [`PROPOSAL_WINDOW_DAYS`] days, so the queue never floods.

use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::replay::{Candidate, Outcome};
use super::signals::{self, Signal};
use crate::memory_consolidation::ConsolidationModel;
use crate::BridgeError;

/// Signals a group needs before it is worth a rule.
pub const MIN_GROUP: f64 = 3.0;
/// Pull requests those signals must come from.
pub const MIN_PRS: usize = 2;
/// New learned proposals per repository per window.
pub const MAX_NEW_PROPOSALS: i64 = 5;
pub const PROPOSAL_WINDOW_DAYS: i64 = 14;
/// Longest rule kept.
pub const MAX_RULE_CHARS: usize = 240;
/// Word overlap at which a draft repeats a rule the repository has.
const REPEAT_OVERLAP: f64 = 0.6;
/// Share of a rule's words that must appear in its evidence.
const GROUNDED_SHARE: f64 = 0.5;
/// Longest finding text shown to the model per signal.
const MAX_EVIDENCE_CHARS: usize = 300;

pub const KIND_REVIEW_AVOID: &str = "review_avoid";
pub const KIND_REVIEW_PREFER: &str = "review_prefer";
pub const KIND_CODE_PREFER: &str = "code_prefer";

/// Schema 108: one row per learning run, so a group is offered again only
/// when it has grown.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS taste_learn_runs(
            id TEXT PRIMARY KEY,
            scope_key TEXT NOT NULL,
            at TEXT NOT NULL,
            status TEXT NOT NULL CHECK(status IN ('completed','failed','skipped')),
            offered INTEGER NOT NULL DEFAULT 0,
            proposed INTEGER NOT NULL DEFAULT 0,
            attached INTEGER NOT NULL DEFAULT 0,
            refused INTEGER NOT NULL DEFAULT 0,
            spend_microusd INTEGER NOT NULL DEFAULT 0,
            detail TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_taste_learn_runs_scope ON taste_learn_runs(scope_key, at);",
    )?;
    Ok(())
}

/// Which way a group points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// Dropped again and again: the reviewer should stop raising it.
    Avoid,
    /// Kept, edited or fixed again and again: raise it, or write code so.
    Prefer,
}

/// Signals that point the same way about the same kind of finding.
#[derive(Debug, Clone)]
pub struct Group {
    pub polarity: Polarity,
    pub signals: Vec<Signal>,
}

impl Group {
    fn prs(&self) -> BTreeSet<u64> {
        self.signals.iter().filter_map(|signal| signal.pr_number).collect()
    }

    /// The text the rule may draw its words from.
    fn evidence(&self) -> Vec<String> {
        self.signals
            .iter()
            .flat_map(|signal| [signal.title.clone(), signal.body.clone(), signal.edited_body.clone(), signal.path.clone()])
            .flatten()
            .collect()
    }
}

/// A signal as the suppressor compares it.
fn as_candidate(signal: &Signal) -> Candidate {
    Candidate {
        finding_id: signal.subject_id.clone(),
        job_id: signal.job_id.clone().unwrap_or_default(),
        scope_key: signal.scope_key.clone(),
        pr_number: signal.pr_number.unwrap_or_default(),
        reviewed_at: signal.at.clone(),
        path: signal.path.clone().unwrap_or_default(),
        severity: signal.severity.clone().unwrap_or_default(),
        category: signal.category.clone().unwrap_or_else(|| "other".into()),
        title: signal.title.clone().unwrap_or_default(),
        body: signal.body.clone().unwrap_or_default(),
        fingerprint: signal.fingerprint.clone().unwrap_or_default(),
        outcome: Outcome::default(),
    }
}

/// Drops of a critical or security finding never teach the reviewer to stop
/// raising it, as they never hide one.
fn exempt(signal: &Signal) -> bool {
    signal.severity.as_deref() == Some("critical") || signal.category.as_deref() == Some("security")
}

fn against(signal: &Signal) -> f64 {
    if exempt(signal) {
        return 0.0;
    }
    match signal.verdict.as_str() {
        "dropped" => 1.0,
        "ignored" => 0.5,
        _ => 0.0,
    }
}

fn wanted(signal: &Signal) -> bool {
    matches!(signal.verdict.as_str(), "kept" | "edited" | "fixed" | "resolved")
}

/// The groups of `scope` worth a rule: review signals no rule has claimed as
/// evidence, clustered by likeness to each cluster's first signal, keeping
/// those that point one way strongly enough.
pub fn groups(db: &Connection, scope: &str) -> Result<Vec<Group>, BridgeError> {
    let claimed: HashSet<String> = {
        let mut statement = db.prepare("SELECT DISTINCT signal_id FROM memory_evidence")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let open: Vec<Signal> = signals::live_in(db, scope)?
        .into_iter()
        .filter(|signal| signal.source != signals::SOURCE_PERMISSION && !claimed.contains(&signal.id))
        .collect();
    let mut clusters: Vec<Vec<Signal>> = Vec::new();
    for signal in open {
        let seat = clusters.iter_mut().find(|cluster| super::suppress::alike(&as_candidate(&cluster[0]), &signal));
        match seat {
            Some(cluster) => cluster.push(signal),
            None => clusters.push(vec![signal]),
        }
    }
    let mut found = Vec::new();
    for cluster in clusters {
        let weight_against: f64 = cluster.iter().map(against).sum();
        let weight_for = cluster.iter().filter(|signal| wanted(signal)).count() as f64;
        let (polarity, members): (Polarity, Vec<Signal>) = if weight_against >= MIN_GROUP && weight_against > weight_for {
            (Polarity::Avoid, cluster.into_iter().filter(|signal| against(signal) > 0.0).collect())
        } else if weight_for >= MIN_GROUP && weight_for > weight_against {
            (Polarity::Prefer, cluster.into_iter().filter(wanted).collect())
        } else {
            continue;
        };
        let group = Group { polarity, signals: members };
        if group.prs().len() >= MIN_PRS {
            found.push(group);
        }
    }
    // A stable order, so group numbers in a prompt do not depend on ids:
    // the earliest first, by title when two start at one instant.
    let key = |group: &Group| -> (String, String) {
        let first = group.signals.iter().min_by(|a, b| (&a.at, &a.title).cmp(&(&b.at, &b.title)));
        (first.map(|signal| signal.at.clone()).unwrap_or_default(), first.and_then(|signal| signal.title.clone()).unwrap_or_default())
    };
    found.sort_by_key(key);
    Ok(found)
}

/// One rule as the model drafts it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    /// 1-based index of the group it is for.
    pub group: usize,
    pub kind: String,
    pub rule: String,
    #[serde(default)]
    pub path_globs: Option<Vec<String>>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Drafts {
    #[serde(default)]
    rules: Vec<Draft>,
}

/// The instructions and the groups, as one prompt.
pub fn compose_prompt(scope: &str, groups: &[Group], existing: &[String]) -> String {
    let mut prompt = format!(
        "You turn a person's code review decisions in {scope} into rules an AI reviewer and AI coding agents will follow.\n\
         Each group below is findings a reviewer raised and what the person did with them.\n\n\
         For each group, write at most one rule, in the person's terms, under {MAX_RULE_CHARS} characters:\n\
         - kind `{KIND_REVIEW_AVOID}`: the reviewer should stop raising this (for dropped groups).\n\
         - kind `{KIND_REVIEW_PREFER}`: the reviewer should raise this, worded the way the person edited it (for kept groups).\n\
         - kind `{KIND_CODE_PREFER}`: how code in this repository should be written, when a kept group is about the code itself.\n\
         Use words from the findings. The rule says only what to do; put why in `rationale`.\n\
         Give `pathGlobs` only when every finding in the group is under those paths.\n\
         Leave a group out when it shows no general preference. Never repeat an existing rule.\n\n"
    );
    if !existing.is_empty() {
        prompt.push_str("Existing rules:\n");
        for rule in existing {
            prompt.push_str(&format!("- {rule}\n"));
        }
        prompt.push('\n');
    }
    for (index, group) in groups.iter().enumerate() {
        let prs: Vec<String> = group.prs().iter().map(|number| format!("#{number}")).collect();
        let verb = match group.polarity {
            Polarity::Avoid => "dropped",
            Polarity::Prefer => "kept, edited or fixed",
        };
        prompt.push_str(&format!("Group {} ({verb} {} times on {}):\n", index + 1, group.signals.len(), prs.join(", ")));
        for signal in &group.signals {
            let clip = |text: &str| text.chars().take(MAX_EVIDENCE_CHARS).collect::<String>().replace('\n', " ");
            prompt.push_str(&format!(
                "- [{}] {} ({}): {}",
                signal.verdict,
                signal.title.as_deref().unwrap_or_default(),
                signal.path.as_deref().unwrap_or_default(),
                clip(signal.body.as_deref().unwrap_or_default())
            ));
            if let Some(edited) = &signal.edited_body {
                prompt.push_str(&format!(" | edited to: {}", clip(edited)));
            }
            prompt.push('\n');
        }
        prompt.push('\n');
    }
    prompt.push_str(
        "End with exactly one fenced ```json block and nothing after it:\n\
         ```json\n{\"rules\":[{\"group\":1,\"kind\":\"review_avoid\",\"rule\":\"...\",\"pathGlobs\":[\"src/**\"],\"rationale\":\"one line\"}]}\n```\n",
    );
    prompt
}

/// The drafts in a model's answer: its last fenced json block, or the whole
/// answer when it is bare JSON.
pub fn parse(answer: &str) -> Result<Vec<Draft>, String> {
    let block = answer
        .rsplit_once("```json")
        .and_then(|(_, rest)| rest.split_once("```").map(|(block, _)| block))
        .unwrap_or(answer);
    serde_json::from_str::<Drafts>(block.trim()).map(|drafts| drafts.rules).map_err(|error| error.to_string())
}

/// The words a rule says what to do with, which no finding needs to contain.
const INSTRUCTION_WORDS: &[&str] = &[
    "always", "never", "not", "no", "don't", "dont", "do", "does", "ask", "asking", "asks", "stop", "raise", "raising",
    "flag", "flagging", "avoid", "skip", "skipping", "prefer", "use", "using", "instead", "request", "requests",
    "missing", "worth", "small", "like", "such", "should", "must", "keep", "write", "code", "review", "reviews",
    "reviewer", "finding", "findings", "it", "them", "these", "this", "that", "here", "there", "every", "any",
];

/// A term as grounding compares it: lowercase, a trailing plural `s` folded.
fn fold(term: &str) -> String {
    let term = term.to_lowercase();
    match term.strip_suffix('s') {
        Some(stem) if stem.len() >= 3 && !stem.ends_with('s') => stem.to_string(),
        _ => term,
    }
}

/// Whether most of a rule's content words, and at least two, appear in its
/// evidence. Instruction words ("stop", "ask", "never") are the rule's
/// grammar, not its claim, so they are not counted either way.
pub fn grounded(rule: &str, evidence: &[String]) -> bool {
    let known: BTreeSet<String> =
        evidence.iter().flat_map(|text| crate::memory_similarity::terms(text)).map(|term| fold(&term)).collect();
    let terms: BTreeSet<String> = crate::memory_similarity::terms(rule)
        .into_iter()
        .filter(|term| !INSTRUCTION_WORDS.contains(&term.as_str()))
        .map(|term| fold(&term))
        .collect();
    let shared = terms.iter().filter(|term| known.contains(*term)).count();
    shared >= 2 && shared as f64 >= GROUNDED_SHARE * terms.len() as f64
}

/// What one learning run did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LearnReport {
    pub offered: usize,
    pub proposed: Vec<String>,
    /// Drafts that repeated a rule the repository has: their evidence went to it.
    pub attached: usize,
    /// Drafts refused, each with why.
    pub refused: Vec<String>,
}

/// A taste rule already in the repository, as a repeat check needs it.
struct Existing {
    id: String,
    body: String,
}

fn existing_rules(db: &Connection, scope: &str) -> Result<Vec<Existing>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT id, body FROM memory_records
         WHERE scope_key=?1 AND layer='taste' AND status IN ('active','proposed','rejected') ORDER BY created_at",
    )?;
    let rows = statement.query_map(params![scope], |row| Ok(Existing { id: row.get(0)?, body: row.get(1)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Proposals the learner may still make in `scope` this window.
pub fn allowance(db: &Connection, scope: &str, now: DateTime<Utc>) -> Result<i64, BridgeError> {
    let since = (now - Duration::days(PROPOSAL_WINDOW_DAYS)).to_rfc3339();
    let made: i64 = db.query_row(
        "SELECT COUNT(*) FROM memory_records
         WHERE scope_key=?1 AND layer='taste' AND provenance='model_proposal' AND created_at >= ?2",
        params![scope, since],
        |row| row.get(0),
    )?;
    Ok((MAX_NEW_PROPOSALS - made).max(0))
}

/// When this repository's taste was last learned.
fn last_run(db: &Connection, scope: &str) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row(
            "SELECT at FROM taste_learn_runs WHERE scope_key=?1 AND status<>'skipped' ORDER BY at DESC LIMIT 1",
            params![scope],
            |row| row.get(0),
        )
        .optional()?)
}

/// Groups not offered before: at least one signal newer than the last run.
fn grown(groups: Vec<Group>, last: Option<&str>) -> Vec<Group> {
    match last {
        None => groups,
        Some(last) => groups.into_iter().filter(|group| group.signals.iter().any(|signal| signal.at.as_str() > last)).collect(),
    }
}

/// Learn `scope`'s taste once. `lock` gives the database for each short step;
/// the model runs between them without it.
pub fn run<'a, L>(lock: L, model: &mut dyn ConsolidationModel, scope: &str, now: DateTime<Utc>) -> Result<LearnReport, BridgeError>
where
    L: Fn() -> std::sync::MutexGuard<'a, Connection>,
{
    let (offered, existing, allowance) = {
        let db = lock();
        let last = last_run(&db, scope)?;
        let offered = grown(groups(&db, scope)?, last.as_deref());
        (offered, existing_rules(&db, scope)?, allowance(&db, scope, now)?)
    };
    let mut report = LearnReport { offered: offered.len(), ..LearnReport::default() };
    let at = signals::instant(&now.to_rfc3339());
    if offered.is_empty() || allowance == 0 {
        let db = lock();
        record(&db, scope, &at, "skipped", &report, 0, Some(if offered.is_empty() { "nothing new" } else { "allowance spent" }))?;
        return Ok(report);
    }
    let prompt = compose_prompt(scope, &offered, &existing.iter().map(|rule| rule.body.clone()).collect::<Vec<_>>());
    let output = match model.consolidate(&prompt) {
        Ok(output) => output,
        Err(error) => {
            let db = lock();
            record(&db, scope, &at, "failed", &report, 0, Some(&error.to_string()))?;
            return Err(error);
        }
    };
    let drafts = match parse(&output.text) {
        Ok(drafts) => drafts,
        Err(error) => {
            let db = lock();
            record(&db, scope, &at, "failed", &report, output.spend_microusd, Some(&format!("unreadable answer: {error}")))?;
            return Ok(report);
        }
    };
    let db = lock();
    let mut existing = existing;
    let mut used = HashSet::new();
    for draft in drafts {
        let Some(group) = draft.group.checked_sub(1).and_then(|index| offered.get(index)) else {
            report.refused.push(format!("group {} does not exist", draft.group));
            continue;
        };
        if used.contains(&draft.group) {
            report.refused.push(format!("group {} has a rule already", draft.group));
            continue;
        }
        let rule = draft.rule.trim().to_string();
        let kind_fits = match group.polarity {
            Polarity::Avoid => draft.kind == KIND_REVIEW_AVOID,
            Polarity::Prefer => draft.kind == KIND_REVIEW_PREFER || draft.kind == KIND_CODE_PREFER,
        };
        if !kind_fits {
            report.refused.push(format!("group {}: kind {} does not fit what the person did", draft.group, draft.kind));
            continue;
        }
        if rule.is_empty() || rule.chars().count() > MAX_RULE_CHARS {
            report.refused.push(format!("group {}: a rule is 1 to {MAX_RULE_CHARS} characters", draft.group));
            continue;
        }
        if crate::memory_ledger::require_body(&rule).is_err() {
            report.refused.push(format!("group {}: the rule cannot be stored", draft.group));
            continue;
        }
        if !grounded(&rule, &group.evidence()) {
            report.refused.push(format!("group {}: the rule is not in the evidence's words: {rule}", draft.group));
            continue;
        }
        let weights: Vec<(String, f64)> = group.signals.iter().map(|signal| (signal.id.clone(), against(signal).max(f64::from(u8::from(wanted(signal)))))).collect();
        if let Some(same) = existing.iter().find(|other| crate::reviews::merge::title_overlap(&rule, &other.body) >= REPEAT_OVERLAP) {
            for (signal, weight) in &weights {
                crate::memory_ledger::add_evidence(&db, &same.id, signal, *weight, &at)?;
            }
            report.attached += 1;
            used.insert(draft.group);
            continue;
        }
        if report.proposed.len() as i64 >= allowance {
            report.refused.push(format!("group {}: the allowance for {PROPOSAL_WINDOW_DAYS} days is spent", draft.group));
            continue;
        }
        let globs = draft.path_globs.filter(|globs| !globs.is_empty() && globs.iter().all(|glob| !glob.trim().is_empty()));
        let id = crate::memory_ledger::insert_taste_proposal(
            &db,
            scope,
            &draft.kind,
            &rule,
            draft.rationale.as_deref().map(str::trim).filter(|text| !text.is_empty()),
            globs.as_deref(),
            &weights,
            &at,
        )?;
        existing.push(Existing { id: id.clone(), body: rule });
        report.proposed.push(id);
        used.insert(draft.group);
    }
    record(&db, scope, &at, "completed", &report, output.spend_microusd, None)?;
    Ok(report)
}

fn record(
    db: &Connection,
    scope: &str,
    at: &str,
    status: &str,
    report: &LearnReport,
    spend: i64,
    detail: Option<&str>,
) -> Result<(), BridgeError> {
    let detail = detail.map(str::to_owned).or_else(|| (!report.refused.is_empty()).then(|| report.refused.join("; ")));
    db.execute(
        "INSERT INTO taste_learn_runs(id, scope_key, at, status, offered, proposed, attached, refused, spend_microusd, detail)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            Uuid::new_v4().to_string(),
            scope,
            at,
            status,
            report.offered as i64,
            report.proposed.len() as i64,
            report.attached as i64,
            report.refused.len() as i64,
            spend,
            detail
        ],
    )?;
    Ok(())
}

/// The repository a nightly run should learn now, if any: dreaming is on for
/// the account, the repository has signals no rule has claimed, and it was
/// not learned since tonight's dream hour. One repository per call, so a tick
/// spends at most one bounded model turn.
pub fn due_scope<Tz: chrono::TimeZone>(db: &Connection, now: &DateTime<Tz>) -> Result<Option<String>, BridgeError> {
    let settings = crate::memory_consolidation::settings(db, crate::memory_ledger::account_memory_scope())?;
    if !settings.enabled() {
        return Ok(None);
    }
    for scope in scopes_with_open_signals(db)? {
        let last = last_any_run(db, &scope)?
            .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
            .map(|at| at.with_timezone(&Utc));
        if !crate::memory_consolidation::automatic_dream_due(now, settings.dream_hour, last) {
            continue;
        }
        // Only a repository with a group worth a turn is due.
        if !grown(groups(db, &scope)?, last_run(db, &scope)?.as_deref()).is_empty() {
            return Ok(Some(scope));
        }
    }
    Ok(None)
}

/// When this repository was last looked at, a skipped look included.
fn last_any_run(db: &Connection, scope: &str) -> Result<Option<String>, BridgeError> {
    Ok(db
        .query_row("SELECT at FROM taste_learn_runs WHERE scope_key=?1 ORDER BY at DESC LIMIT 1", params![scope], |row| row.get(0))
        .optional()?)
}

/// Repositories with review signals no rule has claimed, which a nightly run
/// should look at.
pub fn scopes_with_open_signals(db: &Connection) -> Result<Vec<String>, BridgeError> {
    let mut statement = db.prepare(
        "SELECT DISTINCT scope_key FROM taste_signals
         WHERE superseded_by IS NULL AND source<>'permission'
           AND id NOT IN (SELECT signal_id FROM memory_evidence)
         ORDER BY scope_key",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
#[path = "learn_tests.rs"]
mod tests;
