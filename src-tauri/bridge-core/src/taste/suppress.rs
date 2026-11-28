//! Hide what a person keeps dropping (#1200).
//!
//! A new review finding that closely resembles findings the person dropped at
//! least [`MIN_DROPS`] times across at least [`MIN_PRS`] pull requests of the
//! same repository, with nothing like it kept, edited or fixed since, is
//! hidden in the draft with the reason in their words. Hidden is not deleted:
//! the finding stays pending, one click shows it, and it is never posted
//! without a decision. Critical and security findings are never hidden this
//! way, whatever the person dropped.
//!
//! "Resembles" is stricter than second opinion's match, which pairs two
//! reviewers on the same lines of one PR: across PRs there are no lines to
//! lean on, so it takes the same category and language, and either close
//! titles or the same quoted code with related titles.

use rusqlite::{params, Connection, Transaction};

use super::replay::{Candidate, Judge, Judgment};
use super::signals::{self, Signal};
use crate::reviews::merge::{code_overlap, title_overlap};
use crate::BridgeError;

/// Drops of findings like it before one is hidden. Ignored threads count half.
pub const MIN_DROPS: f64 = 3.0;
/// Distinct pull requests those drops came from.
pub const MIN_PRS: usize = 2;
/// Title-word overlap at which two findings read as the same kind.
pub const MIN_TITLE_OVERLAP: f64 = 0.6;
/// Quoted-code overlap that, with related titles, makes two findings alike.
pub const MIN_CODE_OVERLAP: f64 = 0.5;
/// How related the titles must be for shared code to count.
pub const MIN_TITLE_WITH_CODE: f64 = 0.3;

/// Schema 107: a finding's own category and the taste layer's reason to hide it.
pub(crate) fn install(transaction: &Transaction<'_>) -> Result<(), BridgeError> {
    use crate::store::add_column_if_missing;
    add_column_if_missing(transaction, "review_findings", "category", "TEXT")?;
    add_column_if_missing(transaction, "review_findings", "hidden_reason", "TEXT")?;
    add_column_if_missing(transaction, "review_findings", "hidden_by_signals", "TEXT")?;
    Ok(())
}

/// The suppression judge, with its thresholds as parameters so the replay can
/// sweep them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Suppressor {
    pub min_drops: f64,
    pub min_prs: usize,
}

impl Default for Suppressor {
    fn default() -> Self {
        Self { min_drops: MIN_DROPS, min_prs: MIN_PRS }
    }
}

/// Whether a past signal is about the same kind of finding as `candidate`.
pub fn alike(candidate: &Candidate, signal: &Signal) -> bool {
    let Some(title) = signal.title.as_deref() else { return false };
    if signal.category.as_deref() != Some(candidate.category.as_str()) {
        return false;
    }
    if signal.path.as_deref().and_then(signals::language_of) != signals::language_of(&candidate.path) {
        return false;
    }
    let titles = title_overlap(&candidate.title, title);
    if titles >= MIN_TITLE_OVERLAP {
        return true;
    }
    let theirs = format!("{title}\n{}", signal.body.as_deref().unwrap_or_default());
    let ours = format!("{}\n{}", candidate.title, candidate.body);
    titles >= MIN_TITLE_WITH_CODE && code_overlap(&ours, &theirs) >= MIN_CODE_OVERLAP
}

fn was_hidden(signal: &Signal) -> bool {
    signal.detail.as_ref().and_then(|detail| detail.get("wasHidden")).and_then(serde_json::Value::as_bool) == Some(true)
}

impl Judge for Suppressor {
    fn judge(&self, past: &[Signal], candidate: &Candidate) -> Judgment {
        if candidate.severity == "critical" || candidate.category == "security" {
            return Judgment::Show;
        }
        let alike: Vec<&Signal> = past
            .iter()
            .filter(|signal| signal.source != signals::SOURCE_PERMISSION && alike(candidate, signal))
            .collect();
        let against: Vec<&&Signal> = alike.iter().filter(|signal| matches!(signal.verdict.as_str(), "dropped" | "ignored")).collect();
        let weight_against: f64 = against.iter().map(|signal| if signal.verdict == "ignored" { 0.5 } else { 1.0 }).sum();
        let mut weight_for = 0.0;
        let mut latest_for: Option<&str> = None;
        for signal in alike.iter().filter(|signal| matches!(signal.verdict.as_str(), "kept" | "edited" | "fixed" | "resolved")) {
            weight_for += if was_hidden(signal) { 2.0 } else { 1.0 };
            latest_for = latest_for.max(Some(signal.at.as_str()));
        }
        let latest_against = against.iter().map(|signal| signal.at.as_str()).max();
        let prs: std::collections::HashSet<Option<u64>> = against.iter().map(|signal| signal.pr_number).collect();
        let hide = weight_against >= self.min_drops
            && prs.len() >= self.min_prs
            && weight_against > weight_for
            && latest_for < latest_against;
        if !hide {
            return Judgment::Show;
        }
        let drops = against.iter().filter(|signal| signal.verdict == "dropped").count();
        let last_pr = against.iter().max_by_key(|signal| signal.at.as_str()).and_then(|signal| signal.pr_number);
        Judgment::Hide {
            reason: format!(
                "You dropped {drops} like this{}.",
                last_pr.map(|number| format!(", last on #{number}")).unwrap_or_default()
            ),
            evidence: against.iter().map(|signal| signal.id.clone()).collect(),
        }
    }
}

/// Hide the job's new findings that the person keeps dropping, when the
/// setting is on. Only pending findings not hidden before are judged, against
/// the repository's live signals. Returns how many were hidden.
pub(crate) fn apply(db: &Connection, job_id: &str) -> Result<usize, BridgeError> {
    if !crate::reviews::settings::load(db)?.hide_repeat_drops {
        return Ok(0);
    }
    let candidates = super::replay::candidates_of_job(db, job_id)?;
    let Some(scope) = candidates.first().map(|candidate| candidate.scope_key.clone()) else { return Ok(0) };
    let past = signals::live_in(db, &scope)?;
    let judge = Suppressor::default();
    let mut hidden = 0;
    for candidate in candidates.iter().filter(|candidate| candidate.outcome.decision == "pending") {
        if let Judgment::Hide { reason, evidence } = judge.judge(&past, candidate) {
            hidden += db.execute(
                "UPDATE review_findings SET hidden_reason=?2, hidden_by_signals=?3
                 WHERE id=?1 AND decision='pending' AND hidden_reason IS NULL",
                params![candidate.finding_id, reason, serde_json::to_string(&evidence).unwrap_or_else(|_| "[]".into())],
            )?;
        }
    }
    Ok(hidden)
}

#[cfg(test)]
#[path = "suppress_tests.rs"]
mod tests;
