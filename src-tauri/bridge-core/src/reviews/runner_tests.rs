use super::*;
use std::collections::VecDeque;
use std::time::Instant;

use bridge_protocol::messages::{self as wire, ReviewSeverity, ReviewSide};
use rusqlite::params;

use crate::reviews::steps::{post_marker, DraftFinding, PassOutcome, PostedReview};
use crate::reviews::PrHead;

/// What a scripted post does.
enum PostPlan {
    /// Posts and answers with the review.
    Post,
    /// Posts, then times out before answering: GitHub has it, Bridge does not.
    PostThenTimeOut,
    Fail(StepError),
}

/// Steps that follow a script and record every call. Reviews block while the
/// gate is closed, stopping only when cancelled.
#[derive(Default)]
struct Script {
    prepare: Mutex<VecDeque<Result<Prepared, StepError>>>,
    review: Mutex<HashMap<String, VecDeque<Result<PassOutcome, StepError>>>>,
    post: Mutex<VecDeque<PostPlan>>,
    gate_closed: AtomicBool,
    /// Reviews ignore cancellation while waiting at the gate, like a harness
    /// that finishes its turn after Bridge stopped listening.
    stubborn: AtomicBool,
    panic_on_prepare: AtomicBool,
    /// The review checkout budget is full.
    full: AtomicBool,
    /// Review bodies on the simulated pull request.
    github: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
}

impl Script {
    fn call(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self, prefix: &str) -> Vec<String> {
        self.calls.lock().unwrap().iter().filter(|call| call.starts_with(prefix)).cloned().collect()
    }

    fn close_gate(&self) {
        self.gate_closed.store(true, Ordering::SeqCst);
    }

    fn open_gate(&self) {
        self.gate_closed.store(false, Ordering::SeqCst);
    }

    fn plan_review(&self, harness: &str, results: Vec<Result<PassOutcome, StepError>>) {
        self.review.lock().unwrap().entry(harness.into()).or_default().extend(results);
    }
}

fn outcome() -> PassOutcome {
    PassOutcome {
        summary: "Looks mostly fine.".into(),
        findings: vec![DraftFinding {
            path: "src/lib.rs".into(),
            line: 12,
            start_line: None,
            side: ReviewSide::Right,
            severity: ReviewSeverity::Major,
            title: "Unchecked unwrap".into(),
            body: "This panics on empty input.".into(),
            confidence: 0.8,
            fingerprint: "fp-1".into(),
            outside_diff: false,
        }],
        session_id: Some("hidden".into()),
        ..PassOutcome::default()
    }
}

impl ReviewSteps for Script {
    fn prepare(&self, job: &JobRow, _cancel: &CancelToken) -> Result<Prepared, StepError> {
        self.call(format!("prepare:{}", job.id));
        if self.panic_on_prepare.load(Ordering::SeqCst) {
            panic!("worktree exploded");
        }
        self.prepare
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Ok(Prepared { head_sha: job.head_sha.clone(), ..Prepared::default() }))
    }

    fn review(&self, request: PassRequest<'_>, cancel: &CancelToken) -> Result<PassOutcome, StepError> {
        self.call(format!("review:{}:{}", request.job.id, request.harness));
        while self.gate_closed.load(Ordering::SeqCst) {
            if self.stubborn.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            if cancel.sleep(Duration::from_millis(5)) {
                self.call(format!("review-stopped:{}", request.job.id));
                return Err(StepError::Cancelled);
            }
        }
        let planned = self.review.lock().unwrap().get_mut(request.harness).and_then(VecDeque::pop_front);
        planned.unwrap_or_else(|| Ok(outcome()))
    }

    fn find_posted(&self, job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        self.call(format!("find:{}", job.id));
        let marker = post_marker(&job.publication_key);
        let github = self.github.lock().unwrap();
        Ok(github.iter().position(|body| body.contains(&marker)).map(|index| PostedReview {
            review_id: format!("review-{index}"),
            url: format!("https://github.com/octo/repo/pull/{}#pullrequestreview-{index}", job.number),
            findings: Vec::new(),
        }))
    }

    fn post(&self, job: &JobRow, _cancel: &CancelToken) -> Result<PostedReview, StepError> {
        self.call(format!("post:{}", job.id));
        let plan = self.post.lock().unwrap().pop_front().unwrap_or(PostPlan::Post);
        let publish = |github: &mut Vec<String>| {
            github.push(format!("Review\n\n{}", post_marker(&job.publication_key)));
            github.len() - 1
        };
        match plan {
            PostPlan::Post => {
                let index = publish(&mut self.github.lock().unwrap());
                Ok(PostedReview {
                    review_id: format!("review-{index}"),
                    url: format!("https://github.com/octo/repo/pull/{}#pullrequestreview-{index}", job.number),
                    findings: Vec::new(),
                })
            }
            PostPlan::PostThenTimeOut => {
                publish(&mut self.github.lock().unwrap());
                Err(StepError::Transient("gh timed out after 30s".into()))
            }
            PostPlan::Fail(error) => Err(error),
        }
    }

    fn cleanup(&self, job_id: &str) {
        self.call(format!("cleanup:{job_id}"));
    }

    fn has_capacity(&self) -> bool {
        !self.full.load(Ordering::SeqCst)
    }

    fn reap(&self) {
        self.call("reap".into());
    }
}

