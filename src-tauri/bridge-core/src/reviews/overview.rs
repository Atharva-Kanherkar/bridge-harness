//! The Reviews dashboard's data: every review job of a period, across
//! repositories, with what needs a person, numbers, one row per reviewer and
//! recent findings. Read-only, from the review store.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use bridge_protocol::messages::{self as wire, ReviewFindingDecision as Decision, ReviewJobStatus as Status, ReviewPassStatus};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};

use super::store::{self, JobRow};
use crate::runtime::BridgeCore;
use crate::BridgeError;

/// The most jobs listed (live ones always are, on top).
pub const MAX_JOBS: usize = 200;
/// The most findings listed.
pub const MAX_FINDINGS: usize = 300;

pub fn overview(core: &Arc<BridgeCore>, params: &wire::ReviewOverviewParams) -> Result<wire::ReviewOverview, BridgeError> {
    let since = match params.days {
        None => None,
        Some(days @ 1..=365) => Some(store::stamp(Utc::now() - Duration::days(i64::from(days)))),
        Some(days) => return Err(BridgeError::Invalid(format!("days must be 1 to 365, not {days}"))),
    };
    let db = super::lock_db(core);
    overview_at(&db, since.as_deref(), params.repository.as_deref())
}

/// One job with what it ran and found.
pub struct Loaded {
    pub job: JobRow,
    pub passes: Vec<wire::ReviewPass>,
    pub findings: Vec<wire::ReviewFinding>,
}

/// The overview of jobs created at or after `since` (all when absent), of
/// `repository` (`owner/name`) when given. Live jobs are listed whatever
/// their age.
pub fn overview_at(db: &Connection, since: Option<&str>, repository: Option<&str>) -> Result<wire::ReviewOverview, BridgeError> {
    let repository = match repository {
        None => None,
        Some(text) => match text.split_once('/') {
            Some((owner, name)) if !owner.is_empty() && !name.is_empty() => Some((owner, name)),
            _ => return Err(BridgeError::Invalid(format!("a repository is owner/name, not `{text}`"))),
        },
    };
    let mut period = load(db, since, repository, false)?;
    let mut live = load(db, None, repository, true)?;
    // A job that prepared before titles were stored borrows its PR's title
    // from another review of the same PR.
    let titles: HashMap<(String, String, u64), (String, Option<String>)> = db
        .prepare(
            "SELECT lower(repo_owner), lower(repo_name), pr_number, pr_title, pr_author FROM review_jobs
             WHERE pr_title IS NOT NULL ORDER BY created_at",
        )?
        .query_map([], |row| Ok(((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)?.max(0) as u64), (row.get(3)?, row.get(4)?))))?
        .collect::<Result<_, _>>()?;
    for loaded in period.iter_mut().chain(live.iter_mut()) {
        let job = &mut loaded.job;
        if job.pr_title.is_none() {
            if let Some((title, author)) = titles.get(&(job.repo_owner.to_lowercase(), job.repo_name.to_lowercase(), job.number)) {
                job.pr_title = Some(title.clone());
                job.pr_author = job.pr_author.clone().or_else(|| author.clone());
            }
        }
    }
    let mut jobs = Vec::new();
    let mut listed = std::collections::HashSet::new();
    // Live first (they need attention), then the period's newest.
    for loaded in live.iter().chain(period.iter().take(MAX_JOBS)) {
        if listed.insert(loaded.job.id.clone()) {
            jobs.push(store::to_wire(db, &loaded.job)?);
        }
    }
    let (stats, harnesses) = summarize(&period);
    let mut statement = db.prepare(
        "SELECT repo_owner || '/' || repo_name FROM review_jobs GROUP BY lower(repo_owner), lower(repo_name) ORDER BY lower(repo_owner), lower(repo_name)",
    )?;
    let repositories = statement.query_map([], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(wire::ReviewOverview { jobs, stats, harnesses, findings: recent_findings(&period), repositories })
}

const LIVE: &str = "('queued','preparing','reviewing','drafted','posting')";

fn load(db: &Connection, since: Option<&str>, repository: Option<(&str, &str)>, live_only: bool) -> Result<Vec<Loaded>, BridgeError> {
    let live = if live_only { format!(" AND status IN {LIVE}") } else { String::new() };
    let mut statement = db.prepare(&format!(
        "{} WHERE (?1 IS NULL OR created_at>=?1)
            AND (?2 IS NULL OR (repo_owner=?2 COLLATE NOCASE AND repo_name=?3 COLLATE NOCASE)){live}
         ORDER BY created_at DESC, rowid DESC",
        store::SELECT_JOB
    ))?;
    let (owner, name) = repository.unzip();
    let rows = statement.query_map(params![since, owner, name], store::read_job)?.collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|job| {
            Ok(Loaded { passes: store::passes(db, &job.id)?, findings: store::findings(db, &job.id)?, job })
        })
        .collect()
}

fn seconds_between(start: Option<&str>, end: Option<&str>) -> Option<u32> {
    let start = DateTime::parse_from_rfc3339(start?).ok()?;
    let end = DateTime::parse_from_rfc3339(end?).ok()?;
    u32::try_from((end - start).num_seconds()).ok()
}

/// The middle value; the lower of the two middles for an even count.
fn median(mut values: Vec<u32>) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[(values.len() - 1) / 2])
}

