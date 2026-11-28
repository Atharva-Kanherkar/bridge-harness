//! How well each taste rule holds up (#1203).
//!
//! Scores are recomputed from what happened, never nudged in place, so they
//! are reproducible and decay falls out of the sum: each event counts
//! `0.5^(age / HALF_LIFE_DAYS)` of its weight.
//!
//! - A finding a rule shaped (it cites the rule): kept or edited `+1`, fixed
//!   `+1`, dropped `-1`.
//! - A posted review that was given a `review_avoid` rule: `+0.5`, or `-1`
//!   when the person kept a finding like the rule's evidence anyway (they
//!   raised what it avoids).
//! - A finding the taste layer hid that the person brought back and kept:
//!   `-2` for each rule whose evidence hid it.
//!
//! A learned rule pauses itself when its score reaches [`PAUSE_SCORE`] or
//! when at least [`NOISY_MIN_EVENTS`] events in [`NOISY_WINDOW_DAYS`] are
//! mostly against it, and expires after [`EXPIRE_DAYS`] without anything for
//! it. Rules the person wrote, and rules they pinned, do neither.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, Transaction};

use crate::BridgeError;

pub const HALF_LIFE_DAYS: f64 = 60.0;
pub const PAUSE_SCORE: f64 = -2.0;
pub const NOISY_WINDOW_DAYS: i64 = 30;
pub const NOISY_MIN_EVENTS: usize = 3;
pub const NOISY_SHARE: f64 = 0.6;
pub const EXPIRE_DAYS: i64 = 90;
/// Passes a rule must be given before "never used" means anything.
pub const NEVER_USED_PASSES: i64 = 10;
pub const STATUS_PAUSED: &str = "paused";

/// Schema 111: a pin that keeps a learned rule from pausing or expiring, and
/// why a rule paused.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    use crate::store::add_column_if_missing;
    add_column_if_missing(transaction, "memory_records", "pinned", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(transaction, "memory_records", "paused_reason", "TEXT")?;
    Ok(())
}

/// One thing that happened to a rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub rule_id: String,
    pub at: DateTime<Utc>,
    pub delta: f64,
}

fn instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw).ok().map(|at| at.with_timezone(&Utc))
}

fn ids(raw: Option<String>) -> Vec<String> {
    raw.and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok()).unwrap_or_default()
}

/// A rule as scoring needs it.
struct Rule {
    id: String,
    kind: String,
    evidence: HashSet<String>,
}

