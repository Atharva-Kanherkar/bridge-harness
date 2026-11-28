//! The taste replay: what would the taste layer have done with the reviews a
//! person already triaged, and would they have been better off?
//!
//! Reviews are walked in the order they were started. Before each one, the
//! judge sees only the signals written strictly before it, as they stood then
//! (a decision later changed counts as it was), and only those of the same
//! repository. It says, for each finding of that review, show or hide. The
//! person's real outcome on the finding then scores the call: hiding what
//! they kept, edited or fixed is a mistake, hiding a critical finding is never
//! allowed, and the keep rate of what stays visible should rise.
//!
//! Nothing is written. Thresholds are the judge's parameters, so suppression
//! (#1200) and the learner (#1201) can sweep them against the same history.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

use super::signals::{self, Signal};
use crate::BridgeError;

/// A finding as it appeared in its review, with what the person did with it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub finding_id: String,
    pub job_id: String,
    pub scope_key: String,
    pub pr_number: u64,
    /// When its review started: the judge sees signals strictly before this.
    pub reviewed_at: String,
    pub path: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    #[serde(skip)]
    pub body: String,
    pub fingerprint: String,
    pub outcome: Outcome,
}

/// What the person did with a finding, as it stands now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// `pending`, `kept`, `edited` or `dropped`.
    pub decision: &'static str,
    pub posted: bool,
    pub fixed: bool,
}

impl Outcome {
    /// The finding was worth raising: kept, edited, or fixed by the author.
    pub fn wanted(&self) -> bool {
        matches!(self.decision, "kept" | "edited") || self.fixed
    }

    pub fn decided(&self) -> bool {
        self.decision != "pending"
    }
}

/// A judge's call on one finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Judgment {
    Show,
    /// `evidence` names the signals that made the call.
    Hide { reason: String, evidence: Vec<String> },
}

/// What the taste layer would do with a finding, given only the past.
pub trait Judge {
    /// `past` holds the candidate's repository's signals from before its
    /// review, one per subject and source, as each stood then, oldest first.
    fn judge(&self, past: &[Signal], candidate: &Candidate) -> Judgment;
}

/// The taste layer switched off: the baseline every judge is compared with.
pub struct NeverHide;

impl Judge for NeverHide {
    fn judge(&self, _past: &[Signal], _candidate: &Candidate) -> Judgment {
        Judgment::Show
    }
}

/// A reference judge that hides a finding whose normalized title the person
/// dropped at least `min_drops` times across at least `min_prs` pull
/// requests, with nothing like it kept, edited or fixed since the last drop.
/// Never hides a critical or security finding. Suppression (#1200) replaces
/// the exact-title match with the reviewer's similarity measure.
pub struct RepeatedDrops {
    pub min_drops: usize,
    pub min_prs: usize,
}

impl Default for RepeatedDrops {
    fn default() -> Self {
        Self { min_drops: 3, min_prs: 2 }
    }
}

impl Judge for RepeatedDrops {
    fn judge(&self, past: &[Signal], candidate: &Candidate) -> Judgment {
        if candidate.severity == "critical" || candidate.category == "security" {
            return Judgment::Show;
        }
        let title = crate::reviews::contract::normalized(&candidate.title);
        let alike: Vec<&Signal> = past
            .iter()
            .filter(|signal| signal.source != signals::SOURCE_PERMISSION)
            .filter(|signal| signal.title.as_deref().map(crate::reviews::contract::normalized).as_deref() == Some(title.as_str()))
            .collect();
        let drops: Vec<&&Signal> = alike.iter().filter(|signal| signal.verdict == "dropped").collect();
        let last_drop = drops.iter().map(|signal| signal.at.as_str()).max();
        let wanted_since = alike
            .iter()
            .filter(|signal| matches!(signal.verdict.as_str(), "kept" | "edited" | "fixed" | "resolved"))
            .any(|signal| Some(signal.at.as_str()) >= last_drop);
        let prs: std::collections::HashSet<Option<u64>> = drops.iter().map(|signal| signal.pr_number).collect();
        if drops.len() >= self.min_drops && prs.len() >= self.min_prs && !wanted_since {
            let last_pr = drops.iter().max_by_key(|signal| signal.at.as_str()).and_then(|signal| signal.pr_number);
            Judgment::Hide {
                reason: format!(
                    "You dropped {} like this{}.",
                    drops.len(),
                    last_pr.map(|number| format!(", last on #{number}")).unwrap_or_default()
                ),
                evidence: drops.iter().map(|signal| signal.id.clone()).collect(),
            }
        } else {
            Judgment::Show
        }
    }
}

