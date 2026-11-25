use super::*;
use std::sync::{Arc, Barrier};

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-23T10:00:00Z").unwrap().with_timezone(&Utc) + Duration::seconds(seconds)
}

fn new_job(number: u64, sha: &str, harnesses: &[&str]) -> NewJob {
    NewJob {
        workspace_id: "w1".into(),
        repo_host: Some("github.com".into()),
        repo_owner: "octo".into(),
        repo_name: "repo".into(),
        number,
        head_sha: sha.into(),
        base_sha: Some("base".into()),
        incremental_from_sha: None,
        trigger: ReviewTrigger::Manual,
        post_mode: ReviewPostMode::Draft,
        harnesses: harnesses.iter().map(|harness| harness.to_string()).collect(),
        retry_of: None,
        publication_key: None,
    }
}

/// Claim the next job and hand back its lease token.
fn claim(db: &Connection, runner: &str, now: DateTime<Utc>) -> (JobRow, String) {
    let job = claim_next(db, runner, now).unwrap().expect("a claimable job");
    let token = job.lease_owner.clone().unwrap();
    (job, token)
}

fn status(db: &Connection, id: &str) -> Status {
    require_job(db, id).unwrap().status
}

#[test]
fn migration_creates_the_review_tables_and_indexes() {
    let (_scratch, db) = open();
    for table in ["review_jobs", "review_passes", "review_findings"] {
        let found: i64 = db
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1", params![table], |row| row.get(0))
            .unwrap();
        assert_eq!(found, 1, "{table}");
    }
    for index in ["idx_review_jobs_pr", "idx_review_jobs_status", "idx_review_jobs_active_dedupe"] {
        let sql: String = db
            .query_row("SELECT sql FROM sqlite_master WHERE type='index' AND name=?1", params![index], |row| row.get(0))
            .unwrap();
        if index == "idx_review_jobs_active_dedupe" {
            assert!(sql.contains("UNIQUE") && sql.contains("WHERE status IN"), "{sql}");
        }
    }
    let columns: Vec<String> = db
        .prepare("SELECT name FROM pragma_table_info('review_jobs')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for column in [
        "workspace_id", "repo_owner", "repo_name", "pr_number", "head_sha", "base_sha", "incremental_from_sha",
        "trigger_kind", "post_mode", "status", "error", "posted_review_id", "posted_review_url", "created_at",
        "started_at", "finished_at", "lease_owner", "lease_expires_at", "attempts", "retry_of", "dedupe_key",
    ] {
        assert!(columns.iter().any(|name| name == column), "review_jobs.{column}");
    }
}

#[test]
fn start_is_idempotent_for_the_same_pr_sha_and_harness_set() {
    let (_scratch, db) = open();
    let (first, created) = insert(&db, &new_job(7, "sha1", &["claude", "codex"]), at(0)).unwrap();
    assert!(created);
    assert_eq!(first.status, Status::Queued);
    let (again, created) = insert(&db, &new_job(7, "sha1", &["codex", "claude"]), at(5)).unwrap();
    assert!(!created, "harness order does not make a new review");
    assert_eq!(again.id, first.id);
    // Still the same job while it is drafted: a draft is not terminal.
    db.execute("UPDATE review_jobs SET status='drafted' WHERE id=?1", params![first.id]).unwrap();
    assert_eq!(insert(&db, &new_job(7, "sha1", &["claude", "codex"]), at(6)).unwrap().0.id, first.id);
    let count: i64 = db.query_row("SELECT COUNT(*) FROM review_jobs", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_new_job_is_created_once_the_previous_one_is_terminal() {
    let (_scratch, db) = open();
    let (first, _) = insert(&db, &new_job(7, "sha1", &["claude"]), at(0)).unwrap();
    for terminal in [Status::Failed, Status::Cancelled, Status::Interrupted] {
        db.execute("UPDATE review_jobs SET status=?2 WHERE id=?1", params![first.id, terminal.as_str()]).unwrap();
        let (next, created) = insert(&db, &new_job(7, "sha1", &["claude"]), at(1)).unwrap();
        assert!(created, "{}", terminal.as_str());
        assert_ne!(next.id, first.id);
        db.execute("DELETE FROM review_jobs WHERE id=?1", params![next.id]).unwrap();
    }
    db.execute("UPDATE review_jobs SET status='posted' WHERE id=?1", params![first.id]).unwrap();
    assert!(insert(&db, &new_job(7, "sha1", &["claude"]), at(2)).unwrap().1);
}

#[test]
fn a_different_harness_set_is_a_different_job() {
    let (_scratch, db) = open();
    let (claude, _) = insert(&db, &new_job(7, "sha1", &["claude"]), at(0)).unwrap();
    let (pair, created) = insert(&db, &new_job(7, "sha1", &["claude", "codex"]), at(1)).unwrap();
    assert!(created);
    assert_ne!(pair.id, claude.id);
    let (moved, created) = insert(&db, &new_job(7, "sha2", &["claude"]), at(2)).unwrap();
    assert!(created, "a new head is a new review");
    assert_ne!(moved.id, claude.id);
    let (other_pr, created) = insert(&db, &new_job(8, "sha1", &["claude"]), at(3)).unwrap();
    assert!(created);
    assert_ne!(other_pr.id, claude.id);
    assert!(insert(&db, &new_job(9, "sha1", &[]), at(4)).unwrap_err().to_string().contains("at least one harness"));
}

#[test]
fn claims_are_fifo() {
    let (_scratch, db) = open();
    let ids: Vec<String> = (0..3).map(|n| insert(&db, &new_job(n, "sha", &["claude"]), at(n as i64)).unwrap().0.id).collect();
    for expected in &ids {
        let claimed = claim_next(&db, "runner-a", at(10)).unwrap().unwrap();
        assert_eq!(&claimed.id, expected);
        assert_eq!(claimed.status, Status::Preparing);
        assert_eq!(claimed.attempts, 1);
        assert!(claimed.lease_owner.as_deref().is_some_and(|token| token.starts_with("runner-a/")), "{:?}", claimed.lease_owner);
        assert_eq!(claimed.lease_expires_at, Some(stamp(at(10 + LEASE_SECONDS))));
        assert!(claimed.started_at.is_some());
    }
    assert!(claim_next(&db, "runner-a", at(11)).unwrap().is_none());
}

#[test]
fn two_runners_racing_for_one_job_produce_one_claim() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("bridge.db");
    let seed = crate::store::open(&path).unwrap();
    for round in 0..20u64 {
        insert(&seed, &new_job(round, "sha", &["claude"]), at(round as i64)).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let racers: Vec<_> = ["runner-a", "runner-b"]
            .into_iter()
            .map(|owner| {
                let barrier = barrier.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    let db = crate::store::open(&path).unwrap();
                    barrier.wait();
                    claim_next(&db, owner, at(100)).unwrap().map(|job| job.id)
                })
            })
            .collect();
        let wins: Vec<String> = racers.into_iter().filter_map(|racer| racer.join().unwrap()).collect();
        assert_eq!(wins.len(), 1, "round {round}: exactly one runner claims the job, got {wins:?}");
    }
}

