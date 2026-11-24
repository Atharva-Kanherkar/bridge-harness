use super::*;
use crate::reviews::steps::{DraftFinding, PassOutcome};
use bridge_protocol::messages::{ReviewPostMode, ReviewSeverity, ReviewSide, ReviewTrigger};

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

fn at(day: u32, seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("2026-11-{day:02}T10:00:00Z")).unwrap().with_timezone(&Utc) + Duration::seconds(seconds)
}

fn finding(path: &str, severity: ReviewSeverity) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line: 3,
        start_line: None,
        side: ReviewSide::Right,
        severity,
        title: format!("Problem in {path}"),
        body: "Body.".into(),
        confidence: 0.7,
        fingerprint: format!("fp-{path}"),
        outside_diff: false,
    }
}

/// A job in `workspace` created on `day`, run by `harnesses` for `seconds`,
/// ending in `status`, with `findings` from its first pass. Returns its id
/// and pass ids.
fn job(
    db: &Connection,
    workspace: &str,
    number: u64,
    day: u32,
    harnesses: &[&str],
    seconds: i64,
    status: &str,
    findings: &[DraftFinding],
) -> (String, Vec<String>) {
    let (job, _) = store::insert(
        db,
        &store::NewJob {
            workspace_id: workspace.into(),
            repo_host: Some("github.com".into()),
            repo_owner: "octo".into(),
            repo_name: format!("repo-{workspace}"),
            number,
            head_sha: format!("sha-{number}-{day}"),
            base_sha: None,
            incremental_from_sha: None,
            trigger: ReviewTrigger::Manual,
            post_mode: ReviewPostMode::Draft,
            harnesses: harnesses.iter().map(|harness| harness.to_string()).collect(),
            retry_of: None,
            publication_key: None,
        },
        at(day, 0),
    )
    .unwrap();
    let mut passes = Vec::new();
    for (slot, harness) in harnesses.iter().enumerate() {
        let pass = store::insert_pass(db, &job.id, slot as u32, 1, harness, at(day, 1)).unwrap();
        let outcome = PassOutcome { cost_microusd: 10_000, tokens_in: 100, tokens_out: 10, ..PassOutcome::default() };
        store::finish_pass(db, &pass, ReviewPassStatus::Done, None, Some(&outcome), at(day, 1 + seconds)).unwrap();
        passes.push(pass);
    }
    if let Some(first) = passes.first() {
        store::insert_findings(db, &job.id, first, findings, at(day, seconds)).unwrap();
    }
    // As the store leaves it: only a terminal job has a finish time; a draft
    // waits for a person.
    let terminal = matches!(status, "posted" | "failed" | "interrupted" | "cancelled");
    db.execute(
        "UPDATE review_jobs SET status=?2, started_at=?3, finished_at=?4, pr_title=?5 WHERE id=?1",
        params![job.id, status, store::stamp(at(day, 1)), terminal.then(|| store::stamp(at(day, 1 + seconds + 30))), format!("PR {number}")],
    )
    .unwrap();
    (job.id, passes)
}

fn decide(db: &Connection, job_id: &str, path: &str, decision: &str) {
    db.execute("UPDATE review_findings SET decision=?3 WHERE job_id=?1 AND path=?2", params![job_id, path, decision]).unwrap();
}

#[test]
fn stats_count_states_findings_and_decisions() {
    let (_scratch, db) = open();
    let (drafted, _) = job(&db, "w1", 1, 20, &["claude"], 60, "drafted", &[finding("a", ReviewSeverity::Critical), finding("b", ReviewSeverity::Nit)]);
    job(&db, "w1", 2, 20, &["codex"], 120, "posted", &[finding("c", ReviewSeverity::Major)]);
    job(&db, "w1", 3, 20, &["claude"], 30, "failed", &[]);
    job(&db, "w1", 4, 20, &["claude"], 30, "interrupted", &[]);
    job(&db, "w1", 5, 20, &["claude"], 30, "cancelled", &[]);
    job(&db, "w1", 6, 20, &["claude"], 30, "reviewing", &[]);
    decide(&db, &drafted, "a", "kept");
    decide(&db, &drafted, "b", "dropped");
    let stats = overview_at(&db, None, None).unwrap().stats;
    assert_eq!((stats.reviews, stats.running, stats.waiting, stats.posted, stats.failed, stats.cancelled), (6, 1, 1, 1, 2, 1));
    assert_eq!(stats.findings, wire::ReviewFindingCounts { total: 3, critical: 1, major: 1, minor: 0, nit: 1 });
    assert_eq!((stats.kept, stats.edited, stats.dropped, stats.pending), (1, 0, 1, 1));
    assert_eq!((stats.spend_microusd, stats.tokens_in, stats.tokens_out), (60_000, 600, 60));
}