/// A finding the judge hid that the person in fact wanted.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Miss {
    pub finding_id: String,
    pub pr_number: u64,
    pub scope_key: String,
    pub severity: String,
    pub title: String,
    pub reason: String,
    pub outcome: Outcome,
}

/// What a replay found.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub reviews: usize,
    pub findings: usize,
    pub decided: usize,
    pub hidden: usize,
    /// Kept plus edited over decided, for every finding: the baseline.
    pub keep_rate_all: Option<f64>,
    /// The same for the findings left visible.
    pub keep_rate_visible: Option<f64>,
    /// Hidden findings the person kept, edited or fixed, each in full.
    pub wrongly_hidden: Vec<Miss>,
    /// Must be 0.
    pub critical_hidden: usize,
    pub posted: usize,
    /// Posted findings later fixed, over posted.
    pub address_rate: Option<f64>,
    /// Reviews where the judge hid at least one finding.
    pub reviews_affected: usize,
    /// Every call, in review order: (finding id, hidden).
    #[serde(skip)]
    pub calls: Vec<(String, bool)>,
}

fn ratio(numerator: usize, denominator: usize) -> Option<f64> {
    (denominator > 0).then(|| numerator as f64 / denominator as f64)
}

/// Every reviewed finding, in the order its review started.
pub fn candidates(db: &Connection) -> Result<Vec<Candidate>, BridgeError> {
    candidates_where(db, "1", params![])
}

/// One review's findings, as [`candidates`] reads them.
pub fn candidates_of_job(db: &Connection, job_id: &str) -> Result<Vec<Candidate>, BridgeError> {
    candidates_where(db, "f.job_id = ?1", params![job_id])
}