#[test]
fn renewal_fails_once_the_job_is_cancelled_or_reclaimed() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    let (_, token) = claim(&db, "runner-a", at(0));
    assert!(renew(&db, &job.id, &token, at(15)).unwrap());
    assert!(!renew(&db, &job.id, "runner-a", at(16)).unwrap(), "the runner id alone is not the claim");
    cancel(&db, &job.id, at(20)).unwrap();
    assert!(!renew(&db, &job.id, &token, at(21)).unwrap(), "a cancelled job cannot be renewed");

    let (second, _) = insert(&db, &new_job(2, "sha", &["claude"]), at(30)).unwrap();
    let (_, token) = claim(&db, "runner-a", at(30));
    assert!(!renew(&db, &second.id, &token, at(30 + LEASE_SECONDS + 1)).unwrap(), "an expired lease is not renewed");
    reclaim_expired(&db, at(30 + LEASE_SECONDS + 1)).unwrap();
    assert!(!renew(&db, &second.id, &token, at(30 + LEASE_SECONDS + 2)).unwrap());
}

#[test]
fn an_expired_lease_returns_the_job_to_the_queue() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    let (_, first) = claim(&db, "runner-a", at(0));
    assert!(reclaim_expired(&db, at(LEASE_SECONDS - 1)).unwrap().is_empty(), "a live lease is left alone");
    assert!(advance(&db, &job.id, &first, Status::Preparing, Status::Reviewing, false).unwrap());
    assert_eq!(reclaim_expired(&db, at(LEASE_SECONDS)).unwrap(), [job.id.clone()]);
    let reclaimed = require_job(&db, &job.id).unwrap();
    assert_eq!(reclaimed.status, Status::Queued);
    assert_eq!(reclaimed.lease_owner, None);
    let next = claim_next(&db, "runner-b", at(LEASE_SECONDS + 1)).unwrap().unwrap();
    assert_eq!(next.id, job.id);
    assert_eq!(next.attempts, 2);
    assert!(
        !advance(&db, &job.id, &first, Status::Preparing, Status::Reviewing, false).unwrap(),
        "the runner that lost the lease writes nothing"
    );
}

#[test]
fn an_expired_posting_lease_keeps_the_job_posting_without_a_lease() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    let (_, token) = claim(&db, "runner-a", at(0));
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    assert!(advance(&db, &job.id, &token, Status::Reviewing, Status::Drafted, false).unwrap());
    assert!(advance(&db, &job.id, &token, Status::Drafted, Status::Posting, false).unwrap());
    reclaim_expired(&db, at(LEASE_SECONDS + 1)).unwrap();
    let reclaimed = require_job(&db, &job.id).unwrap();
    assert_eq!(reclaimed.status, Status::Posting);
    assert_eq!(reclaimed.lease_owner, None);
    let (claimed, token) = claim(&db, "runner-b", at(LEASE_SECONDS + 2));
    assert_eq!(claimed.status, Status::Posting, "a posting job is claimed as-is");
    let posted = PostedReview { review_id: "r1".into(), url: "https://example.test/r1".into(), commit_id: None, findings: Vec::new() };
    assert!(mark_posted(&db, &job.id, &token, &posted, at(LEASE_SECONDS + 3)).unwrap());
    let posted = require_job(&db, &job.id).unwrap();
    assert_eq!(posted.status, Status::Posted);
    assert_eq!(posted.posted_review_url.as_deref(), Some("https://example.test/r1"));
    assert!(posted.finished_at.is_some());
}

#[test]
fn the_third_reclaim_fails_the_job() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    for claim in 0..MAX_ATTEMPTS as i64 {
        let start = claim * 100;
        claim_next(&db, "runner", at(start)).unwrap().unwrap();
        reclaim_expired(&db, at(start + LEASE_SECONDS)).unwrap();
    }
    let failed = require_job(&db, &job.id).unwrap();
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(failed.error.as_deref(), Some(STOPPED_RESPONDING));
    assert_eq!(failed.attempts, MAX_ATTEMPTS);
    assert!(claim_next(&db, "runner", at(1000)).unwrap().is_none());
}

#[test]
fn startup_recovery_interrupts_in_flight_jobs_only() {
    let (_scratch, db) = open();
    let mut ids = std::collections::HashMap::new();
    for (number, label) in ["queued", "preparing", "reviewing", "drafted", "posting", "posted", "failed"].into_iter().enumerate() {
        // An automatic post: a post a person asked for returns to their draft
        // (`a_draft_post_interrupted_by_restart_returns_to_the_draft`).
        let new = NewJob { post_mode: ReviewPostMode::Auto, ..new_job(number as u64, "sha", &["claude"]) };
        let (job, _) = insert(&db, &new, at(number as i64)).unwrap();
        db.execute(
            "UPDATE review_jobs SET status=?2, lease_owner=CASE WHEN ?2 IN ('preparing','reviewing','posting') THEN 'old' END
             WHERE id=?1",
            params![job.id, label],
        )
        .unwrap();
        ids.insert(label, job.id);
    }
    let mut interrupted = interrupt_in_flight(&db, at(100)).unwrap();
    interrupted.sort();
    let mut expected = vec![ids["preparing"].clone(), ids["reviewing"].clone(), ids["posting"].clone()];
    expected.sort();
    assert_eq!(interrupted, expected);
    for label in ["preparing", "reviewing", "posting"] {
        let job = require_job(&db, &ids[label]).unwrap();
        assert_eq!(job.status, Status::Interrupted, "{label}");
        assert_eq!(job.error.as_deref(), Some(RESTARTED));
        assert_eq!(job.lease_owner, None);
        assert!(job.finished_at.is_some());
    }
    for (label, expected) in [
        ("queued", Status::Queued),
        ("drafted", Status::Drafted),
        ("posted", Status::Posted),
        ("failed", Status::Failed),
    ] {
        assert_eq!(status(&db, &ids[label]), expected, "{label} is left alone");
    }
}

#[test]
fn list_is_newest_first_and_capped_at_fifty() {
    let (_scratch, db) = open();
    for n in 0..55u64 {
        insert(&db, &new_job(n % 2, &format!("sha{n}"), &["claude"]), at(n as i64)).unwrap();
    }
    let all = list(&db, "w1", None).unwrap();
    assert_eq!(all.len(), LIST_LIMIT as usize);
    assert_eq!(all[0].head_sha, "sha54");
    assert!(all.windows(2).all(|pair| pair[0].created_at >= pair[1].created_at));
    let odd = list(&db, "w1", Some(1)).unwrap();
    assert_eq!(odd.len(), 27);
    assert!(odd.iter().all(|job| job.number == 1));
    assert!(list(&db, "other", None).unwrap().is_empty());
}