#[test]
fn kept_rate_inputs_and_fixed_later() {
    let (_scratch, db) = open();
    let (id, _) = job(&db, "w1", 1, 20, &["claude"], 60, "posted", &[finding("a", ReviewSeverity::Major), finding("b", ReviewSeverity::Minor), finding("c", ReviewSeverity::Minor)]);
    decide(&db, &id, "a", "edited");
    decide(&db, &id, "b", "kept");
    db.execute("UPDATE review_findings SET fixed_in_sha='later' WHERE path='a'", []).unwrap();
    let stats = overview_at(&db, None, None).unwrap().stats;
    assert_eq!((stats.kept, stats.edited, stats.dropped, stats.pending), (1, 1, 0, 1), "pending is not a decision");
    assert_eq!(stats.fixed_later, 1);
}

#[test]
fn median_seconds_over_drafted_and_posted_jobs() {
    let (_scratch, db) = open();
    job(&db, "w1", 1, 20, &["claude"], 60, "drafted", &[]);
    job(&db, "w1", 2, 20, &["claude"], 300, "posted", &[]);
    job(&db, "w1", 3, 20, &["claude"], 120, "drafted", &[]);
    job(&db, "w1", 4, 20, &["claude"], 9_000, "failed", &[]);
    let overview = overview_at(&db, None, None).unwrap();
    assert_eq!(overview.stats.median_seconds, Some(120), "a failed job's time is not a review's time");
    assert_eq!(overview.harnesses[0].median_seconds, Some(120), "a pass's own time");
    let (_scratch, empty) = open();
    assert_eq!(overview_at(&empty, None, None).unwrap().stats.median_seconds, None);
}

#[test]
fn the_period_cuts_old_jobs_but_keeps_live_ones() {
    let (_scratch, db) = open();
    let (old_posted, _) = job(&db, "w1", 1, 1, &["claude"], 60, "posted", &[finding("a", ReviewSeverity::Major)]);
    let (old_draft, _) = job(&db, "w1", 2, 1, &["claude"], 60, "drafted", &[]);
    let (recent, _) = job(&db, "w1", 3, 20, &["codex"], 60, "posted", &[]);
    let overview = overview_at(&db, Some(&store::stamp(at(15, 0))), None).unwrap();
    let ids: Vec<&str> = overview.jobs.iter().map(|job| job.id.as_str()).collect();
    assert_eq!(ids, [old_draft.as_str(), recent.as_str()], "the old draft still waits for a person; the old posted job is out");
    assert!(!ids.contains(&old_posted.as_str()));
    assert_eq!(overview.stats.reviews, 1, "numbers cover the period only");
    assert!(overview.findings.is_empty());
    assert_eq!(overview.harnesses.iter().map(|row| row.harness.as_str()).collect::<Vec<_>>(), ["codex"]);
}