fn fast() -> RunnerConfig {
    RunnerConfig {
        renew_every: Duration::from_millis(30),
        retry_backoff: Duration::from_millis(1),
        step_retries: 3,
        poll: Duration::from_millis(10),
    }
}

fn fixture() -> (tempfile::TempDir, Arc<BridgeCore>, Arc<Script>) {
    let scratch = tempfile::tempdir().unwrap();
    let core = Arc::new(BridgeCore::for_tests(scratch.path()));
    {
        let db = core.db.lock().unwrap();
        db.execute(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p1','p1',?1,'now')",
            params![scratch.path().to_string_lossy()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w1','p1','Kyoto','w1','main',?1,'idle','now')",
            params![scratch.path().to_string_lossy()],
        )
        .unwrap();
    }
    let script = Arc::new(Script::default());
    core.reviews.set_steps(script.clone());
    (scratch, core, script)
}

fn settings(core: &Arc<BridgeCore>, change: impl FnOnce(&mut wire::ReviewJobSettings)) {
    let db = core.db.lock().unwrap();
    let mut settings = crate::reviews::settings::load(&db).unwrap();
    change(&mut settings);
    crate::reviews::settings::save(&db, &settings).unwrap();
}

fn queue(core: &Arc<BridgeCore>, number: u64, harnesses: &[&str]) -> String {
    let params = wire::StartReviewParams {
        workspace_id: "w1".into(),
        number,
        harnesses: Some(harnesses.iter().map(|harness| harness.to_string()).collect()),
        trigger: None,
    };
    let head = PrHead { repo_owner: "octo".into(), repo_name: "repo".into(), head_sha: format!("sha-{number}"), base_sha: None };
    crate::reviews::start_with(core, &params, head).unwrap().id
}

fn job(core: &Arc<BridgeCore>, id: &str) -> JobRow {
    store::require_job(&core.db.lock().unwrap(), id).unwrap()
}

fn wait_for(core: &Arc<BridgeCore>, id: &str, status: Status) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while job(core, id).status != status {
        assert!(Instant::now() < deadline, "job {id} never reached {}: {:?}", status.as_str(), job(core, id).status);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Tick until nothing is claimable or driven.
fn drain(runner: &Runner) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let handles = runner.tick().unwrap();
        let idle = handles.is_empty() && runner.active() == 0;
        for handle in handles {
            handle.join().unwrap();
        }
        if idle {
            return;
        }
        assert!(Instant::now() < deadline, "the runner never went idle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn two_jobs_run_at_once_and_a_third_waits_in_queued() {
    let (_scratch, core, script) = fixture();
    script.close_gate();
    let ids: Vec<String> = (1..=3).map(|number| queue(&core, number, &["claude"])).collect();
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    let first = runner.tick().unwrap();
    assert_eq!(first.len(), 2, "the default pool has two slots");
    wait_for(&core, &ids[0], Status::Reviewing);
    wait_for(&core, &ids[1], Status::Reviewing);
    assert!(runner.tick().unwrap().is_empty(), "no third claim while both slots are busy");
    assert_eq!(job(&core, &ids[2]).status, Status::Queued);
    script.open_gate();
    for handle in first {
        handle.join().unwrap();
    }
    drain(&runner);
    for id in &ids {
        assert_eq!(job(&core, id).status, Status::Drafted);
    }
}

#[test]
fn max_concurrent_reviews_sizes_the_pool() {
    let (_scratch, core, script) = fixture();
    script.close_gate();
    settings(&core, |settings| settings.max_concurrent_reviews = 1);
    for number in 1..=4 {
        queue(&core, number, &["claude"]);
    }
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    let mut handles = runner.tick().unwrap();
    assert_eq!(handles.len(), 1);
    settings(&core, |settings| settings.max_concurrent_reviews = 3);
    handles.extend(runner.tick().unwrap());
    assert_eq!(handles.len(), 3, "raising the setting widens the pool on the next tick");
    assert_eq!(runner.active(), 3);
    script.open_gate();
    for handle in handles {
        handle.join().unwrap();
    }
    drain(&runner);
    assert_eq!(runner.active(), 0);
}

#[test]
fn cancel_during_reviewing_stops_the_step_and_cleans_up_within_five_seconds() {
    let (_scratch, core, script) = fixture();
    script.close_gate();
    let id = queue(&core, 1, &["claude"]);
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    let handles = runner.tick().unwrap();
    wait_for(&core, &id, Status::Reviewing);
    let started = Instant::now();
    let cancelled = crate::reviews::cancel(&core, &id).unwrap();
    assert_eq!(cancelled.status, Status::Cancelled);
    for handle in handles {
        handle.join().unwrap();
    }
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}", started.elapsed());
    assert_eq!(script.calls(&format!("review-stopped:{id}")).len(), 1, "the running step saw the cancel");
    assert!(!script.calls(&format!("cleanup:{id}")).is_empty(), "the job's worktree is cleaned up");
    let detail = crate::reviews::get(&core, &id).unwrap();
    assert_eq!(detail.job.status, Status::Cancelled);
    assert_eq!(detail.passes[0].status, ReviewPassStatus::Cancelled);
    assert_eq!(runner.active(), 0);
}

#[test]
fn a_harness_failure_falls_back_to_the_next_configured_harness() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.fallback_harnesses = vec!["codex".into(), "opencode".into()]);
    script.plan_review("claude", vec![Err(StepError::Harness("Claude is signed out".into()))]);
    let id = queue(&core, 1, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    let detail = crate::reviews::get(&core, &id).unwrap();
    assert_eq!(detail.job.status, Status::Drafted);
    assert_eq!(detail.job.harnesses, ["codex"], "the job reports the harness that actually ran");
    assert_eq!(detail.passes.len(), 2);
    assert_eq!(
        (detail.passes[0].harness.as_str(), detail.passes[0].status, detail.passes[0].error.as_deref()),
        ("claude", ReviewPassStatus::Failed, Some("Claude is signed out"))
    );
    assert_eq!((detail.passes[1].harness.as_str(), detail.passes[1].status), ("codex", ReviewPassStatus::Done));
    assert_eq!(detail.findings.len(), 1);
    assert_eq!(detail.findings[0].pass_id, detail.passes[1].id);
    assert!(script.calls("review:").iter().all(|call| !call.ends_with(":opencode")), "the first fallback that works is enough");
}