#[test]
fn retry_copies_the_inputs_and_links_retry_of() {
    let (_scratch, db) = open();
    let mut original = new_job(7, "sha1", &["claude", "codex"]);
    original.trigger = ReviewTrigger::Push;
    original.post_mode = ReviewPostMode::Auto;
    let (job, _) = insert(&db, &original, at(0)).unwrap();
    finish(&db, &job.id, None, Status::Interrupted, Some(RESTARTED), at(1)).unwrap();
    let (retried, created) = retry(&db, &job.id, at(2)).unwrap();
    assert!(created);
    assert_ne!(retried.id, job.id);
    assert_eq!(retried.retry_of.as_deref(), Some(job.id.as_str()));
    assert_eq!(retried.status, Status::Queued);
    assert_eq!(
        (retried.workspace_id.as_str(), retried.number, retried.head_sha.as_str(), retried.trigger, retried.post_mode),
        ("w1", 7, "sha1", ReviewTrigger::Push, ReviewPostMode::Auto)
    );
    assert_eq!(retried.harnesses, ["claude", "codex"]);
    assert_eq!(retried.error, None);
    let (again, created) = retry(&db, &job.id, at(3)).unwrap();
    assert!(!created, "retrying twice returns the active retry");
    assert_eq!(again.id, retried.id);
}

#[test]
fn retry_refuses_an_active_job() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(7, "sha1", &["claude"]), at(0)).unwrap();
    for active in ["queued", "preparing", "reviewing", "drafted", "posting", "posted"] {
        db.execute("UPDATE review_jobs SET status=?2 WHERE id=?1", params![job.id, active]).unwrap();
        assert!(retry(&db, &job.id, at(1)).unwrap_err().to_string().contains("Only a failed"), "{active}");
    }
    assert!(retry(&db, "missing", at(1)).unwrap_err().to_string().contains("No review job"));
}

#[test]
fn cancel_settles_a_queued_job_and_refuses_a_terminal_one() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(7, "sha1", &["claude"]), at(0)).unwrap();
    let cancelled = cancel(&db, &job.id, at(1)).unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    assert_eq!(cancelled.finished_at, Some(stamp(at(1))));
    assert!(cancel(&db, &job.id, at(2)).unwrap_err().to_string().contains("already cancelled"));
    let (held, _) = insert(&db, &new_job(8, "sha1", &["claude"]), at(3)).unwrap();
    claim_next(&db, "runner", at(3)).unwrap().unwrap();
    assert_eq!(cancel(&db, &held.id, at(4)).unwrap().lease_owner, None, "cancel takes the lease away");
}

#[test]
fn advance_and_finish_follow_the_state_table() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(7, "sha1", &["claude"]), at(0)).unwrap();
    let (_, token) = claim(&db, "runner", at(0));
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Posting, false).is_err());
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Failed, false).is_err(), "terminal goes through finish");
    assert!(finish(&db, &job.id, None, Status::Posted, None, at(1)).is_err(), "posted goes through mark_posted");
    assert!(!finish(&db, &job.id, Some("someone-else"), Status::Failed, Some("x"), at(1)).unwrap());
    assert!(finish(&db, &job.id, Some(&token), Status::Failed, Some("boom"), at(1)).unwrap());
    let failed = require_job(&db, &job.id).unwrap();
    assert_eq!((failed.status, failed.error.as_deref()), (Status::Failed, Some("boom")));
    assert!(!finish(&db, &job.id, None, Status::Cancelled, None, at(2)).unwrap(), "nothing leaves a terminal status");
}

#[test]
fn passes_and_findings_round_trip_into_the_wire_job() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(7, "sha1", &["claude", "codex"]), at(0)).unwrap();
    let failed = insert_pass(&db, &job.id, 1, 1, "codex", at(1)).unwrap();
    finish_pass(&db, &failed, ReviewPassStatus::Failed, Some("signed out"), None, at(2)).unwrap();
    let fallback = insert_pass(&db, &job.id, 1, 1, "opencode", at(3)).unwrap();
    let outcome = PassOutcome { session_id: Some("hidden-1".into()), tokens_in: 10, tokens_out: 5, cost_microusd: 7, denied_requests: 2, ..Default::default() };
    finish_pass(&db, &fallback, ReviewPassStatus::Done, None, Some(&outcome), at(4)).unwrap();
    let finding = |severity, confidence| DraftFinding {
        path: "src/a.rs".into(),
        line: 4,
        start_line: Some(2),
        side: ReviewSide::Right,
        severity,
        title: "t".into(),
        body: "b".into(),
        confidence,
        fingerprint: "f".into(),
        outside_diff: false,
    };
    insert_findings(&db, &job.id, &fallback, &[finding(ReviewSeverity::Nit, 0.9), finding(ReviewSeverity::Critical, 1.7)], at(5)).unwrap();

    let wire_job = to_wire(&db, &require_job(&db, &job.id).unwrap()).unwrap();
    assert_eq!(wire_job.harnesses, ["claude", "opencode"], "slot 1 reports the harness that actually ran");
    assert_eq!(wire_job.finding_counts, wire::ReviewFindingCounts { total: 2, critical: 1, major: 0, minor: 0, nit: 1 });
    let stored = passes(&db, &job.id).unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!((stored[0].status, stored[0].error.as_deref()), (ReviewPassStatus::Failed, Some("signed out")));
    assert_eq!((stored[1].tokens_in, stored[1].denied_requests), (10, 2));
    assert_eq!(stored[1].session_id.as_deref(), Some("hidden-1"));
    let stored = findings(&db, &job.id).unwrap();
    assert_eq!(stored[0].severity, ReviewSeverity::Critical, "ranked by severity");
    assert_eq!(stored[0].confidence, 1.0, "confidence is clamped");
    assert_eq!(stored[0].decision, ReviewFindingDecision::Pending);
    assert_eq!(stored[0].start_line, Some(2));
}

fn draft(path: &str) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line: 4,
        start_line: None,
        side: ReviewSide::Right,
        severity: ReviewSeverity::Major,
        title: "t".into(),
        body: "b".into(),
        confidence: 0.5,
        fingerprint: format!("fp-{path}"),
        outside_diff: false,
    }
}

#[test]
fn each_claim_gets_its_own_lease_token() {
    let (_scratch, db) = open();
    let (a, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    insert(&db, &new_job(2, "sha", &["claude"]), at(1)).unwrap();
    let (_, first) = claim(&db, "runner", at(2));
    let (_, second) = claim(&db, "runner", at(2));
    assert_ne!(first, second, "one runner, two jobs, two tokens");
    reclaim_expired(&db, at(2 + LEASE_SECONDS)).unwrap();
    let (again, third) = claim(&db, "runner", at(100));
    assert_eq!(again.id, a.id);
    assert_ne!(third, first, "the same runner re-claiming the same job gets a new token");
    assert!(third.starts_with("runner/"));
}

#[test]
fn a_superseded_claim_cannot_advance_or_renew_the_replacement() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    let (_, stale) = claim(&db, "runner", at(0));
    assert!(advance(&db, &job.id, &stale, Status::Preparing, Status::Reviewing, false).unwrap());
    reclaim_expired(&db, at(LEASE_SECONDS)).unwrap();
    // The same runner claims the job again into another slot.
    let (_, current) = claim(&db, "runner", at(LEASE_SECONDS + 1));
    assert!(advance(&db, &job.id, &current, Status::Preparing, Status::Reviewing, false).unwrap());
    let now = at(LEASE_SECONDS + 2);
    assert!(!renew(&db, &job.id, &stale, now).unwrap());
    assert!(!advance(&db, &job.id, &stale, Status::Reviewing, Status::Drafted, true).unwrap());
    assert!(!finish_review(&db, &job.id, &stale, "stale summary", None, true).unwrap());
    assert!(begin_pass(&db, &job.id, &stale, 0, "claude", now).unwrap().is_none());
    assert!(!finish(&db, &job.id, Some(&stale), Status::Failed, Some("stale"), now).unwrap());
    assert_eq!(status(&db, &job.id), Status::Reviewing, "the replacement still holds the job");
    assert!(renew(&db, &job.id, &current, now).unwrap());
    let pass = begin_pass(&db, &job.id, &current, 0, "claude", now).unwrap().unwrap();
    let outcome = PassOutcome { findings: vec![draft("src/a.rs")], ..Default::default() };
    assert!(!complete_pass(&db, &job.id, &stale, &pass, &outcome, now).unwrap(), "a stale claim stores no pass");
    assert!(findings(&db, &job.id).unwrap().is_empty());
    assert!(complete_pass(&db, &job.id, &current, &pass, &outcome, now).unwrap());
    assert!(finish_review(&db, &job.id, &current, "summary", None, true).unwrap());
    let drafted = require_job(&db, &job.id).unwrap();
    assert_eq!((drafted.status, drafted.summary.as_deref(), drafted.lease_owner), (Status::Drafted, Some("summary"), None));
    assert_eq!(passes(&db, &job.id).unwrap()[0].attempt, 2);
}