#[test]
fn a_merged_finding_counts_for_both_harnesses() {
    let (_scratch, db) = open();
    let (id, passes) = job(&db, "w1", 1, 20, &["claude", "codex"], 60, "drafted", &[finding("a", ReviewSeverity::Major), finding("b", ReviewSeverity::Minor)]);
    let pass_ids = serde_json::to_string(&passes).unwrap();
    db.execute("UPDATE review_findings SET agreed_by=2, pass_ids=?2, decision='kept' WHERE job_id=?1 AND path='a'", params![id, pass_ids]).unwrap();
    let overview = overview_at(&db, None, None).unwrap();
    assert_eq!(overview.stats.agreed, 1);
    let row = |harness: &str| overview.harnesses.iter().find(|row| row.harness == harness).unwrap().clone();
    assert_eq!((row("claude").findings, row("claude").kept), (2, 1));
    assert_eq!((row("codex").findings, row("codex").kept), (1, 1), "codex agreed on the merged one");
    let merged = overview.findings.iter().find(|item| item.finding.path == "a").unwrap();
    assert_eq!(merged.harnesses, ["claude", "codex"]);
    assert_eq!((merged.repo_name.as_str(), merged.number, merged.pr_title.as_deref()), ("repo-w1", 1, Some("PR 1")));
}

#[test]
fn repository_filter_limits_everything() {
    let (_scratch, db) = open();
    job(&db, "w1", 1, 20, &["claude"], 60, "drafted", &[finding("a", ReviewSeverity::Major)]);
    let (other, _) = job(&db, "w2", 2, 20, &["codex"], 60, "posted", &[finding("b", ReviewSeverity::Minor)]);
    // A second worktree of the same repository.
    let (sibling, _) = job(&db, "w2", 3, 20, &["codex"], 60, "posted", &[]);
    db.execute("UPDATE review_jobs SET workspace_id='w2-worktree' WHERE id=?1", params![sibling]).unwrap();
    let overview = overview_at(&db, None, Some("OCTO/Repo-W2")).unwrap();
    assert_eq!(overview.jobs.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(), [sibling.as_str(), other.as_str()], "any case, every workspace");
    assert_eq!((overview.stats.reviews, overview.stats.findings.total), (2, 1));
    assert_eq!(overview.findings.len(), 1);
    assert_eq!(overview.harnesses.len(), 1);
    assert_eq!(overview.repositories, ["octo/repo-w1", "octo/repo-w2"], "the filter's options do not depend on the filter");
    assert!(overview_at(&db, None, Some("no-slash")).is_err());
}

#[test]
fn jobs_and_findings_are_capped_and_newest_first() {
    let (_scratch, db) = open();
    let many: Vec<DraftFinding> = (0..2).map(|index| finding(&format!("f{index}"), ReviewSeverity::Minor)).collect();
    for number in 0..(MAX_JOBS as u64 + 5) {
        let (id, _) = job(&db, "w1", number, 20, &["claude"], 60, "posted", &many);
        db.execute("UPDATE review_jobs SET created_at=?2 WHERE id=?1", params![id, store::stamp(at(1, number as i64))]).unwrap();
    }
    let overview = overview_at(&db, None, None).unwrap();
    assert_eq!(overview.jobs.len(), MAX_JOBS);
    assert_eq!(overview.jobs[0].number, MAX_JOBS as u64 + 4, "newest first");
    assert_eq!(overview.findings.len(), MAX_FINDINGS);
    assert_eq!(overview.findings[0].number, MAX_JOBS as u64 + 4);
    assert_eq!(overview.stats.reviews, MAX_JOBS as u32 + 5, "numbers are not capped");
}

#[test]
fn a_job_without_a_title_borrows_its_prs() {
    let (_scratch, db) = open();
    let (untitled, _) = job(&db, "w1", 7, 20, &["claude"], 60, "drafted", &[finding("a", ReviewSeverity::Minor)]);
    job(&db, "w1", 7, 21, &["codex"], 60, "posted", &[]);
    job(&db, "w1", 8, 21, &["codex"], 60, "posted", &[]);
    db.execute("UPDATE review_jobs SET pr_title=NULL WHERE id=?1", params![untitled]).unwrap();
    db.execute("UPDATE review_jobs SET pr_title=NULL WHERE pr_number=8", []).unwrap();
    let overview = overview_at(&db, None, None).unwrap();
    let title = |number: u64| overview.jobs.iter().find(|job| job.number == number).unwrap().pr_title.clone();
    assert_eq!(title(7), Some("PR 7".into()), "from the other review of PR 7");
    assert_eq!(title(8), None, "no review of PR 8 knows its title");
    assert_eq!(overview.findings[0].pr_title.as_deref(), Some("PR 7"));
}