#[test]
fn a_job_fails_when_every_fallback_harness_fails() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.fallback_harnesses = vec!["codex".into()]);
    script.plan_review("claude", vec![Err(StepError::Harness("usage limit".into()))]);
    script.plan_review("codex", vec![Err(StepError::Harness("crashed".into()))]);
    let id = queue(&core, 1, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    let failed = job(&core, &id);
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(failed.error.as_deref(), Some("codex could not review: crashed"));
    assert!(!script.calls(&format!("cleanup:{id}")).is_empty());
}

#[test]
fn transient_errors_retry_three_times_then_fail() {
    let (_scratch, core, script) = fixture();
    script.prepare.lock().unwrap().extend((0..4).map(|_| Err(StepError::Transient("git fetch failed".into()))));
    let id = queue(&core, 1, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    assert_eq!(script.calls("prepare:").len(), 4, "one attempt and three retries");
    let failed = job(&core, &id);
    assert_eq!((failed.status, failed.error.as_deref()), (Status::Failed, Some("git fetch failed")));
}

#[test]
fn a_transient_error_then_success_completes() {
    let (_scratch, core, script) = fixture();
    script.prepare.lock().unwrap().extend((0..2).map(|_| Err(StepError::Transient("gh: connection reset".into()))));
    script.plan_review("claude", vec![Err(StepError::Transient("rate limited".into()))]);
    let id = queue(&core, 1, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    assert_eq!(script.calls("prepare:").len(), 3);
    assert_eq!(script.calls("review:").len(), 2);
    assert_eq!(job(&core, &id).status, Status::Drafted);
}

#[test]
fn draft_mode_stops_at_drafted_and_releases_the_lease() {
    let (_scratch, core, script) = fixture();
    let id = queue(&core, 1, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    let drafted = job(&core, &id);
    assert_eq!(drafted.status, Status::Drafted);
    assert_eq!((drafted.lease_owner, drafted.lease_expires_at), (None, None));
    assert_eq!(drafted.summary.as_deref(), Some("Looks mostly fine."));
    assert!(script.calls("post:").is_empty() && script.calls("find:").is_empty(), "nothing reaches GitHub");
    assert!(script.github.lock().unwrap().is_empty());
    let detail = crate::reviews::get(&core, &id).unwrap();
    assert_eq!(detail.job.finding_counts.major, 1);
    assert_eq!(detail.findings[0].decision, wire::ReviewFindingDecision::Pending);
}

#[test]
fn auto_mode_goes_from_reviewing_to_posted() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.post_mode = wire::ReviewPostMode::Auto);
    let id = queue(&core, 9, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    let posted = job(&core, &id);
    assert_eq!(posted.status, Status::Posted);
    assert_eq!(posted.posted_review_id.as_deref(), Some("review-0"));
    assert_eq!(posted.posted_review_url.as_deref(), Some("https://github.com/octo/repo/pull/9#pullrequestreview-0"));
    assert_eq!(script.github.lock().unwrap().len(), 1);
}

#[test]
fn a_post_retried_after_a_timeout_never_creates_a_second_review() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.post_mode = wire::ReviewPostMode::Auto);
    script.post.lock().unwrap().push_back(PostPlan::PostThenTimeOut);
    let id = queue(&core, 3, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    assert_eq!(script.calls("post:").len(), 1, "the retry found the marker instead of posting again");
    assert_eq!(script.calls("find:").len(), 2, "the marker is checked before every attempt");
    assert_eq!(script.github.lock().unwrap().len(), 1, "exactly one review on the PR");
    let posted = job(&core, &id);
    assert_eq!((posted.status, posted.posted_review_id.as_deref()), (Status::Posted, Some("review-0")));
}

#[test]
fn a_failed_post_in_draft_mode_returns_to_the_draft() {
    let (_scratch, core, script) = fixture();
    let id = queue(&core, 4, &["claude"]);
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    drain(&runner);
    // What `reviews/post` will do: hand the draft to the runner to post.
    core.db.lock().unwrap().execute("UPDATE review_jobs SET status='posting' WHERE id=?1", params![id]).unwrap();
    script.post.lock().unwrap().push_back(PostPlan::Fail(StepError::Fatal("gh: HTTP 422 line must be part of the diff".into())));
    drain(&runner);
    let back = job(&core, &id);
    assert_eq!(back.status, Status::Drafted, "the person can post again");
    assert_eq!(back.error.as_deref(), Some("gh: HTTP 422 line must be part of the diff"));
    assert_eq!(back.lease_owner, None);
}

#[test]
fn a_runner_that_stops_renewing_loses_the_job_within_sixty_seconds() {
    let (_scratch, core, script) = fixture();
    let id = queue(&core, 1, &["claude"]);
    let start = Utc::now();
    // A runner claims the job and then dies: it never renews.
    store::claim_next(&core.db.lock().unwrap(), "dead-runner", start).unwrap().unwrap();
    let now = Arc::new(Mutex::new(start + chrono::Duration::seconds(44)));
    let clock_now = now.clone();
    let survivor = Runner::with(core.clone(), fast(), Arc::new(move || *clock_now.lock().unwrap()));
    assert!(survivor.tick().unwrap().is_empty(), "a live lease is respected");
    *now.lock().unwrap() = start + chrono::Duration::seconds(store::LEASE_SECONDS + 5);
    assert!(store::LEASE_SECONDS + 5 <= 60);
    for handle in survivor.tick().unwrap() {
        handle.join().unwrap();
    }
    let taken = job(&core, &id);
    assert_eq!(taken.status, Status::Drafted, "the surviving runner finished the job");
    assert_eq!(taken.attempts, 2);
    assert_eq!(script.calls("prepare:").len(), 1);
}

#[test]
fn restart_interrupts_the_job_and_retry_completes() {
    let (_scratch, core, script) = fixture();
    script.close_gate();
    let id = queue(&core, 1, &["claude"]);
    let before = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    let stranded = before.tick().unwrap();
    wait_for(&core, &id, Status::Reviewing);

    // A new host boots against the same store while the old run is mid-review.
    let after = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    assert_eq!(after.recover().unwrap(), [id.clone()]);
    let interrupted = job(&core, &id);
    assert_eq!((interrupted.status, interrupted.error.as_deref()), (Status::Interrupted, Some(store::RESTARTED)));
    assert!(!script.calls(&format!("cleanup:{id}")).is_empty(), "recovery reaps the job's worktree");

    script.open_gate();
    for handle in stranded {
        handle.join().unwrap();
    }
    assert_eq!(job(&core, &id).status, Status::Interrupted, "the stranded run writes nothing over it");

    let retried = crate::reviews::retry(&core, &id).unwrap();
    assert_eq!(retried.retry_of.as_deref(), Some(id.as_str()));
    drain(&after);
    assert_eq!(job(&core, &retried.id).status, Status::Drafted);
}

#[test]
fn every_transition_publishes_job_updated() {
    let (_scratch, core, _script) = fixture();
    settings(&core, |settings| settings.post_mode = wire::ReviewPostMode::Auto);
    let mut receiver = core.events.subscribe();
    let id = queue(&core, 1, &["claude"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    let mut statuses = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        if let crate::events::CoreEvent::ReviewJobUpdated { job } = event {
            assert_eq!(job.id, id);
            if statuses.last() != Some(&job.status) {
                statuses.push(job.status);
            }
        }
    }
    assert_eq!(
        statuses,
        [Status::Queued, Status::Preparing, Status::Reviewing, Status::Drafted, Status::Posting, Status::Posted]
    );
}

#[test]
fn cleanup_runs_on_every_terminal_transition() {
    let (_scratch, core, script) = fixture();
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));

    settings(&core, |settings| settings.post_mode = wire::ReviewPostMode::Auto);
    let posted = queue(&core, 1, &["claude"]);
    drain(&runner);
    assert_eq!(job(&core, &posted).status, Status::Posted);

    script.prepare.lock().unwrap().push_back(Err(StepError::Fatal("fork is gone".into())));
    let failed = queue(&core, 2, &["claude"]);
    drain(&runner);
    assert_eq!(job(&core, &failed).status, Status::Failed);

    let cancelled = queue(&core, 3, &["claude"]);
    crate::reviews::cancel(&core, &cancelled).unwrap();

    script.close_gate();
    let interrupted = queue(&core, 4, &["claude"]);
    let stranded = runner.tick().unwrap();
    wait_for(&core, &interrupted, Status::Reviewing);
    Runner::with(core.clone(), fast(), Arc::new(Utc::now)).recover().unwrap();
    script.open_gate();
    for handle in stranded {
        handle.join().unwrap();
    }

    for id in [&posted, &failed, &cancelled, &interrupted] {
        assert!(job(&core, id).status.is_terminal());
        assert!(!script.calls(&format!("cleanup:{id}")).is_empty(), "{:?} was not cleaned up", job(&core, id).status);
    }
}