/// Every event of every taste rule, from the review tables and the log.
pub fn events(db: &Connection) -> Result<Vec<Event>, BridgeError> {
    let rules: Vec<Rule> = {
        let mut statement = db.prepare("SELECT id, kind FROM memory_records WHERE layer='taste' AND status IN ('active','paused')")?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        let mut rules = Vec::new();
        for row in rows {
            let (id, kind) = row?;
            let mut evidence = db.prepare("SELECT signal_id FROM memory_evidence WHERE record_id=?1")?;
            let linked: HashSet<String> = evidence.query_map(params![id], |row| row.get(0))?.collect::<Result<_, _>>()?;
            rules.push(Rule { id, kind, evidence: linked });
        }
        rules
    };
    let known: HashSet<&str> = rules.iter().map(|rule| rule.id.as_str()).collect();
    let mut events = Vec::new();

    // Findings: who cited which rule, what became of them, what hid them.
    struct Finding {
        id: String,
        job_id: String,
        decision: String,
        decided_at: Option<String>,
        created_at: String,
        rule_ids: Vec<String>,
        hidden_by: Vec<String>,
        fixed: bool,
    }
    let findings: Vec<Finding> = {
        let mut statement = db.prepare(
            "SELECT id, job_id, decision, decided_at, created_at, rule_ids, hidden_by_signals, fixed_in_sha IS NOT NULL
             FROM review_findings WHERE rule_ids IS NOT NULL OR hidden_by_signals IS NOT NULL OR decision IN ('kept','edited')",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Finding {
                id: row.get(0)?,
                job_id: row.get(1)?,
                decision: row.get(2)?,
                decided_at: row.get(3)?,
                created_at: row.get(4)?,
                rule_ids: ids(row.get(5)?),
                hidden_by: ids(row.get(6)?),
                fixed: row.get(7)?,
            })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let fixed_at: HashMap<String, String> = {
        let mut statement = db.prepare(
            "SELECT subject_id, at FROM taste_signals WHERE source='review_fixed' AND superseded_by IS NULL",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for finding in &findings {
        let decided = finding.decided_at.as_deref().unwrap_or(&finding.created_at);
        let Some(at) = instant(decided) else { continue };
        let kept = matches!(finding.decision.as_str(), "kept" | "edited");
        for rule in finding.rule_ids.iter().filter(|rule| known.contains(rule.as_str())) {
            match finding.decision.as_str() {
                "kept" | "edited" => events.push(Event { rule_id: rule.clone(), at, delta: 1.0 }),
                "dropped" => events.push(Event { rule_id: rule.clone(), at, delta: -1.0 }),
                _ => {}
            }
            if finding.fixed {
                let when = fixed_at.get(&finding.id).and_then(|raw| instant(raw)).unwrap_or(at);
                events.push(Event { rule_id: rule.clone(), at: when, delta: 1.0 });
            }
        }
        if kept && !finding.hidden_by.is_empty() {
            for rule in rules.iter().filter(|rule| finding.hidden_by.iter().any(|signal| rule.evidence.contains(signal))) {
                events.push(Event { rule_id: rule.id.clone(), at, delta: -2.0 });
            }
        }
    }

    // Posted reviews that were given an avoid rule.
    let posted: Vec<(String, String, Vec<String>)> = {
        let mut statement = db.prepare(
            "SELECT j.id, COALESCE(j.finished_at, j.created_at), p.taste_rule_ids
             FROM review_jobs j JOIN review_passes p ON p.job_id = j.id
             WHERE j.status='posted' AND p.taste_rule_ids IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, ids(row.get(2)?))))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut credited: HashSet<(String, String)> = HashSet::new();
    for (job, finished, given) in posted {
        let Some(at) = instant(&finished) else { continue };
        for rule in rules.iter().filter(|rule| rule.kind == "review_avoid" && given.contains(&rule.id)) {
            if !credited.insert((job.clone(), rule.id.clone())) {
                continue;
            }
            let raised_anyway = findings
                .iter()
                .filter(|finding| finding.job_id == job && matches!(finding.decision.as_str(), "kept" | "edited"))
                .any(|finding| raised_what_it_avoids(db, &finding.id, &rule.evidence));
            events.push(Event { rule_id: rule.id.clone(), at, delta: if raised_anyway { -1.0 } else { 0.5 } });
        }
    }
    Ok(events)
}

/// Whether a kept finding is like the decisions an avoid rule learned from.
fn raised_what_it_avoids(db: &Connection, finding_id: &str, evidence: &HashSet<String>) -> bool {
    let Ok(Some(candidate)) = super::replay::candidate(db, finding_id) else { return false };
    evidence.iter().any(|signal| {
        super::signals::by_id(db, signal).ok().flatten().is_some_and(|signal| super::suppress::alike(&candidate, &signal))
    })
}

/// A rule's score at `now`: each event's weight, halved every
/// [`HALF_LIFE_DAYS`].
pub fn score(events: &[&Event], now: DateTime<Utc>) -> f64 {
    events
        .iter()
        .map(|event| {
            let age = (now - event.at).num_seconds().max(0) as f64 / 86_400.0;
            event.delta * 0.5f64.powf(age / HALF_LIFE_DAYS)
        })
        .sum()
}

/// What a recompute changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Recomputed {
    pub scored: usize,
    pub paused: Vec<String>,
    pub expired: Vec<String>,
}