fn candidates_where(db: &Connection, filter: &str, values: &[&dyn rusqlite::ToSql]) -> Result<Vec<Candidate>, BridgeError> {
    let mut statement = db.prepare(&format!(
        "SELECT f.id, f.job_id, j.workspace_id, j.repo_owner, j.repo_name, j.pr_number, j.created_at,
                f.path, f.severity, f.title, f.body, f.fingerprint, f.decision, f.posted_at IS NOT NULL,
                f.fixed_in_sha IS NOT NULL, f.category
         FROM review_findings f JOIN review_jobs j ON j.id = f.job_id
         WHERE {filter}
         ORDER BY j.created_at, j.id, f.created_at, f.id"
    ))?;
    let rows = statement.query_map(values, |row| {
        let decision: String = row.get(12)?;
        let title: String = row.get(9)?;
        let body: String = row.get(10)?;
        Ok(Candidate {
            finding_id: row.get(0)?,
            job_id: row.get(1)?,
            scope_key: signals::repository_scope(&row.get::<_, String>(3)?, &row.get::<_, String>(4)?, &row.get::<_, String>(2)?),
            pr_number: row.get::<_, i64>(5)?.max(0) as u64,
            reviewed_at: signals::instant(&row.get::<_, String>(6)?),
            path: row.get(7)?,
            severity: row.get(8)?,
            category: signals::category_or_guess(row.get::<_, Option<String>>(15)?.as_deref(), &title, &body).to_string(),
            title,
            body,
            fingerprint: row.get(11)?,
            outcome: Outcome {
                decision: match decision.as_str() {
                    "kept" => "kept",
                    "edited" => "edited",
                    "dropped" => "dropped",
                    _ => "pending",
                },
                posted: row.get(13)?,
                fixed: row.get(14)?,
            },
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// The signals of `scope` as they stood just before `at`: for each subject
/// and source, the latest one written before it.
pub fn as_of<'a>(every: &'a [Signal], scope: &str, at: &str) -> Vec<&'a Signal> {
    let mut latest: HashMap<(&str, &str), &Signal> = HashMap::new();
    for signal in every.iter().filter(|signal| signal.scope_key == scope && before(&signal.at, at)) {
        latest.insert((signal.source.as_str(), signal.subject_id.as_str()), signal);
    }
    let mut past: Vec<&Signal> = latest.into_values().collect();
    past.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    past
}

/// `a` strictly before `b`, by the instant each names; text that is not an
/// instant compares as text.
fn before(a: &str, b: &str) -> bool {
    match (DateTime::parse_from_rfc3339(a), DateTime::parse_from_rfc3339(b)) {
        (Ok(a), Ok(b)) => a.with_timezone(&Utc) < b.with_timezone(&Utc),
        _ => a < b,
    }
}

/// Replay every triaged review in `db` with `judge`.
pub fn replay(db: &Connection, judge: &dyn Judge) -> Result<Report, BridgeError> {
    let every = signals::all(db)?;
    let candidates = candidates(db)?;
    Ok(score(&every, &candidates, judge))
}

/// Score `judge` on `candidates` against `every` signal. Pure, so the
/// no-lookahead property can be checked by changing the future and replaying.
pub fn score(every: &[Signal], candidates: &[Candidate], judge: &dyn Judge) -> Report {
    let mut report = Report::default();
    let mut reviews = std::collections::BTreeSet::new();
    let mut affected = std::collections::BTreeSet::new();
    let (mut kept_all, mut kept_visible, mut decided_visible, mut fixed_posted) = (0, 0, 0, 0);
    let mut past_by_review: HashMap<&str, Vec<Signal>> = HashMap::new();
    for candidate in candidates {
        reviews.insert(candidate.job_id.as_str());
        let past = past_by_review
            .entry(candidate.job_id.as_str())
            .or_insert_with(|| as_of(every, &candidate.scope_key, &candidate.reviewed_at).into_iter().cloned().collect());
        let judgment = judge.judge(past, candidate);
        let hidden = matches!(judgment, Judgment::Hide { .. });
        report.findings += 1;
        report.calls.push((candidate.finding_id.clone(), hidden));
        let outcome = candidate.outcome;
        if outcome.decided() {
            report.decided += 1;
            kept_all += usize::from(matches!(outcome.decision, "kept" | "edited"));
            if !hidden {
                decided_visible += 1;
                kept_visible += usize::from(matches!(outcome.decision, "kept" | "edited"));
            }
        }
        if outcome.posted {
            report.posted += 1;
            fixed_posted += usize::from(outcome.fixed);
        }
        if let Judgment::Hide { reason, .. } = judgment {
            report.hidden += 1;
            affected.insert(candidate.job_id.as_str());
            report.critical_hidden += usize::from(candidate.severity == "critical");
            if outcome.wanted() {
                report.wrongly_hidden.push(Miss {
                    finding_id: candidate.finding_id.clone(),
                    pr_number: candidate.pr_number,
                    scope_key: candidate.scope_key.clone(),
                    severity: candidate.severity.clone(),
                    title: candidate.title.clone(),
                    reason,
                    outcome,
                });
            }
        }
    }
    report.reviews = reviews.len();
    report.reviews_affected = affected.len();
    report.keep_rate_all = ratio(kept_all, report.decided);
    report.keep_rate_visible = ratio(kept_visible, decided_visible);
    report.address_rate = ratio(fixed_posted, report.posted);
    report
}

/// One row of a threshold sweep.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepRow {
    pub min_drops: usize,
    pub min_prs: usize,
    pub hidden: usize,
    pub wrongly_hidden: usize,
    pub critical_hidden: usize,
    pub keep_rate_visible: Option<f64>,
    pub reviews_affected: usize,
}

/// Replay suppression ([`super::suppress::Suppressor`]) at every pair of
/// thresholds in `grid`.
pub fn sweep(db: &Connection, grid: &[(usize, usize)]) -> Result<Vec<SweepRow>, BridgeError> {
    let every = signals::all(db)?;
    let candidates = candidates(db)?;
    Ok(grid
        .iter()
        .map(|&(min_drops, min_prs)| {
            let judge = super::suppress::Suppressor { min_drops: min_drops as f64, min_prs };
            let report = score(&every, &candidates, &judge);
            SweepRow {
                min_drops,
                min_prs,
                hidden: report.hidden,
                wrongly_hidden: report.wrongly_hidden.len(),
                critical_hidden: report.critical_hidden,
                keep_rate_visible: report.keep_rate_visible,
                reviews_affected: report.reviews_affected,
            }
        })
        .collect())
}

/// A report as a person reads it: counts and titles, never finding bodies.
pub fn render(report: &Report) -> String {
    let percent = |rate: Option<f64>| rate.map(|rate| format!("{:.0}%", rate * 100.0)).unwrap_or_else(|| "n/a".into());
    let mut text = format!(
        "{} reviews, {} findings, {} decided\n\
         hidden {} in {} reviews; critical hidden {}\n\
         keep rate {} overall, {} of what stays visible\n\
         address rate {} ({} posted)\n\
         wrongly hidden {}\n",
        report.reviews,
        report.findings,
        report.decided,
        report.hidden,
        report.reviews_affected,
        report.critical_hidden,
        percent(report.keep_rate_all),
        percent(report.keep_rate_visible),
        percent(report.address_rate),
        report.posted,
        report.wrongly_hidden.len(),
    );
    for miss in &report.wrongly_hidden {
        text.push_str(&format!(
            "  #{} {} [{}] {} ({})\n",
            miss.pr_number, miss.scope_key, miss.severity, miss.title, miss.outcome.decision
        ));
    }
    text
}

#[cfg(test)]
pub(crate) mod fixture;

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