#[test]
fn a_panicking_step_fails_the_job_and_frees_its_slot() {
    let (_scratch, core, script) = fixture();
    script.panic_on_prepare.store(true, Ordering::SeqCst);
    let id = queue(&core, 1, &["claude"]);
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    drain(&runner);
    let failed = job(&core, &id);
    assert_eq!(failed.status, Status::Failed);
    assert!(failed.error.as_deref().unwrap_or_default().contains("worktree exploded"), "{:?}", failed.error);
    assert_eq!(runner.active(), 0);
    assert!(!script.calls(&format!("cleanup:{id}")).is_empty());
}

#[test]
fn starting_a_review_creates_no_session() {
    let (_scratch, core, _script) = fixture();
    let sessions = |core: &Arc<BridgeCore>| -> i64 {
        core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0)).unwrap()
    };
    let before = sessions(&core);
    let id = queue(&core, 1, &["claude", "codex"]);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    assert_eq!(job(&core, &id).status, Status::Drafted);
    assert_eq!(sessions(&core), before, "a review is not a chat and creates no session row");
    let detail = crate::reviews::get(&core, &id).unwrap();
    assert_eq!(detail.passes.len(), 2, "a second opinion is two passes on one job");
}

#[test]
fn start_resolves_harnesses_from_the_request_then_the_settings() {
    let (_scratch, core, _script) = fixture();
    let head = || PrHead { repo_owner: "octo".into(), repo_name: "repo".into(), head_sha: "abc".into(), base_sha: None };
    let params = |harnesses: Option<Vec<&str>>| wire::StartReviewParams {
        workspace_id: "w1".into(),
        number: 5,
        harnesses: harnesses.map(|list| list.into_iter().map(String::from).collect()),
        trigger: Some(wire::ReviewTrigger::Push),
    };
    settings(&core, |settings| settings.default_harnesses = vec!["opencode".into()]);
    let defaulted = crate::reviews::start_with(&core, &params(None), head()).unwrap();
    assert_eq!(defaulted.harnesses, ["opencode"]);
    assert_eq!(defaulted.trigger, wire::ReviewTrigger::Push);
    let aliased = crate::reviews::start_with(&core, &params(Some(vec!["claude-code"])), head()).unwrap();
    assert_eq!(aliased.harnesses, ["claude"]);
    for (refused, reason) in [
        (vec![], "Pick a harness"),
        (vec!["bugbot"], "Cursor Bugbot"),
        (vec!["shell"], "Unknown reviewer harness `shell`"),
        (vec!["claude", "codex", "opencode"], "at most 2"),
    ] {
        let error = crate::reviews::start_with(&core, &params(Some(refused.clone())), head()).unwrap_err().to_string();
        assert!(error.contains(reason), "{refused:?}: {error}");
    }
    let unknown_workspace = wire::StartReviewParams { workspace_id: "nope".into(), ..params(None) };
    assert!(crate::reviews::start_with(&core, &unknown_workspace, head()).is_err());
}