/// Recompute every rule's score and confirmation, then pause and expire the
/// learned, unpinned rules that call for it.
pub fn recompute(db: &Connection, now: DateTime<Utc>) -> Result<Recomputed, BridgeError> {
    let all = events(db)?;
    let mut by_rule: HashMap<&str, Vec<&Event>> = HashMap::new();
    for event in &all {
        by_rule.entry(event.rule_id.as_str()).or_default().push(event);
    }
    struct Row {
        id: String,
        status: String,
        provenance: String,
        pinned: bool,
        created_at: String,
    }
    let rows: Vec<Row> = {
        let mut statement = db.prepare(
            "SELECT id, status, provenance, pinned, created_at FROM memory_records
             WHERE layer='taste' AND status IN ('active','paused')",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Row { id: row.get(0)?, status: row.get(1)?, provenance: row.get(2)?, pinned: row.get(3)?, created_at: row.get(4)? })
        })?;
        rows.collect::<Result<_, _>>()?
    };
    let mut done = Recomputed::default();
    let stamp = super::signals::instant(&now.to_rfc3339());
    for row in rows {
        let events = by_rule.get(row.id.as_str()).cloned().unwrap_or_default();
        let value = score(&events, now);
        let confirmed = events.iter().filter(|event| event.delta > 0.0).map(|event| event.at).max();
        let confirmed_text = confirmed.map(|at| super::signals::instant(&at.to_rfc3339()));
        db.execute(
            "UPDATE memory_records SET outcome_score=?2, last_confirmed_at=COALESCE(?3, last_confirmed_at) WHERE id=?1",
            params![row.id, (value * 100.0).round() / 100.0, confirmed_text],
        )?;
        done.scored += 1;
        let protected = row.pinned || row.provenance == "user_explicit";
        if protected || row.status != "active" {
            continue;
        }
        let recent: Vec<&&Event> = events.iter().filter(|event| now - event.at <= Duration::days(NOISY_WINDOW_DAYS)).collect();
        let against = recent.iter().filter(|event| event.delta < 0.0).count();
        let noisy = recent.len() >= NOISY_MIN_EVENTS && against as f64 >= NOISY_SHARE * recent.len() as f64;
        if value <= PAUSE_SCORE || noisy {
            let reason = if value <= PAUSE_SCORE {
                format!("Paused: its trust fell to {value:.1}.")
            } else {
                format!("Paused: {against} of its last {} uses went against it.", recent.len())
            };
            db.execute(
                "UPDATE memory_records SET status=?2, paused_reason=?3, updated_at=?4 WHERE id=?1 AND status='active'",
                params![row.id, STATUS_PAUSED, reason, stamp],
            )?;
            done.paused.push(row.id);
            continue;
        }
        let since = confirmed.or_else(|| instant(&row.created_at));
        if since.is_some_and(|since| now - since > Duration::days(EXPIRE_DAYS)) {
            if crate::memory_ledger::retire(db, &row.id, &stamp)? {
                done.expired.push(row.id);
            }
        }
    }
    Ok(done)
}

/// Rules given to at least [`NEVER_USED_PASSES`] passes and cited by none.
/// Avoid rules shape a review by leaving things out, so they are never
/// "unused" this way.
pub fn never_used(db: &Connection) -> Result<HashSet<String>, BridgeError> {
    let mut given: HashMap<String, i64> = HashMap::new();
    let mut statement = db.prepare("SELECT taste_rule_ids FROM review_passes WHERE taste_rule_ids IS NOT NULL")?;
    for row in statement.query_map([], |row| row.get::<_, Option<String>>(0))? {
        for id in ids(row?) {
            *given.entry(id).or_default() += 1;
        }
    }
    let mut cited: HashSet<String> = HashSet::new();
    let mut statement = db.prepare("SELECT rule_ids FROM review_findings WHERE rule_ids IS NOT NULL")?;
    for row in statement.query_map([], |row| row.get::<_, Option<String>>(0))? {
        cited.extend(ids(row?));
    }
    let avoid: HashSet<String> = {
        let mut statement = db.prepare("SELECT id FROM memory_records WHERE layer='taste' AND kind='review_avoid'")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    Ok(given
        .into_iter()
        .filter(|(id, count)| *count >= NEVER_USED_PASSES && !cited.contains(id) && !avoid.contains(id))
        .map(|(id, _)| id)
        .collect())
}

/// Pause, resume or pin a rule by hand. Resuming clears the reason; pinning
/// keeps a learned rule from pausing or expiring on its own.
pub fn set_state(db: &Connection, record_id: &str, state: Option<&str>, pinned: Option<bool>) -> Result<(), BridgeError> {
    let status: Option<String> = db
        .query_row("SELECT status FROM memory_records WHERE id=?1 AND layer='taste'", params![record_id], |row| row.get(0))
        .ok();
    let Some(status) = status else {
        return Err(BridgeError::Invalid(format!("There is no taste rule '{record_id}'.")));
    };
    let now = super::signals::instant(&Utc::now().to_rfc3339());
    match state {
        None => {}
        Some("paused") if status == "active" => {
            db.execute(
                "UPDATE memory_records SET status='paused', paused_reason='Paused by you.', updated_at=?2 WHERE id=?1",
                params![record_id, now],
            )?;
        }
        Some("active") if status == "paused" => {
            db.execute(
                "UPDATE memory_records SET status='active', paused_reason=NULL, updated_at=?2, last_confirmed_at=?2 WHERE id=?1",
                params![record_id, now],
            )?;
        }
        Some(wanted) if wanted == status => {}
        Some(wanted) => {
            return Err(BridgeError::Invalid(format!("A {status} taste rule cannot be made {wanted}.")));
        }
    }
    if let Some(pinned) = pinned {
        db.execute("UPDATE memory_records SET pinned=?2, updated_at=?3 WHERE id=?1", params![record_id, pinned, now])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "score_tests.rs"]
mod tests;