#[test]
fn reclaiming_settles_abandoned_passes_and_drops_their_findings() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude", "codex"]), at(0)).unwrap();
    let (_, first) = claim(&db, "runner", at(0));
    assert!(advance(&db, &job.id, &first, Status::Preparing, Status::Reviewing, false).unwrap());
    let done = begin_pass(&db, &job.id, &first, 0, "claude", at(1)).unwrap().unwrap();
    let outcome = PassOutcome { findings: vec![draft("src/a.rs")], ..Default::default() };
    assert!(complete_pass(&db, &job.id, &first, &done, &outcome, at(2)).unwrap());
    let running = begin_pass(&db, &job.id, &first, 1, "codex", at(3)).unwrap().unwrap();
    assert_eq!(to_wire(&db, &require_job(&db, &job.id).unwrap()).unwrap().finding_counts.total, 1);

    // The runner dies during slot 1; nothing of it is left to settle anything.
    reclaim_expired(&db, at(LEASE_SECONDS + 1)).unwrap();
    let (_, second) = claim(&db, "runner", at(LEASE_SECONDS + 2));
    let stored = passes(&db, &job.id).unwrap();
    let abandoned = stored.iter().find(|pass| pass.id == running).unwrap();
    assert_eq!((abandoned.status, abandoned.error.as_deref()), (ReviewPassStatus::Cancelled, Some(ABANDONED)));
    assert!(findings(&db, &job.id).unwrap().is_empty(), "the abandoned attempt's findings are gone");
    assert_eq!(to_wire(&db, &require_job(&db, &job.id).unwrap()).unwrap().finding_counts.total, 0);

    assert!(advance(&db, &job.id, &second, Status::Preparing, Status::Reviewing, false).unwrap());
    let again = begin_pass(&db, &job.id, &second, 0, "claude", at(LEASE_SECONDS + 3)).unwrap().unwrap();
    assert!(complete_pass(&db, &job.id, &second, &again, &outcome, at(LEASE_SECONDS + 4)).unwrap());
    assert_eq!(findings(&db, &job.id).unwrap().len(), 1, "one finding, not the old one plus the new one");
    let attempts: Vec<u32> = passes(&db, &job.id).unwrap().iter().map(|pass| pass.attempt).collect();
    assert_eq!(attempts, [1, 1, 2]);
}

#[test]
fn a_terminal_transition_settles_running_passes() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(1, "sha", &["claude"]), at(0)).unwrap();
    let (_, token) = claim(&db, "runner", at(0));
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    begin_pass(&db, &job.id, &token, 0, "claude", at(1)).unwrap().unwrap();
    interrupt_in_flight(&db, at(2)).unwrap();
    let pass = &passes(&db, &job.id).unwrap()[0];
    assert_eq!((pass.status, pass.error.as_deref()), (ReviewPassStatus::Cancelled, Some(RESTARTED)));
}

// --- triage and posting (testing/feat-review-draft-post.md) ---------------------