#[test]
fn a_superseded_driver_cannot_store_its_pass_or_clean_up_the_replacement() {
    let (_scratch, core, script) = fixture();
    script.close_gate();
    script.stubborn.store(true, Ordering::SeqCst);
    let id = queue(&core, 1, &["claude"]);
    let start = Utc::now();

    // The first driver claims the job and stalls in its review past the lease.
    let first = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    let stalled = first.tick().unwrap();
    wait_for(&core, &id, Status::Reviewing);

    // Another runner, 50 s later by its clock, reclaims and claims the job.
    let later = start + chrono::Duration::seconds(store::LEASE_SECONDS + 5);
    let second = Runner::with(core.clone(), fast(), Arc::new(move || later));
    let replacement = second.tick().unwrap();
    assert_eq!(replacement.len(), 1, "the expired job is claimed again");
    let deadline = Instant::now() + Duration::from_secs(5);
    while script.calls("review:").len() < 2 {
        assert!(Instant::now() < deadline, "the replacement never started its review");
        std::thread::sleep(Duration::from_millis(5));
    }

    // Both reviews finish. Only the replacement's may count.
    script.open_gate();
    for handle in stalled.into_iter().chain(replacement) {
        handle.join().unwrap();
    }
    let detail = crate::reviews::get(&core, &id).unwrap();
    assert_eq!(detail.job.status, Status::Drafted);
    assert_eq!(detail.findings.len(), 1, "one requested pass, one finding: the stale result was not stored");
    assert_eq!(detail.job.finding_counts.total, 1);
    let done: Vec<_> = detail.passes.iter().filter(|pass| pass.status == ReviewPassStatus::Done).collect();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].attempt, 2, "the stored pass is the replacement's");
    assert_eq!(detail.findings[0].pass_id, done[0].id);
    assert_eq!(
        script.calls(&format!("cleanup:{id}")).len(),
        1,
        "only the claim that drafted the job cleans up; the stale driver removed nothing"
    );
    assert_eq!(first.active() + second.active(), 0);
}

