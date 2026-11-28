//! A synthetic review history for the taste replay and the sub-issues that
//! tune against it (#1200, #1201). Built through the review store itself, at
//! fixed instants, so every signal is the one the real producers write.
//!
//! `octo/repo`, one PR a day:
//!
//! | PR  | the nit (`helper` docs) | SQL injection (critical) | others |
//! |-----|-------------------------|--------------------------|--------|
//! | 101 | dropped                 | dropped                  | unwrap → edited |
//! | 102 | dropped                 | dropped                  | unwrap → edited |
//! | 103 | dropped                 | dropped                  | off-by-one kept, posted, fixed by its next commit |
//! | 104 | dropped                 | kept                     | `Api` docs (near the nit) kept |
//! | 105 | dropped                 |                          | |
//!
//! `octo/other`, PR 7 between 104 and 105: the same nit, kept.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};

use crate::reviews::steps::{DraftFinding, PassOutcome};
use crate::reviews::store::{self, NewJob};
use bridge_protocol::messages::{
    self as wire, ReviewFindingDecision as Decision, ReviewJobStatus as Status, ReviewPostMode, ReviewSeverity, ReviewSide,
    ReviewTrigger,
};

pub const NIT: &str = "Add a doc comment to `helper`";
pub const CRITICAL: &str = "SQL injection in `search`";
pub const NEAR_NIT: &str = "Add a doc comment to the public `Api` type";
pub const UNWRAP: &str = "Use `?` instead of `unwrap()`";
pub const OFF_BY_ONE: &str = "Off-by-one in `get`";

/// The finding ids, by what they are.
#[derive(Debug, Default)]
pub struct Fixture {
    /// The nit in PRs 101 to 105, in order.
    pub nit: Vec<String>,
    /// The critical finding in PRs 101 to 104, in order.
    pub critical: Vec<String>,
    pub near_nit: String,
    pub other_repo_nit: String,
    pub unwrap: Vec<String>,
    pub off_by_one: String,
    /// Review jobs in the order they started.
    pub jobs: Vec<String>,
}

pub fn day(days: f64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-01T09:00:00Z").unwrap().with_timezone(&Utc)
        + Duration::minutes((days * 24.0 * 60.0) as i64)
}

fn finding(path: &str, severity: ReviewSeverity, title: &str) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line: 10,
        start_line: None,
        side: ReviewSide::Right,
        severity,
        title: title.into(),
        body: format!("Body of: {title}"),
        confidence: 0.7,
        fingerprint: format!("fp-{path}-{title}"),
        outside_diff: false,
    }
}