#[derive(Default)]
struct HarnessTally {
    passes: u32,
    failed: u32,
    findings: u32,
    kept: u32,
    dropped: u32,
    seconds: Vec<u32>,
    spend: u64,
}

/// Stats and per-reviewer rows over `jobs`. Pure, for tests.
pub fn summarize(jobs: &[Loaded]) -> (wire::ReviewOverviewStats, Vec<wire::ReviewHarnessStats>) {
    let mut stats = wire::ReviewOverviewStats::default();
    let mut job_seconds = Vec::new();
    let mut tallies: BTreeMap<String, HarnessTally> = BTreeMap::new();
    for Loaded { job, passes, findings } in jobs {
        stats.reviews += 1;
        match job.status {
            Status::Queued | Status::Preparing | Status::Reviewing | Status::Posting => stats.running += 1,
            Status::Drafted => stats.waiting += 1,
            Status::Posted => stats.posted += 1,
            Status::Failed | Status::Interrupted => stats.failed += 1,
            Status::Cancelled => stats.cancelled += 1,
        }
        if matches!(job.status, Status::Drafted | Status::Posting | Status::Posted) {
            // A draft has no finish time of its own (it waits for a person):
            // the review took until its last pass finished.
            let reviewed = passes
                .iter()
                .filter(|pass| pass.status == ReviewPassStatus::Done)
                .filter_map(|pass| pass.finished_at.as_deref())
                .max();
            job_seconds.extend(seconds_between(job.started_at.as_deref(), reviewed));
        }
        let harness_of: HashMap<&str, &str> = passes.iter().map(|pass| (pass.id.as_str(), pass.harness.as_str())).collect();
        for pass in passes {
            stats.spend_microusd += pass.cost_microusd;
            stats.tokens_in += pass.tokens_in;
            stats.tokens_out += pass.tokens_out;
            let tally = tallies.entry(pass.harness.clone()).or_default();
            tally.passes += 1;
            tally.spend += pass.cost_microusd;
            if pass.status == ReviewPassStatus::Failed {
                tally.failed += 1;
            }
            if pass.status == ReviewPassStatus::Done {
                tally.seconds.extend(seconds_between(pass.started_at.as_deref(), pass.finished_at.as_deref()));
            }
        }
        for finding in findings {
            stats.findings.total += 1;
            match finding.severity {
                wire::ReviewSeverity::Critical => stats.findings.critical += 1,
                wire::ReviewSeverity::Major => stats.findings.major += 1,
                wire::ReviewSeverity::Minor => stats.findings.minor += 1,
                wire::ReviewSeverity::Nit => stats.findings.nit += 1,
            }
            match finding.decision {
                Decision::Kept => stats.kept += 1,
                Decision::Edited => stats.edited += 1,
                Decision::Dropped => stats.dropped += 1,
                Decision::Pending => stats.pending += 1,
            }
            if finding.fixed_in_sha.is_some() {
                stats.fixed_later += 1;
            }
            if finding.agreed_by > 1 {
                stats.agreed += 1;
            }
            for harness in finding_harnesses(finding, &harness_of) {
                let tally = tallies.entry(harness.to_owned()).or_default();
                tally.findings += 1;
                match finding.decision {
                    Decision::Kept | Decision::Edited => tally.kept += 1,
                    Decision::Dropped => tally.dropped += 1,
                    Decision::Pending => {}
                }
            }
        }
    }
    stats.median_seconds = median(job_seconds);
    let harnesses = tallies
        .into_iter()
        .map(|(harness, tally)| wire::ReviewHarnessStats {
            harness,
            passes: tally.passes,
            failed: tally.failed,
            findings: tally.findings,
            kept: tally.kept,
            dropped: tally.dropped,
            median_seconds: median(tally.seconds),
            spend_microusd: tally.spend,
        })
        .collect();
    (stats, harnesses)
}

/// The harnesses that found `finding`: each pass of a merged one, each once.
fn finding_harnesses<'a>(finding: &wire::ReviewFinding, harness_of: &HashMap<&str, &'a str>) -> Vec<&'a str> {
    let passes: Vec<&str> =
        if finding.pass_ids.is_empty() { vec![finding.pass_id.as_str()] } else { finding.pass_ids.iter().map(String::as_str).collect() };
    let mut found = Vec::new();
    for pass in passes {
        if let Some(harness) = harness_of.get(pass) {
            if !found.contains(harness) {
                found.push(*harness);
            }
        }
    }
    found
}

fn recent_findings(jobs: &[Loaded]) -> Vec<wire::ReviewOverviewFinding> {
    let mut out = Vec::new();
    // `jobs` is newest first; within a job, the store's ranking.
    for Loaded { job, passes, findings } in jobs {
        let harness_of: HashMap<&str, &str> = passes.iter().map(|pass| (pass.id.as_str(), pass.harness.as_str())).collect();
        for finding in findings {
            if out.len() == MAX_FINDINGS {
                return out;
            }
            out.push(wire::ReviewOverviewFinding {
                harnesses: finding_harnesses(finding, &harness_of).into_iter().map(str::to_owned).collect(),
                finding: finding.clone(),
                workspace_id: job.workspace_id.clone(),
                repo_owner: job.repo_owner.clone(),
                repo_name: job.repo_name.clone(),
                number: job.number,
                pr_title: job.pr_title.clone(),
                created_at: job.created_at.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "overview_tests.rs"]
mod tests;