#[test]
fn a_retry_finds_the_review_its_interrupted_original_posted() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.post_mode = wire::ReviewPostMode::Auto);
    let id = queue(&core, 1, &["claude"]);
    // GitHub accepted the post, then Bridge stopped before recording it.
    let original = {
        let db = core.db.lock().unwrap();
        db.execute("UPDATE review_jobs SET status='posting', lease_owner='gone/1', lease_expires_at='2999-01-01T00:00:00.000Z' WHERE id=?1", params![id]).unwrap();
        store::require_job(&db, &id).unwrap()
    };
    script.github.lock().unwrap().push(format!("Earlier review\n\n{}", post_marker(&original.publication_key)));

    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    assert_eq!(runner.recover().unwrap(), [id.clone()]);
    let retried = crate::reviews::retry(&core, &id).unwrap();
    drain(&runner);

    let posted = job(&core, &retried.id);
    assert_eq!(posted.publication_key, original.publication_key, "a retry carries its original's publication key");
    assert_eq!(posted.status, Status::Posted);
    assert_eq!(posted.posted_review_id.as_deref(), Some("review-0"), "it recorded the review already on GitHub");
    assert!(script.calls("post:").is_empty(), "nothing was posted again");
    assert_eq!(script.github.lock().unwrap().len(), 1, "one review on the PR");
}