/// A drafted review of `octo/<repo>#number` at `sha`, started at `at`.
/// Returns the job and its finding ids in the order given.
fn review(
    db: &Connection,
    repo: &str,
    number: u64,
    sha: &str,
    at: DateTime<Utc>,
    findings: Vec<DraftFinding>,
    fixed: Vec<String>,
) -> (String, Vec<String>) {
    let new = NewJob {
        workspace_id: format!("w-{repo}"),
        repo_host: Some("github.com".into()),
        repo_owner: "octo".into(),
        repo_name: repo.into(),
        number,
        head_sha: sha.into(),
        base_sha: Some("base".into()),
        incremental_from_sha: None,
        trigger: ReviewTrigger::Manual,
        post_mode: ReviewPostMode::Draft,
        harnesses: vec!["claude".into()],
        retry_of: None,
        publication_key: None,
    };
    let (job, _) = store::insert(db, &new, at).unwrap();
    let claimed = store::claim_next(db, "fixture", at).unwrap().expect("the fixture's job is next");
    assert_eq!(claimed.id, job.id);
    let token = claimed.lease_owner.unwrap();
    assert!(store::advance(db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = store::begin_pass(db, &job.id, &token, 0, "claude", at + Duration::minutes(1)).unwrap().unwrap();
    let titles: Vec<String> = findings.iter().map(|finding| finding.title.clone()).collect();
    let outcome = PassOutcome { findings, fixed, ..Default::default() };
    assert!(store::complete_pass(db, &job.id, &token, &pass, &outcome, at + Duration::minutes(5)).unwrap());
    assert!(store::finish_review(db, &job.id, &token, "summary", None, true).unwrap());
    let stored = store::findings(db, &job.id).unwrap();
    let ids = titles
        .iter()
        .map(|title| stored.iter().find(|finding| &finding.title == title).expect("stored").id.clone())
        .collect();
    (job.id, ids)
}

fn decide(db: &Connection, job: &str, at: DateTime<Utc>, choices: &[(&str, Decision, Option<&str>)]) {
    let decisions: Vec<wire::ReviewFindingDecisionInput> = choices
        .iter()
        .map(|(id, decision, body)| wire::ReviewFindingDecisionInput {
            finding_id: id.to_string(),
            decision: *decision,
            edited_body: body.map(str::to_owned),
        })
        .collect();
    store::decide(db, job, &decisions, at).unwrap();
}

/// Write the history into `db`, a freshly opened Bridge database.
pub fn build(db: &Connection) -> Fixture {
    let mut fixture = Fixture::default();
    for (index, number) in (101..=105).enumerate() {
        let at = day(index as f64 + 1.0);
        let mut findings = vec![finding("src/helper.rs", ReviewSeverity::Nit, NIT)];
        if number <= 104 {
            findings.push(finding("src/search.rs", ReviewSeverity::Critical, CRITICAL));
        }
        if number <= 102 {
            findings.push(finding("src/parse.rs", ReviewSeverity::Major, UNWRAP));
        }
        if number == 103 {
            findings.push(finding("src/get.rs", ReviewSeverity::Major, OFF_BY_ONE));
        }
        if number == 104 {
            findings.push(finding("src/api.rs", ReviewSeverity::Minor, NEAR_NIT));
        }
        let (job, ids) = review(db, "repo", number, &format!("sha-{number}"), at, findings, vec![]);
        fixture.jobs.push(job.clone());
        fixture.nit.push(ids[0].clone());
        let mut choices = vec![(ids[0].as_str(), Decision::Dropped, None)];
        if number <= 104 {
            fixture.critical.push(ids[1].clone());
            let verdict = if number == 104 { Decision::Kept } else { Decision::Dropped };
            choices.push((ids[1].as_str(), verdict, None));
        }
        if number <= 102 {
            fixture.unwrap.push(ids[2].clone());
            choices.push((ids[2].as_str(), Decision::Edited, Some("Return the error with `?`; the caller reports it.")));
        }
        if number == 103 {
            fixture.off_by_one = ids[2].clone();
            choices.push((ids[2].as_str(), Decision::Kept, None));
        }
        if number == 104 {
            fixture.near_nit = ids[2].clone();
            choices.push((ids[2].as_str(), Decision::Kept, None));
        }
        decide(db, &job, at + Duration::hours(1), &choices);
        if number == 103 {
            db.execute(
                "UPDATE review_findings SET posted_at=?2 WHERE id=?1",
                params![fixture.off_by_one, store::stamp(at + Duration::hours(2))],
            )
            .unwrap();
            // The author's next commit fixes it; the re-review says so.
            let (fix_job, _) =
                review(db, "repo", 103, "sha-103-fix", at + Duration::hours(6), vec![], vec![fixture.off_by_one.clone()]);
            fixture.jobs.push(fix_job);
        }
        if number == 104 {
            let other_at = day(4.5);
            let (other, ids) = review(db, "other", 7, "sha-7", other_at, vec![finding("src/helper.rs", ReviewSeverity::Nit, NIT)], vec![]);
            fixture.jobs.push(other.clone());
            fixture.other_repo_nit = ids[0].clone();
            decide(db, &other, other_at + Duration::hours(1), &[(ids[0].as_str(), Decision::Kept, None)]);
        }
    }
    fixture
}