/// A drafted job of PR `number` waiting for a person, with one finding per
/// path. Returns the job and its finding ids in path order.
fn waiting_draft_with(db: &Connection, number: u64, mode: ReviewPostMode, paths: &[&str]) -> (JobRow, Vec<String>) {
    let new = NewJob { post_mode: mode, ..new_job(number, "sha", &["claude"]) };
    let (job, _) = insert(db, &new, at(0)).unwrap();
    let claimed = claim_next(db, "runner", at(0)).unwrap().unwrap();
    assert_eq!(claimed.id, job.id);
    let token = claimed.lease_owner.unwrap();
    assert!(advance(db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = begin_pass(db, &job.id, &token, 0, "claude", at(1)).unwrap().unwrap();
    let outcome = PassOutcome { findings: paths.iter().map(|path| draft(path)).collect(), ..Default::default() };
    assert!(complete_pass(db, &job.id, &token, &pass, &outcome, at(2)).unwrap());
    assert!(finish_review(db, &job.id, &token, "summary", None, true).unwrap());
    let mut ids: Vec<(String, String)> =
        findings(db, &job.id).unwrap().into_iter().map(|finding| (finding.path, finding.id)).collect();
    ids.sort();
    (require_job(db, &job.id).unwrap(), ids.into_iter().map(|(_, id)| id).collect())
}

fn decision(id: &str, decision: ReviewFindingDecision, edited_body: Option<&str>) -> wire::ReviewFindingDecisionInput {
    wire::ReviewFindingDecisionInput { finding_id: id.into(), decision, edited_body: edited_body.map(str::to_owned) }
}

fn decisions_of(db: &Connection, job_id: &str) -> Vec<(String, ReviewFindingDecision, Option<String>, bool)> {
    let mut found: Vec<_> = findings(db, job_id)
        .unwrap()
        .into_iter()
        .map(|finding| (finding.path, finding.decision, finding.edited_body, finding.decided_at.is_some()))
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

#[test]
fn decisions_are_stored_with_a_timestamp_and_survive_reopening() {
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("bridge.db");
    let job_id = {
        let db = crate::store::open(&path).unwrap();
        let (job, ids) = waiting_draft_with(&db, 1, ReviewPostMode::Draft, &["a.rs", "b.rs", "c.rs"]);
        decide(
            &db,
            &job.id,
            &[
                decision(&ids[0], ReviewFindingDecision::Kept, None),
                decision(&ids[1], ReviewFindingDecision::Edited, Some("  Use the guard here.  ")),
            ],
            at(10),
        )
        .unwrap();
        // A later decision replaces an earlier one, and keeping clears an edit.
        decide(&db, &job.id, &[decision(&ids[0], ReviewFindingDecision::Dropped, None)], at(11)).unwrap();
        job.id
    };
    let db = crate::store::open(&path).unwrap();
    assert_eq!(
        decisions_of(&db, &job_id),
        [
            ("a.rs".to_owned(), ReviewFindingDecision::Dropped, None, true),
            ("b.rs".to_owned(), ReviewFindingDecision::Edited, Some("Use the guard here.".to_owned()), true),
            ("c.rs".to_owned(), ReviewFindingDecision::Pending, None, false),
        ]
    );
    let stored = findings(&db, &job_id).unwrap();
    let dropped = stored.iter().find(|finding| finding.path == "a.rs").unwrap();
    assert_eq!(dropped.decided_at.as_deref(), Some(stamp(at(11)).as_str()));
    let ids: Vec<String> = stored.iter().filter(|finding| finding.path == "b.rs").map(|finding| finding.id.clone()).collect();
    decide(&db, &job_id, &[decision(&ids[0], ReviewFindingDecision::Kept, None)], at(12)).unwrap();
    assert_eq!(decisions_of(&db, &job_id)[1], ("b.rs".to_owned(), ReviewFindingDecision::Kept, None, true), "keeping clears the edit");
}

#[test]
fn decide_refuses_outside_a_waiting_draft_and_bad_batches() {
    let (_scratch, db) = open();
    let (job, ids) = waiting_draft_with(&db, 1, ReviewPostMode::Draft, &["a.rs", "b.rs"]);
    let (other, other_ids) = waiting_draft_with(&db, 2, ReviewPostMode::Draft, &["z.rs"]);
    let kept = decision(&ids[0], ReviewFindingDecision::Kept, None);
    let refusals: Vec<(Vec<wire::ReviewFindingDecisionInput>, &str)> = vec![
        (vec![kept.clone(), decision("missing", ReviewFindingDecision::Kept, None)], "no finding missing"),
        (vec![kept.clone(), decision(&other_ids[0], ReviewFindingDecision::Kept, None)], "no finding"),
        (vec![kept.clone(), kept.clone()], "decided twice"),
        (vec![decision(&ids[1], ReviewFindingDecision::Pending, None)], "kept, dropped or edited"),
        (vec![decision(&ids[1], ReviewFindingDecision::Edited, None)], "needs its new body"),
        (vec![decision(&ids[1], ReviewFindingDecision::Edited, Some("   "))], "needs its new body"),
        (vec![decision(&ids[1], ReviewFindingDecision::Kept, Some("text"))], "Only an edited finding"),
        (
            vec![decision(&ids[1], ReviewFindingDecision::Edited, Some(&"x".repeat(MAX_EDITED_BODY_CHARS + 1)))],
            "at most",
        ),
    ];
    for (batch, reason) in refusals {
        let refused = decide(&db, &job.id, &batch, at(5)).unwrap_err().to_string();
        assert!(refused.contains(reason), "{refused} should say {reason}");
        assert!(decisions_of(&db, &job.id).iter().all(|row| row.1 == ReviewFindingDecision::Pending), "all or nothing");
    }
    assert!(decisions_of(&db, &other.id).iter().all(|row| row.1 == ReviewFindingDecision::Pending));

    // A draft a runner still holds (auto mode on its way to posting), a job
    // posting, and a finished one are not a person's to triage.
    db.execute("UPDATE review_jobs SET lease_owner='runner/x' WHERE id=?1", params![job.id]).unwrap();
    assert!(decide(&db, &job.id, &[kept.clone()], at(6)).unwrap_err().to_string().contains("waiting for you"));
    for status in ["posting", "posted", "cancelled"] {
        db.execute("UPDATE review_jobs SET status=?2, lease_owner=NULL WHERE id=?1", params![job.id, status]).unwrap();
        let refused = decide(&db, &job.id, &[kept.clone()], at(6)).unwrap_err().to_string();
        assert!(refused.contains(status), "{refused}");
    }
}

#[test]
fn post_requires_a_kept_finding_or_no_findings() {
    let (_scratch, db) = open();
    let (job, ids) = waiting_draft_with(&db, 1, ReviewPostMode::Draft, &["a.rs", "b.rs"]);
    assert!(request_post(&db, &job.id).unwrap_err().to_string().contains("Keep or edit at least one"));
    decide(&db, &job.id, &[decision(&ids[0], ReviewFindingDecision::Dropped, None)], at(3)).unwrap();
    assert!(request_post(&db, &job.id).unwrap_err().to_string().contains("Keep or edit at least one"));
    assert_eq!(status(&db, &job.id), Status::Drafted);
    decide(&db, &job.id, &[decision(&ids[1], ReviewFindingDecision::Edited, Some("better"))], at(4)).unwrap();
    db.execute("UPDATE review_jobs SET error='the last post failed' WHERE id=?1", params![job.id]).unwrap();
    let posting = request_post(&db, &job.id).unwrap();
    assert_eq!((posting.status, posting.error, posting.lease_owner), (Status::Posting, None, None));
    assert!(request_post(&db, &job.id).unwrap_err().to_string().contains("posting"), "only once");
    let claimed = claim_next(&db, "runner", at(5)).unwrap().unwrap();
    assert_eq!((claimed.id, claimed.status), (job.id, Status::Posting), "the runner picks the post up");

    let (empty, _) = waiting_draft_with(&db, 2, ReviewPostMode::Draft, &[]);
    assert_eq!(request_post(&db, &empty.id).unwrap().status, Status::Posting, "a summary-only review");
}

#[test]
fn marking_posted_records_comment_ids_and_posted_at() {
    let (_scratch, db) = open();
    let (job, ids) = waiting_draft_with(&db, 1, ReviewPostMode::Draft, &["a.rs", "b.rs", "c.rs"]);
    decide(
        &db,
        &job.id,
        &[decision(&ids[0], ReviewFindingDecision::Kept, None), decision(&ids[1], ReviewFindingDecision::Kept, None)],
        at(3),
    )
    .unwrap();
    request_post(&db, &job.id).unwrap();
    let token = claim_next(&db, "runner", at(4)).unwrap().unwrap().lease_owner.unwrap();
    let posted = PostedReview {
        review_id: "9".into(),
        url: "https://github.com/octo/repo/pull/1#pullrequestreview-9".into(),
        commit_id: None,
        findings: vec![
            crate::reviews::steps::PostedFinding { finding_id: ids[0].clone(), comment_id: Some("101".into()) },
            crate::reviews::steps::PostedFinding { finding_id: ids[1].clone(), comment_id: None },
        ],
    };
    assert!(!mark_posted(&db, &job.id, "runner/stale", &posted, at(5)).unwrap(), "only the holder");
    assert!(findings(&db, &job.id).unwrap().iter().all(|finding| finding.posted_at.is_none()));
    assert!(mark_posted(&db, &job.id, &token, &posted, at(5)).unwrap());
    let job = require_job(&db, &job.id).unwrap();
    assert_eq!((job.status, job.posted_review_id.as_deref()), (Status::Posted, Some("9")));
    let mut stored: Vec<_> = findings(&db, &job.id)
        .unwrap()
        .into_iter()
        .map(|finding| (finding.path, finding.github_comment_id, finding.posted_at))
        .collect();
    stored.sort();
    let at5 = Some(stamp(at(5)));
    assert_eq!(
        stored,
        [
            ("a.rs".to_owned(), Some("101".to_owned()), at5.clone()),
            ("b.rs".to_owned(), None, at5),
            ("c.rs".to_owned(), None, None),
        ]
    );
}

#[test]
fn a_draft_post_interrupted_by_restart_returns_to_the_draft() {
    let (_scratch, db) = open();
    let (job, ids) = waiting_draft_with(&db, 1, ReviewPostMode::Draft, &["a.rs"]);
    decide(&db, &job.id, &[decision(&ids[0], ReviewFindingDecision::Kept, None)], at(3)).unwrap();
    request_post(&db, &job.id).unwrap();
    claim_next(&db, "runner", at(4)).unwrap().unwrap();
    let (auto, _) = waiting_draft_with(&db, 2, ReviewPostMode::Auto, &["a.rs"]);
    db.execute("UPDATE review_jobs SET status='posting' WHERE id=?1", params![auto.id]).unwrap();

    let mut settled = interrupt_in_flight(&db, at(10)).unwrap();
    settled.sort();
    let mut expected = vec![job.id.clone(), auto.id.clone()];
    expected.sort();
    assert_eq!(settled, expected);
    let back = require_job(&db, &job.id).unwrap();
    assert_eq!((back.status, back.error.as_deref(), back.lease_owner), (Status::Drafted, Some(RESTARTED_POSTING), None));
    assert_eq!(back.finished_at, None);
    assert_eq!(decisions_of(&db, &job.id)[0].1, ReviewFindingDecision::Kept, "the triage survives");
    assert_eq!(request_post(&db, &job.id).unwrap().status, Status::Posting, "and it can post again");
    let auto = require_job(&db, &auto.id).unwrap();
    assert_eq!((auto.status, auto.error.as_deref()), (Status::Interrupted, Some(RESTARTED)));
}

#[test]
fn a_re_review_marks_fixed_only_findings_of_the_same_pr_that_were_kept_edited_or_posted() {
    let (_scratch, db) = open();
    // The earlier review of #7, and a review of another PR.
    let (earlier, _) = insert(&db, &new_job(7, "old-head", &["claude"]), at(0)).unwrap();
    let (other_pr, _) = insert(&db, &new_job(8, "x", &["claude"]), at(0)).unwrap();
    for job in [&earlier, &other_pr] {
        let pass = insert_pass(&db, &job.id, 0, 1, "claude", at(0)).unwrap();
        insert_findings(&db, &job.id, &pass, &[draft("src/kept.rs"), draft("src/edited.rs"), draft("src/posted.rs"), draft("src/dropped.rs"), draft("src/pending.rs")], at(0)).unwrap();
    }
    db.execute_batch(
        "UPDATE review_jobs SET status='posted' WHERE pr_number IN (7, 8);
         UPDATE review_findings SET decision='kept' WHERE path='src/kept.rs';
         UPDATE review_findings SET decision='edited', edited_body='e' WHERE path='src/edited.rs';
         UPDATE review_findings SET posted_at='2026-11-23T10:00:00Z' WHERE path='src/posted.rs';
         UPDATE review_findings SET decision='dropped' WHERE path='src/dropped.rs';",
    )
    .unwrap();
    let id_of = |job: &JobRow, path: &str| findings(&db, &job.id).unwrap().into_iter().find(|finding| finding.path == path).unwrap().id;
    let said: Vec<String> = ["src/kept.rs", "src/edited.rs", "src/posted.rs", "src/dropped.rs", "src/pending.rs"]
        .iter()
        .map(|path| id_of(&earlier, path))
        .chain([id_of(&other_pr, "src/kept.rs")])
        .collect();

    let (_, _) = insert(&db, &NewJob { incremental_from_sha: Some("old-head".into()), ..new_job(7, "new-head", &["claude"]) }, at(1)).unwrap();
    let (job, token) = claim(&db, "runner", at(2));
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = begin_pass(&db, &job.id, &token, 0, "claude", at(3)).unwrap().unwrap();
    let outcome = PassOutcome { fixed: said, ..Default::default() };
    assert!(complete_pass(&db, &job.id, &token, &pass, &outcome, at(4)).unwrap());

    let fixed_in = |job: &JobRow, path: &str| {
        findings(&db, &job.id).unwrap().into_iter().find(|finding| finding.path == path).unwrap().fixed_in_sha
    };
    for path in ["src/kept.rs", "src/edited.rs", "src/posted.rs"] {
        assert_eq!(fixed_in(&earlier, path).as_deref(), Some("new-head"), "{path}");
    }
    for path in ["src/dropped.rs", "src/pending.rs"] {
        assert_eq!(fixed_in(&earlier, path), None, "{path} was never a problem to fix");
    }
    assert_eq!(fixed_in(&other_pr, "src/kept.rs"), None, "another PR's finding is not this re-review's to mark");
    let prior = prior_findings(&db, PrKey { host: "github.com", owner: "octo", name: "repo", number: 7 }, &job.id).unwrap();
    assert_eq!(prior.len(), 5, "the re-review's own findings are not prior ones");
    assert_eq!(prior.iter().find(|finding| finding.path == "src/edited.rs").unwrap().body, "e", "the edited body is what was said");
}

#[test]
fn the_posted_head_is_the_commit_github_has_the_review_on() {
    let (_scratch, db) = open();
    insert(&db, &new_job(7, "h2", &["claude"]), at(0)).unwrap();
    let (job, token) = claim(&db, "runner", at(1));
    for (from, to) in [(Status::Preparing, Status::Reviewing), (Status::Reviewing, Status::Drafted), (Status::Drafted, Status::Posting)] {
        assert!(advance(&db, &job.id, &token, from, to, false).unwrap(), "{from:?} -> {to:?}");
    }
    // A retry found the earlier attempt's review, made on h1, already posted.
    let posted = PostedReview { review_id: "r".into(), url: "u".into(), commit_id: Some("h1".into()), findings: Vec::new() };
    assert!(mark_posted(&db, &job.id, &token, &posted, at(2)).unwrap());
    let pr = PrKey { host: "github.com", owner: "octo", name: "repo", number: 7 };
    assert_eq!(latest_posted_head(&db, pr).unwrap().as_deref(), Some("h1"), "never past what GitHub holds");
}

#[test]
fn history_belongs_to_the_repository_not_the_workspace() {
    let (_scratch, db) = open();
    let with = |workspace: &str, name: &str, sha: &str| NewJob { workspace_id: workspace.into(), repo_name: name.into(), ..new_job(7, sha, &["claude"]) };
    let (one, _) = insert(&db, &with("w1", "repo", "a"), at(0)).unwrap();
    let (two, _) = insert(&db, &with("w2", "Repo", "b"), at(1)).unwrap();
    // The same workspace, later pointed at another repository with its own #7.
    let (elsewhere, _) = insert(&db, &with("w1", "other", "c"), at(2)).unwrap();
    for job in [&one, &two, &elsewhere] {
        let pass = insert_pass(&db, &job.id, 0, 1, "claude", at(3)).unwrap();
        insert_findings(&db, &job.id, &pass, &[draft("src/a.rs")], at(3)).unwrap();
    }
    db.execute("UPDATE review_jobs SET status='posted', posted_head_sha=head_sha", []).unwrap();
    let repo = PrKey { host: "github.com", owner: "OCTO", name: "repo", number: 7 };
    let (current, _) = insert(&db, &with("w1", "repo", "d"), at(4)).unwrap();
    let prior: Vec<String> = prior_findings(&db, repo, &current.id).unwrap().into_iter().map(|finding| finding.job_id).collect();
    assert_eq!(prior, [one.id.clone(), two.id.clone()], "both workspaces of the repository, never the other repository");
    assert_eq!(latest_posted_head(&db, repo).unwrap().as_deref(), Some("b"));
    assert!(has_review_of_head(&db, repo, "a").unwrap() && has_review_of_head(&db, repo, "b").unwrap());
    assert!(!has_review_of_head(&db, repo, "c").unwrap(), "another repository's head");
    let other = PrKey { host: "github.com", owner: "octo", name: "other", number: 7 };
    assert!(prior_findings(&db, other, "none").unwrap().iter().all(|finding| finding.job_id == elsewhere.id));
}

#[test]
fn queue_order_does_not_decide_which_fix_marks_stand() {
    let (_scratch, db) = open();
    // The review of the later head h2 was queued first; git places h1 before
    // h2, and the h2 review names h1's finding as fixed.
    let (newer, _) = insert(&db, &new_job(7, "h2", &["claude"]), at(0)).unwrap();
    let (older, _) = insert(&db, &new_job(7, "h1", &["codex"]), at(5)).unwrap();
    let pass = insert_pass(&db, &older.id, 0, 1, "codex", at(5)).unwrap();
    insert_findings(&db, &older.id, &pass, &[draft("src/a.rs")], at(5)).unwrap();
    db.execute_batch("UPDATE review_findings SET decision='kept';").unwrap();
    let finding = findings(&db, &older.id).unwrap().remove(0).id;
    db.execute(&format!("UPDATE review_jobs SET status='posted' WHERE id='{}'", older.id), []).unwrap();
    let (job, token) = claim(&db, "runner", at(11));
    assert_eq!(job.id, newer.id);
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = begin_pass(&db, &job.id, &token, 0, "claude", at(12)).unwrap().unwrap();
    let outcome = PassOutcome { fixed: vec![finding.clone()], ..Default::default() };
    assert!(complete_pass(&db, &job.id, &token, &pass, &outcome, at(13)).unwrap());
    assert_eq!(findings(&db, &older.id).unwrap()[0].fixed_in_sha.as_deref(), Some("h2"), "git's order, not the queue's");
}

/// Run migration 93's backfill again over rows written as before it.
fn migrate_92(db: &Connection) {
    let transaction = Transaction::new_unchecked(db, TransactionBehavior::Immediate).unwrap();
    crate::store::migration_93_review_identity(&transaction).unwrap();
    transaction.commit().unwrap();
}

#[test]
fn legacy_posted_rows_give_a_base_only_when_their_commit_is_known() {
    let (_scratch, db) = open();
    let pr = PrKey { host: "github.com", owner: "octo", name: "repo", number: 7 };
    let (own, _) = insert(&db, &new_job(7, "h1", &["claude"]), at(0)).unwrap();
    // Posted before the commit was recorded, as migration 93 leaves it.
    db.execute(&format!("UPDATE review_jobs SET status='posted', finished_at='{}' WHERE id='{}'", stamp(at(1)), own.id), []).unwrap();
    assert_eq!(latest_posted_head(&db, pr).unwrap(), None, "unknown: a full review, never a guess");
    // What the migration fills in for a job that posted its own review.
    migrate_92(&db);
    assert_eq!(latest_posted_head(&db, pr).unwrap().as_deref(), Some("h1"));
    // A retry that may have reconciled another attempt's review stays unknown.
    let (retry, _) = insert(&db, &NewJob { retry_of: Some(own.id.clone()), ..new_job(7, "h2", &["claude"]) }, at(2)).unwrap();
    db.execute(&format!("UPDATE review_jobs SET status='posted', finished_at='{}' WHERE id='{}'", stamp(at(3)), retry.id), []).unwrap();
    migrate_92(&db);
    assert_eq!(latest_posted_head(&db, pr).unwrap(), None, "the unpublished h2 is never the base");
}

#[test]
fn history_and_fix_marks_never_cross_github_hosts() {
    let (_scratch, db) = open();
    let on = |host: &str, sha: &str| NewJob { repo_host: Some(host.into()), ..new_job(7, sha, &["claude"]) };
    let (public, _) = insert(&db, &on("github.com", "a"), at(0)).unwrap();
    let pass = insert_pass(&db, &public.id, 0, 1, "claude", at(0)).unwrap();
    insert_findings(&db, &public.id, &pass, &[draft("src/a.rs")], at(0)).unwrap();
    db.execute_batch("UPDATE review_jobs SET status='posted', posted_head_sha=head_sha; UPDATE review_findings SET decision='kept';").unwrap();
    let enterprise = PrKey { host: "ghe.example.com", owner: "octo", name: "repo", number: 7 };
    assert_eq!(latest_posted_head(&db, enterprise).unwrap(), None);
    assert!(prior_findings(&db, enterprise, "none").unwrap().is_empty());
    assert!(!has_review_of_head(&db, enterprise, "a").unwrap());
    // A legacy row with no host matches no repository.
    let (legacy, _) = insert(&db, &NewJob { repo_host: None, ..new_job(7, "b", &["claude"]) }, at(1)).unwrap();
    db.execute(&format!("UPDATE review_jobs SET status='posted', posted_head_sha=head_sha WHERE id='{}'", legacy.id), []).unwrap();
    let public_key = PrKey { host: "github.com", owner: "octo", name: "repo", number: 7 };
    assert_eq!(latest_posted_head(&db, public_key).unwrap().as_deref(), Some("a"));
    // An enterprise review never marks a github.com finding fixed.
    insert(&db, &on("ghe.example.com", "c"), at(2)).unwrap();
    let (job, token) = claim(&db, "runner", at(3));
    assert!(advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = begin_pass(&db, &job.id, &token, 0, "claude", at(4)).unwrap().unwrap();
    let finding = findings(&db, &public.id).unwrap().remove(0).id;
    assert!(complete_pass(&db, &job.id, &token, &pass, &PassOutcome { fixed: vec![finding], ..Default::default() }, at(5)).unwrap());
    assert_eq!(findings(&db, &public.id).unwrap()[0].fixed_in_sha, None);
}

/// A two-harness job in `reviewing` under its claim, with both passes done:
/// claude found `a.rs` (minor, short body) and `b.rs`; codex found `a.rs`
/// (major, longer body). Returns the job, its token and the two pass ids.
fn second_opinion(db: &Connection) -> (JobRow, String, String, String) {
    insert(db, &new_job(9, "sha", &["claude", "codex"]), at(0)).unwrap();
    let (job, token) = claim(db, "runner", at(1));
    assert!(advance(db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let claude = insert_pass(db, &job.id, 0, 1, "claude", at(2)).unwrap();
    let codex = insert_pass(db, &job.id, 1, 1, "codex", at(2)).unwrap();
    for pass in [&claude, &codex] {
        finish_pass(db, pass, ReviewPassStatus::Done, None, Some(&PassOutcome::default()), at(3)).unwrap();
    }
    let a = |severity, body: &str| DraftFinding { severity, body: body.into(), title: "Divides by zero on empty input".into(), ..draft("a.rs") };
    insert_findings(db, &job.id, &claude, &[a(ReviewSeverity::Minor, "Zero."), draft("b.rs")], at(4)).unwrap();
    insert_findings(db, &job.id, &codex, &[a(ReviewSeverity::Major, "`average` divides by the length, zero when empty.")], at(4)).unwrap();
    (job, token, claude, codex)
}

#[test]
fn merge_findings_updates_and_deletes_under_the_claim() {
    let (_scratch, db) = open();
    let (job, token, claude, codex) = second_opinion(&db);
    let entries = merge_entries(&db, &job.id).unwrap();
    assert_eq!(entries.len(), 3);
    let merges = crate::reviews::merge::merges(&entries);
    assert_eq!(merges.len(), 1);

    // A driver that lost the job writes nothing.
    assert!(!merge_findings(&db, &job.id, "runner/stale", &merges).unwrap());
    assert_eq!(findings(&db, &job.id).unwrap().len(), 3);

    assert!(merge_findings(&db, &job.id, &token, &merges).unwrap());
    let stored = findings(&db, &job.id).unwrap();
    assert_eq!(stored.len(), 2, "the pair is one finding");
    let top = &stored[0];
    assert_eq!((top.path.as_str(), top.agreed_by), ("a.rs", 2), "agreed findings rank first");
    assert_eq!(top.severity, ReviewSeverity::Major);
    assert_eq!(top.body, "`average` divides by the length, zero when empty.", "the longer body");
    assert_eq!(top.pass_ids, [codex.clone(), claude.clone()]);
    assert_eq!(top.pass_id, codex);
    let harnesses = finding_harnesses(&db, &job.id).unwrap();
    assert_eq!(harnesses[&top.id], ["codex", "claude"]);
    assert!(merge_entries(&db, &job.id).unwrap().iter().all(|entry| entry.id != top.id), "a merged finding is not merged again");
}

#[test]
fn pass_ids_round_trip_on_the_wire() {
    let (_scratch, db) = open();
    let (job, token, _, _) = second_opinion(&db);
    let merges = crate::reviews::merge::merges(&merge_entries(&db, &job.id).unwrap());
    merge_findings(&db, &job.id, &token, &merges).unwrap();
    let stored = findings(&db, &job.id).unwrap();
    let merged = serde_json::to_value(&stored[0]).unwrap();
    assert_eq!(merged["passIds"].as_array().map(Vec::len), Some(2));
    let single = serde_json::to_value(&stored[1]).unwrap();
    assert!(single.get("passIds").is_none(), "an unmerged finding carries no passIds: {single}");
    assert_eq!(single["agreedBy"], 1);
    let decoded: wire::ReviewFinding = serde_json::from_value(single).unwrap();
    assert!(decoded.pass_ids.is_empty());
}

#[test]
fn merge_findings_keeps_the_dropped_identity() {
    let (_scratch, db) = open();
    let (job, token, _, _) = second_opinion(&db);
    let entries = merge_entries(&db, &job.id).unwrap();
    let merges = crate::reviews::merge::merges(&entries);
    let dropped = entries.iter().find(|entry| entry.id == merges[0].drop).unwrap().finding.clone();
    assert!(merge_findings(&db, &job.id, &token, &merges).unwrap());
    assert!(finish_review(&db, &job.id, &token, "s", None, true).unwrap());
    let prior = prior_findings(&db, PrKey { host: "github.com", owner: "octo", name: "repo", number: 9 }, "later").unwrap();
    let merged = prior.iter().find(|finding| finding.id == merges[0].keep).unwrap();
    assert_eq!(merged.also_known_as, [FindingIdentity { title: dropped.title, fingerprint: dropped.fingerprint }]);
    assert!(prior.iter().filter(|finding| finding.id != merged.id).all(|finding| finding.also_known_as.is_empty()));
}

#[test]
fn activity_is_capped_and_cut_per_pass() {
    use wire::ReviewActivityKind as Kind;
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(5, "sha", &["claude", "codex"]), at(0)).unwrap();
    let first = insert_pass(&db, &job.id, 0, 1, "claude", at(1)).unwrap();
    let second = insert_pass(&db, &job.id, 1, 1, "codex", at(1)).unwrap();
    // The second pass records first: the list still goes pass by pass.
    record_activity(&db, &job.id, &second, Kind::Tool, Some("Read"), Some("a.rs"), at(2)).unwrap();
    let long = "x".repeat(MAX_ACTIVITY_TEXT + 50);
    let entry = record_activity(&db, &job.id, &first, Kind::Message, None, Some(&long), at(3)).unwrap().unwrap();
    assert_eq!(entry.seq, 1);
    assert_eq!(entry.text.as_deref().unwrap().chars().count(), MAX_ACTIVITY_TEXT + 1, "cut, with an ellipsis");
    for index in 2..MAX_ACTIVITY_PER_PASS {
        record_activity(&db, &job.id, &first, Kind::Tool, Some("Grep"), Some(&index.to_string()), at(4)).unwrap().unwrap();
    }
    let last = record_activity(&db, &job.id, &first, Kind::Tool, Some("Grep"), None, at(5)).unwrap().unwrap();
    assert_eq!((last.seq, last.title.as_deref()), (MAX_ACTIVITY_PER_PASS, Some("Activity cut")));
    assert!(record_activity(&db, &job.id, &first, Kind::Tool, Some("Grep"), None, at(6)).unwrap().is_none(), "dropped once full");

    let entries = activity(&db, &job.id).unwrap();
    assert_eq!(entries.len(), MAX_ACTIVITY_PER_PASS as usize + 1);
    assert_eq!(entries[0].pass_id, first, "slot order, not arrival order");
    assert_eq!(entries.last().unwrap().pass_id, second);
    assert!(entries.windows(2).filter(|pair| pair[0].pass_id == pair[1].pass_id).all(|pair| pair[0].seq < pair[1].seq));
    let json = serde_json::to_value(&entries[0]).unwrap();
    assert_eq!((json["kind"].as_str(), json["passId"].as_str()), (Some("message"), Some(first.as_str())));
}

#[test]
fn a_pass_records_its_model_when_it_starts() {
    let (_scratch, db) = open();
    let (job, _) = insert(&db, &new_job(6, "sha", &["claude"]), at(0)).unwrap();
    let pass = insert_pass(&db, &job.id, 0, 1, "claude", at(1)).unwrap();
    set_pass_model(&db, &pass, Some("claude-sonnet-4-6"), Some("medium")).unwrap();
    let stored = &passes(&db, &job.id).unwrap()[0];
    assert_eq!((stored.model.as_deref(), stored.effort.as_deref()), (Some("claude-sonnet-4-6"), Some("medium")));
    set_pass_model(&db, &pass, None, None).unwrap();
    assert_eq!(passes(&db, &job.id).unwrap()[0].model.as_deref(), Some("claude-sonnet-4-6"), "nothing known keeps what was");
}