#[test]
fn a_job_failed_by_the_final_reclaim_is_cleaned_up() {
    let (_scratch, core, script) = fixture();
    let id = queue(&core, 1, &["claude"]);
    let start = Utc::now();
    {
        let db = core.db.lock().unwrap();
        store::claim_next(&db, "dead-runner", start).unwrap().unwrap();
        db.execute("UPDATE review_jobs SET attempts=?2 WHERE id=?1", params![id, store::MAX_ATTEMPTS]).unwrap();
    }
    let later = start + chrono::Duration::seconds(store::LEASE_SECONDS + 5);
    let survivor = Runner::with(core.clone(), fast(), Arc::new(move || later));
    assert!(survivor.tick().unwrap().is_empty());
    let failed = job(&core, &id);
    assert_eq!((failed.status, failed.error.as_deref()), (Status::Failed, Some(store::STOPPED_RESPONDING)));
    assert_eq!(script.calls(&format!("cleanup:{id}")).len(), 1, "no driver is left, so the tick cleans up");
}

#[test]
fn a_moved_head_updates_the_dedupe_key() {
    let (_scratch, core, script) = fixture();
    let id = queue(&core, 1, &["claude"]);
    script.prepare.lock().unwrap().push_back(Ok(Prepared { head_sha: "sha-moved".into(), ..Prepared::default() }));
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    assert_eq!(job(&core, &id).head_sha, "sha-moved");
    let params = wire::StartReviewParams { workspace_id: "w1".into(), number: 1, harnesses: Some(vec!["claude".into()]), trigger: None };
    let head = PrHead { repo_owner: "octo".into(), repo_name: "repo".into(), head_sha: "sha-moved".into(), base_sha: None };
    let again = crate::reviews::start_with(&core, &params, head).unwrap();
    assert_eq!(again.id, id, "starting a review of the new head finds the job that already reviews it");
}

#[test]
fn a_moved_head_already_under_review_supersedes_the_job() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.max_concurrent_reviews = 1);
    let stale = queue(&core, 1, &["claude"]);
    let current = {
        let params = wire::StartReviewParams { workspace_id: "w1".into(), number: 1, harnesses: Some(vec!["claude".into()]), trigger: None };
        let head = PrHead { repo_owner: "octo".into(), repo_name: "repo".into(), head_sha: "sha-2".into(), base_sha: None };
        crate::reviews::start_with(&core, &params, head).unwrap().id
    };
    // The older job prepares and finds the PR at the head the newer one reviews.
    script.prepare.lock().unwrap().push_back(Ok(Prepared { head_sha: "sha-2".into(), ..Prepared::default() }));
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    let retired = job(&core, &stale);
    assert_eq!(retired.status, Status::Cancelled);
    assert!(retired.error.as_deref().unwrap_or_default().contains(&current), "{:?}", retired.error);
    assert_eq!(retired.head_sha, "sha-1", "the retired job keeps the head it was started for");
    assert!(!script.calls(&format!("cleanup:{stale}")).is_empty());
    assert!(script.calls(&format!("review:{stale}")).is_empty(), "the retired job spent no model run");
    assert_eq!(job(&core, &current).status, Status::Drafted);
}

#[test]
fn a_full_worktree_budget_holds_queued_jobs_until_a_slot_frees() {
    let (_scratch, core, script) = fixture();
    script.full.store(true, Ordering::SeqCst);
    let id = queue(&core, 1, &["claude"]);
    let runner = Runner::with(core.clone(), fast(), Arc::new(Utc::now));
    assert!(runner.tick().unwrap().is_empty(), "no claim while every checkout is in use");
    assert_eq!(job(&core, &id).status, Status::Queued);
    assert_eq!(job(&core, &id).attempts, 0, "holding a job back does not spend an attempt");
    script.full.store(false, Ordering::SeqCst);
    drain(&runner);
    assert_eq!(job(&core, &id).status, Status::Drafted);
}

#[test]
fn posting_jobs_are_claimed_even_when_the_budget_is_full() {
    let (_scratch, core, script) = fixture();
    settings(&core, |settings| settings.post_mode = wire::ReviewPostMode::Auto);
    let id = queue(&core, 1, &["claude"]);
    let waiting = queue(&core, 2, &["claude"]);
    // A post nobody holds, as `reviews/post` or a reclaim leaves it.
    core.db.lock().unwrap().execute("UPDATE review_jobs SET status='posting' WHERE id=?1", params![id]).unwrap();
    script.full.store(true, Ordering::SeqCst);
    drain(&Runner::with(core.clone(), fast(), Arc::new(Utc::now)));
    assert_eq!(job(&core, &id).status, Status::Posted, "posting needs no checkout");
    assert_eq!(job(&core, &waiting).status, Status::Queued);
}

#[test]
fn recovery_reaps_review_worktrees() {
    let (_scratch, core, script) = fixture();
    Runner::with(core.clone(), fast(), Arc::new(Utc::now)).recover().unwrap();
    assert_eq!(script.calls("reap"), ["reap"]);
}
