//! Compiled only under #[cfg(test)] from prepare.rs.
//!
//! Each fixture is a real repository whose `origin` is a local bare
//! repository standing in for GitHub: `main` plus pull requests pushed only to
//! `refs/pull/<n>/head`. `gh` is the shared fake in `testing/fixtures/github`.

use super::*;
use std::collections::VecDeque;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use bridge_protocol::messages::{self as wire, ReviewJobStatus as Status};
use rusqlite::params;

use crate::reviews::runner::{Runner, RunnerConfig};
use crate::reviews::steps::{post_marker, PassOutcome};

fn git(cwd: &Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(cwd).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn identity(repo: &Path) {
    git(repo, &["config", "user.email", "t@example.invalid"]);
    git(repo, &["config", "user.name", "T"]);
    git(repo, &["config", "commit.gpgsign", "false"]);
}

fn commit(repo: &Path, message: &str) -> String {
    git(repo, &["add", "-A"]);
    git(repo, &["commit", "-q", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

struct Fixture {
    _dir: tempfile::TempDir,
    gh: tempfile::TempDir,
    root: PathBuf,
    local: PathBuf,
    remote: PathBuf,
    core: Arc<BridgeCore>,
    /// The head of `refs/pull/7/head`.
    head: String,
}

impl Fixture {
    fn layout(&self) -> Layout {
        Layout::for_core(&self.core)
    }

    fn checkouts(&self) -> Vec<PathBuf> {
        subdirectories(&self.layout().reviews)
    }

    fn registered(&self) -> Vec<PathBuf> {
        git::list_worktrees(&self.local).unwrap().into_iter().map(|entry| entry.path).filter(|path| *path != self.local).collect()
    }

    fn assert_clean(&self) {
        assert!(self.checkouts().is_empty(), "checkouts left: {:?}", self.checkouts());
        assert!(subdirectories(&self.layout().jobs).is_empty(), "job folders left");
        assert!(self.registered().is_empty(), "git still lists: {:?}", self.registered());
        let refs = git(&self.local, &["for-each-ref", "--format=%(refname)", "refs/bridge/reviews"]);
        assert_eq!(refs, "", "fetched refs left");
    }

    fn invocations(&self) -> String {
        std::fs::read_to_string(self.gh.path().join("invocations.log")).unwrap_or_default()
    }
}

/// `gh` answering from the shared fixtures, with the repository resolved
/// through its default because `origin` is a local path.
fn fake_gh() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/github").canonicalize().unwrap();
    std::fs::write(directory.path().join("fixture-root"), fixtures.to_str().unwrap()).unwrap();
    std::fs::write(directory.path().join("auth-exit"), "0").unwrap();
    std::fs::write(directory.path().join("default-repository"), "fixture/project").unwrap();
    std::os::unix::fs::symlink(fixtures.join("fake-gh.sh"), directory.path().join("gh")).unwrap();
    directory
}

/// A repository with every kind of change in pull request #7: a modified
/// file, an added one, a rename, a deletion and a binary change. `main`
/// moves on after the PR branched, so the merge base is not `main`'s tip.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let remote = root.join("github.git");
    let local = root.join("local");
    std::fs::create_dir(&local).unwrap();
    git(&local, &["init", "-q", "-b", "main"]);
    identity(&local);
    std::fs::write(local.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(local.join("old.txt"), "moving\nfile\ncontents\n").unwrap();
    std::fs::write(local.join("gone.txt"), "bye\n").unwrap();
    std::fs::write(local.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
    std::fs::write(local.join("zz.txt"), "last\n").unwrap();
    commit(&local, "base");
    // Named, so the suite does not depend on init.defaultBranch.
    git(&root, &["init", "-q", "--bare", "-b", "main", remote.to_str().unwrap()]);
    git(&local, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&local, &["push", "-q", "origin", "main"]);

    git(&local, &["checkout", "-q", "-b", "pr"]);
    std::fs::write(local.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(local.join("new.txt"), "only in the PR\n").unwrap();
    std::fs::rename(local.join("old.txt"), local.join("renamed.txt")).unwrap();
    std::fs::remove_file(local.join("gone.txt")).unwrap();
    std::fs::write(local.join("bin.dat"), [9u8, 0, 8, 0, 7]).unwrap();
    let head = commit(&local, "the PR");
    git(&local, &["push", "-q", "origin", "HEAD:refs/pull/7/head"]);
    git(&local, &["checkout", "-q", "main"]);
    git(&local, &["branch", "-q", "-D", "pr"]);
    std::fs::write(local.join("later.txt"), "main moved on\n").unwrap();
    commit(&local, "later on main");
    git(&local, &["push", "-q", "origin", "main"]);

    let gh = fake_gh();
    let mut core = BridgeCore::for_tests(&root.join("data"));
    core.github_surface = crate::github_surface::GithubSurface::discover_on_path(gh.path());
    let core = Arc::new(core);
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')", params![local.to_string_lossy()])
            .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Oslo','Demo','main',?1,'ready','now')",
            params![local.to_string_lossy()],
        )
        .unwrap();
    }
    Fixture { _dir: dir, gh, root, local, remote, core, head }
}

/// Queue a job for PR `number` at `head` and claim it, as the runner would.
fn claimed(fixture: &Fixture, number: u64, head: &str) -> JobRow {
    claimed_with(fixture, number, head, "claude")
}

fn claimed_with(fixture: &Fixture, number: u64, head: &str, harness: &str) -> JobRow {
    let db = fixture.core.db.lock().unwrap();
    store::insert(
        &db,
        &store::NewJob {
            workspace_id: "w".into(),
            repo_owner: "fixture".into(),
            repo_name: "project".into(),
            number,
            head_sha: head.into(),
            base_sha: None,
            incremental_from_sha: None,
            trigger: Default::default(),
            post_mode: Default::default(),
            harnesses: vec![harness.into()],
            retry_of: None,
            publication_key: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    store::claim_next(&db, "test", chrono::Utc::now()).unwrap().expect("a claim")
}

/// The runner that held `job` died: its lease expires and the job is claimed
/// again, as a new attempt.
fn reclaimed(fixture: &Fixture, job: &JobRow) -> JobRow {
    let db = fixture.core.db.lock().unwrap();
    let later = chrono::Utc::now() + chrono::Duration::seconds(store::LEASE_SECONDS + 1);
    store::reclaim_expired(&db, later).unwrap();
    let again = store::claim_next(&db, "test", later).unwrap().expect("the job is claimed again");
    assert_eq!(again.id, job.id);
    again
}

fn prepare_job(fixture: &Fixture, job: &JobRow) -> Result<Prepared, StepError> {
    prepare(&fixture.core, &PrepareConfig::default(), job, &CancelToken::new())
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn prepare_checks_out_the_pr_head_with_a_file_the_local_branch_lacks() {
    let fixture = fixture();
    let local_head = git(&fixture.local, &["rev-parse", "HEAD"]);
    let workspaces = || -> i64 {
        fixture.core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM workspaces", [], |row| row.get(0)).unwrap()
    };
    let before = workspaces();
    let job = claimed(&fixture, 7, &fixture.head);
    let prepared = prepare_job(&fixture, &job).unwrap();

    let checkout = prepared.worktree.clone().unwrap();
    assert_eq!(checkout, fixture.layout().checkout(&job));
    assert!(checkout.file_name().unwrap().to_string_lossy().starts_with(&format!("pr-7-{}-1", job_tag(&job.id))));
    assert_eq!(std::fs::read_to_string(checkout.join("new.txt")).unwrap(), "only in the PR\n");
    assert!(!fixture.local.join("new.txt").exists(), "the local branch never had it");
    assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), fixture.head);
    assert_eq!(prepared.head_sha, fixture.head);

    assert_eq!(git(&fixture.local, &["rev-parse", "HEAD"]), local_head, "local HEAD unchanged");
    assert_eq!(git(&fixture.local, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(git(&fixture.local, &["status", "--porcelain"]), "", "local checkout clean");
    assert_eq!(git(&fixture.local, &["branch", "--list"]), "* main", "no branch created");
    assert_eq!(workspaces(), before, "no workspace row, so no sidebar node");

    let stored = store::require_job(&fixture.core.db.lock().unwrap(), &job.id).unwrap();
    assert_eq!(stored.worktree_path, Some(canonical_key(&checkout)));
    let record = worktree_registry::records(&fixture.core.db.lock().unwrap())
        .unwrap()
        .into_iter()
        .find(|record| record.path == canonical_key(&checkout))
        .expect("the checkout is in the inventory");
    assert_eq!(record.kind, worktree_registry::KIND_REVIEW);
    assert_eq!(record.base_commit.as_deref(), Some(fixture.head.as_str()));
    drop(prepared);
    fixture.assert_clean();
}

#[test]
fn a_fork_pr_prepares_from_its_pull_ref() {
    let fixture = fixture();
    // Someone else's fork: its own clone, its own commit, pushed to the base
    // repository's pull ref only, as GitHub does for a fork PR.
    let fork = fixture.root.join("fork");
    git(&fixture.root, &["clone", "-q", "--branch", "main", fixture.remote.to_str().unwrap(), fork.to_str().unwrap()]);
    identity(&fork);
    std::fs::write(fork.join("from-fork.txt"), "fork work\n").unwrap();
    let head = commit(&fork, "fork change");
    git(&fork, &["push", "-q", "origin", "HEAD:refs/pull/8/head"]);
    let branches = git(&fixture.remote, &["branch", "--contains", &head]);
    assert!(branches.is_empty(), "the fork's commit is on no branch of the base repository");

    let job = claimed(&fixture, 8, &head);
    let prepared = prepare_job(&fixture, &job).unwrap();
    let checkout = prepared.worktree.clone().unwrap();
    assert_eq!(std::fs::read_to_string(checkout.join("from-fork.txt")).unwrap(), "fork work\n");
    assert_eq!(prepared.head_sha, head);
}

#[test]
fn a_moved_head_is_reported_for_the_runner_to_adopt() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, "0000000000000000000000000000000000000000");
    let prepared = prepare_job(&fixture, &job).unwrap();
    assert_eq!(prepared.head_sha, fixture.head, "the head actually on GitHub, for set_head");
    assert_eq!(git(prepared.worktree.as_ref().unwrap(), &["rev-parse", "HEAD"]), fixture.head);
}

fn assert_pr_json(pr: &serde_json::Value, fixture: &Fixture) {
    assert_eq!(pr["number"], 7);
    assert_eq!((pr["repoOwner"].as_str(), pr["repoName"].as_str()), (Some("fixture"), Some("project")));
    assert_eq!(pr["title"], "Changes requested", "from gh");
    assert_eq!(pr["body"], "Please review the typed GitHub surface.");
    assert_eq!(pr["author"], "review-author");
    assert_eq!(pr["baseRef"], "main");
    assert_eq!(pr["headSha"], fixture.head.as_str());
    let merge_base = git(&fixture.local, &["merge-base", "main", &fixture.head]);
    assert_eq!(pr["mergeBase"], merge_base.as_str());
    assert_ne!(pr["baseSha"], pr["mergeBase"], "main moved on after the PR branched");
    assert_eq!(pr["changedFiles"], 5);
    assert_eq!(pr["diffTruncated"], false);
    assert_eq!(pr["omittedFiles"], serde_json::json!([]));
}

fn assert_files_json(files: &serde_json::Value) {
    let by_path = |path: &str| files.as_array().unwrap().iter().find(|file| file["path"] == path).cloned().unwrap();
    assert_eq!(by_path("a.txt")["status"], "modified");
    assert_eq!((by_path("a.txt")["additions"].clone(), by_path("a.txt")["deletions"].clone()), (serde_json::json!(1), serde_json::json!(0)));
    assert_eq!(by_path("new.txt")["status"], "added");
    assert_eq!(by_path("gone.txt")["status"], "deleted");
    assert_eq!(by_path("renamed.txt")["status"], "renamed");
    assert_eq!(by_path("renamed.txt")["previousPath"], "old.txt");
    assert!(by_path("bin.dat")["additions"].is_null(), "binary changes have no line counts");
    assert!(files.as_array().unwrap().iter().all(|file| file["path"] != "later.txt"), "only the PR's changes");
}

#[test]
fn context_files_describe_the_pr() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, &fixture.head);
    let prepared = prepare_job(&fixture, &job).unwrap();
    let context = prepared.context.clone().unwrap();
    assert_eq!(context, fixture.layout().attempt_dir(&job).join("context"), "each attempt has its own context");
    let pr = read_json(&context.join("pr.json"));
    assert_pr_json(&pr, &fixture);
    assert_files_json(&read_json(&context.join("files.json")));
    let patch = std::fs::read_to_string(context.join("diff.patch")).unwrap();
    assert!(patch.contains("+three") && patch.contains("+only in the PR"));
    assert!(!patch.contains("main moved on"));
    assert_eq!(pr["diffBytes"], patch.len());
    let threads = read_json(&context.join("threads.json"));
    assert!(!threads.as_array().unwrap().is_empty(), "the gh stub's threads");
    assert!(threads[0]["path"].is_string() && threads[0].get("isResolved").is_some());
}

#[test]
fn an_oversized_diff_keeps_whole_files_and_lists_the_rest() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, &fixture.head);
    let merge_base = git(&fixture.local, &["merge-base", "main", &fixture.head]);
    let full = git(&fixture.local, &["diff", "-M", "--no-color", &merge_base, &fixture.head]);
    // Room for roughly half of the patch.
    let config = PrepareConfig { max_diff_bytes: full.len() / 2 };
    let prepared = prepare(&fixture.core, &config, &job, &CancelToken::new()).unwrap();
    let context = prepared.context.clone().unwrap();
    let pr = read_json(&context.join("pr.json"));
    let patch = std::fs::read_to_string(context.join("diff.patch")).unwrap();
    assert!(patch.len() <= config.max_diff_bytes);
    assert_eq!(pr["diffTruncated"], true);
    let omitted: Vec<String> = serde_json::from_value(pr["omittedFiles"].clone()).unwrap();
    assert!(!omitted.is_empty());
    let files = read_json(&context.join("files.json"));
    for file in files.as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        let included = patch.contains(&format!(" b/{path}\n"));
        assert_ne!(included, omitted.iter().any(|name| name == path), "{path} is either whole in the patch or listed");
    }
    // Every section kept is a whole file: each starts at a header.
    assert!(patch.is_empty() || patch.starts_with("diff --git "));
    assert_eq!(patch.matches("diff --git ").count() + omitted.len(), 5);
}

#[test]
fn fetch_failures_are_transient_and_a_missing_repository_is_fatal() {
    let fixture = fixture();
    let missing_pr = claimed(&fixture, 99, "abc");
    match prepare_job(&fixture, &missing_pr) {
        Err(StepError::Fatal(message)) => assert!(message.contains("not on GitHub any more"), "{message}"),
        other => panic!("a PR head GitHub does not have is fatal: {other:?}"),
    }
    git(&fixture.local, &["remote", "set-url", "origin", fixture.root.join("unreachable.git").to_str().unwrap()]);
    let unreachable = claimed(&fixture, 7, &fixture.head);
    assert!(matches!(prepare_job(&fixture, &unreachable), Err(StepError::Transient(_))), "a remote that does not answer is worth retrying");
    fixture.core.db.lock().unwrap().execute("UPDATE workspaces SET path=?1", params![fixture.root.join("data").to_string_lossy()]).unwrap();
    let no_repo = claimed(&fixture, 7, "def");
    match prepare_job(&fixture, &no_repo) {
        Err(StepError::Fatal(message)) => assert!(message.contains("not a git repository"), "{message}"),
        other => panic!("{other:?}"),
    }
    fixture.assert_clean();
}

#[test]
fn cleanup_is_idempotent_and_removes_every_attempt() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, &fixture.head);
    let first = prepare_job(&fixture, &job).unwrap();
    // The job is reclaimed and claimed again: a second attempt, a second checkout.
    let second_claim = reclaimed(&fixture, &job);
    assert_eq!(second_claim.attempts, 2);
    let second = prepare_job(&fixture, &second_claim).unwrap();
    assert_ne!(first.worktree, second.worktree);
    assert_eq!(fixture.checkouts().len(), 2);

    cleanup(&fixture.core, &job.id);
    fixture.assert_clean();
    cleanup(&fixture.core, &job.id);
    drop(first);
    drop(second);
    fixture.assert_clean();
    let records = worktree_registry::records(&fixture.core.db.lock().unwrap()).unwrap();
    assert!(records.iter().filter(|record| record.kind == worktree_registry::KIND_REVIEW).all(|record| record.state == "removed"));
}

#[test]
fn the_guard_cleans_up_when_a_step_panics() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, &fixture.head);
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let prepared = prepare_job(&fixture, &job).unwrap();
        assert!(prepared.worktree.as_ref().unwrap().exists());
        panic!("the review step blew up");
    }));
    assert!(outcome.is_err());
    fixture.assert_clean();
}

#[test]
fn the_reaper_removes_orphans_and_keeps_live_checkouts() {
    let fixture = fixture();
    let live = claimed(&fixture, 7, &fixture.head);
    let live_prepared = prepare_job(&fixture, &live).unwrap();
    let dead = claimed(&fixture, 7, "other-head");
    let dead_prepared = prepare_job(&fixture, &dead).unwrap();
    let dead_checkout = dead_prepared.worktree.clone().unwrap();
    // Its process died: the guard never runs.
    let _died = std::mem::ManuallyDrop::new(dead_prepared);
    fixture.core.db.lock().unwrap().execute("UPDATE review_jobs SET status='failed' WHERE id=?1", params![dead.id]).unwrap();
    // A stray directory, a stale job folder, and a registration whose
    // directory someone deleted by hand.
    let stray = fixture.layout().reviews.join("pr-1-stray-1");
    std::fs::create_dir_all(&stray).unwrap();
    std::fs::create_dir_all(fixture.layout().jobs.join("gone-job")).unwrap();
    let deleted = fixture.layout().reviews.join("pr-2-deleted-1");
    git::add_detached_worktree(&fixture.local, &deleted, &fixture.head).unwrap();
    std::fs::remove_dir_all(&deleted).unwrap();

    let reaped = reap(&fixture.core).unwrap();
    // The hand-deleted registration may be pruned while removing the dead
    // checkout, before it is counted; what matters is that git lists only the
    // live checkout afterwards (below).
    assert!(reaped.checkouts >= 2 && reaped.job_dirs >= 2, "{reaped:?}");
    assert!(!dead_checkout.exists() && !stray.exists());
    let live_checkout = live_prepared.worktree.clone().unwrap();
    assert_eq!(fixture.checkouts(), std::slice::from_ref(&live_checkout), "the live checkout stays");
    assert_eq!(fixture.registered(), std::slice::from_ref(&live_checkout), "git lists only the live one");
    assert!(fixture.layout().job_dir(&live.id).exists());
    assert!(!fixture.layout().job_dir(&dead.id).exists());
}

#[test]
fn the_budget_refuses_when_full_after_a_sweep() {
    let fixture = fixture();
    {
        let db = fixture.core.db.lock().unwrap();
        crate::reviews::settings::save(&db, &wire::ReviewJobSettings { max_review_worktrees: 1, ..Default::default() }).unwrap();
    }
    assert!(has_capacity(&fixture.core));
    let job = claimed(&fixture, 7, &fixture.head);
    assert!(!has_capacity(&fixture.core), "a claimed job counts before it cuts its checkout");
    let prepared = prepare_job(&fixture, &job).unwrap();
    assert!(!has_capacity(&fixture.core), "one live checkout fills a budget of one");
    // Its process died: the guard never runs.
    let _died = std::mem::ManuallyDrop::new(prepared);
    fixture.core.db.lock().unwrap().execute("UPDATE review_jobs SET status='failed' WHERE id=?1", params![job.id]).unwrap();
    assert_eq!(fixture.checkouts().len(), 1, "the dead job's checkout is still on disk");
    assert!(has_capacity(&fixture.core), "the sweep frees it");
    fixture.assert_clean();
}

/// Run as a child process by the next test: cut a review checkout, say so,
/// then wait to be killed.
#[test]
#[ignore = "a helper the kill test runs in a child process"]
fn review_kill_child() {
    let (Ok(repo), Ok(checkout), Ok(head)) =
        (std::env::var("REVIEW_KILL_REPO"), std::env::var("REVIEW_KILL_CHECKOUT"), std::env::var("REVIEW_KILL_HEAD"))
    else {
        return;
    };
    let checkout = PathBuf::from(checkout);
    let _guard = Guard::new({
        let checkout = checkout.clone();
        move || drop(std::fs::remove_dir_all(checkout))
    });
    git::add_detached_worktree(Path::new(&repo), &checkout, &head).unwrap();
    std::fs::write(checkout.with_extension("ready"), "ready").unwrap();
    std::thread::sleep(Duration::from_secs(60));
}

#[test]
fn a_killed_process_leaves_nothing_after_restart() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, &fixture.head);
    let checkout = fixture.layout().checkout(&job);
    let ready = checkout.with_extension("ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "reviews::prepare::tests::review_kill_child", "--ignored", "--nocapture"])
        .env("REVIEW_KILL_REPO", &fixture.local)
        .env("REVIEW_KILL_CHECKOUT", &checkout)
        .env("REVIEW_KILL_HEAD", &fixture.head)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "the child never cut its checkout");
        std::thread::sleep(Duration::from_millis(20));
    }
    // SIGKILL: no destructor, no guard, nothing in-process runs.
    child.kill().unwrap();
    child.wait().unwrap();
    std::fs::remove_file(&ready).unwrap();
    assert!(checkout.join("new.txt").exists(), "the killed process left its checkout");
    assert_eq!(fixture.registered(), std::slice::from_ref(&checkout));
    {
        let db = fixture.core.db.lock().unwrap();
        db.execute(
            "UPDATE review_jobs SET status='reviewing', worktree_path=?2 WHERE id=?1",
            params![job.id, canonical_key(&checkout)],
        )
        .unwrap();
    }

    // Bridge starts again.
    fixture.core.reviews.set_steps(Arc::new(LiveSteps::new(&fixture.core)));
    let runner = Runner::new(fixture.core.clone());
    assert_eq!(runner.recover().unwrap(), [job.id.clone()]);
    assert_eq!(store::require_job(&fixture.core.db.lock().unwrap(), &job.id).unwrap().status, Status::Interrupted);
    fixture.assert_clean();
}

// --- every ending, through the runner -----------------------------------------

/// The live preparation and cleanup, with a scripted reviewer and poster.
struct Hybrid {
    live: LiveSteps,
    review: Mutex<VecDeque<ReviewPlan>>,
    posted: Mutex<Vec<String>>,
}

enum ReviewPlan {
    Finish,
    Fail(StepError),
    WaitForCancel,
    Panic,
}

impl ReviewSteps for Hybrid {
    fn prepare(&self, job: &JobRow, cancel: &CancelToken) -> Result<Prepared, StepError> {
        self.live.prepare(job, cancel)
    }

    fn review(&self, _request: PassRequest<'_>, cancel: &CancelToken) -> Result<PassOutcome, StepError> {
        match self.review.lock().unwrap().pop_front().unwrap_or(ReviewPlan::Finish) {
            ReviewPlan::Finish => Ok(PassOutcome { summary: "ok".into(), ..PassOutcome::default() }),
            ReviewPlan::Fail(error) => Err(error),
            ReviewPlan::WaitForCancel => {
                while !cancel.sleep(Duration::from_millis(10)) {}
                Err(StepError::Cancelled)
            }
            ReviewPlan::Panic => panic!("the reviewer crashed"),
        }
    }

    fn find_posted(&self, job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        let marker = post_marker(&job.publication_key);
        Ok(self.posted.lock().unwrap().iter().any(|body| body.contains(&marker)).then(|| PostedReview {
            review_id: "1".into(),
            url: "https://example.test/1".into(),
            findings: Vec::new(),
        }))
    }

    fn post(&self, job: &JobRow, _cancel: &CancelToken) -> Result<PostedReview, StepError> {
        self.posted.lock().unwrap().push(post_marker(&job.publication_key));
        Ok(PostedReview { review_id: "1".into(), url: "https://example.test/1".into(), findings: Vec::new() })
    }

    fn cleanup(&self, job_id: &str) {
        self.live.cleanup(job_id);
    }

    fn has_capacity(&self) -> bool {
        self.live.has_capacity()
    }

    fn reap(&self) {
        self.live.reap();
    }
}

fn fast() -> RunnerConfig {
    RunnerConfig {
        renew_every: Duration::from_millis(30),
        retry_backoff: Duration::from_millis(1),
        step_retries: 1,
        poll: Duration::from_millis(10),
    }
}

fn start(fixture: &Fixture) -> String {
    let params = wire::StartReviewParams { workspace_id: "w".into(), number: 7, harnesses: Some(vec!["claude".into()]), trigger: None };
    let head = crate::reviews::PrHead { repo_owner: "fixture".into(), repo_name: "project".into(), head_sha: fixture.head.clone(), base_sha: None };
    crate::reviews::start_with(&fixture.core, &params, head).unwrap().id
}

fn status(fixture: &Fixture, id: &str) -> Status {
    store::require_job(&fixture.core.db.lock().unwrap(), id).unwrap().status
}

fn drain(runner: &Runner) {
    let deadline = Instant::now() + Duration::from_secs(60);
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
    }
}

fn every_ending() -> (Fixture, Arc<Hybrid>, Runner) {
    let fixture = fixture();
    let steps = Arc::new(Hybrid {
        live: LiveSteps::new(&fixture.core),
        review: Mutex::new(VecDeque::new()),
        posted: Mutex::new(Vec::new()),
    });
    fixture.core.reviews.set_steps(steps.clone());
    let runner = Runner::with(fixture.core.clone(), fast(), Arc::new(chrono::Utc::now));
    (fixture, steps, runner)
}

fn set_post_mode(fixture: &Fixture, mode: wire::ReviewPostMode) {
    let db = fixture.core.db.lock().unwrap();
    let mut settings = crate::reviews::settings::load(&db).unwrap();
    settings.post_mode = mode;
    crate::reviews::settings::save(&db, &settings).unwrap();
}

/// Run one job to its end with the reviewer doing `plan`, and check nothing
/// of it is left on disk or in git.
fn ends_clean(fixture: &Fixture, steps: &Hybrid, runner: &Runner, plan: Option<ReviewPlan>, expected: Status) -> String {
    if let Some(plan) = plan {
        steps.review.lock().unwrap().push_back(plan);
    }
    let id = start(fixture);
    drain(runner);
    assert_eq!(status(fixture, &id), expected);
    fixture.assert_clean();
    id
}

#[test]
fn worktrees_are_gone_after_posting_drafting_and_failing_at_review() {
    let (fixture, steps, runner) = every_ending();
    set_post_mode(&fixture, wire::ReviewPostMode::Auto);
    ends_clean(&fixture, &steps, &runner, None, Status::Posted);
    set_post_mode(&fixture, wire::ReviewPostMode::Draft);
    let drafted = ends_clean(&fixture, &steps, &runner, None, Status::Drafted);
    crate::reviews::cancel(&fixture.core, &drafted).unwrap();
    ends_clean(&fixture, &steps, &runner, Some(ReviewPlan::Fail(StepError::Fatal("the model refused".into()))), Status::Failed);
}

#[test]
fn worktrees_are_gone_after_a_failed_prepare_and_a_panicking_review() {
    let (fixture, steps, runner) = every_ending();
    let url = git(&fixture.local, &["remote", "get-url", "origin"]);
    git(&fixture.local, &["remote", "set-url", "origin", fixture.root.join("nowhere.git").to_str().unwrap()]);
    ends_clean(&fixture, &steps, &runner, None, Status::Failed);
    git(&fixture.local, &["remote", "set-url", "origin", &url]);
    ends_clean(&fixture, &steps, &runner, Some(ReviewPlan::Panic), Status::Failed);
}

#[test]
fn worktrees_are_gone_after_a_cancel_mid_review() {
    let (fixture, steps, runner) = every_ending();
    steps.review.lock().unwrap().push_back(ReviewPlan::WaitForCancel);
    let cancelled = start(&fixture);
    let handles = runner.tick().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while status(&fixture, &cancelled) != Status::Reviewing {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fixture.checkouts().len(), 1, "the checkout exists while the review runs");
    crate::reviews::cancel(&fixture.core, &cancelled).unwrap();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(status(&fixture, &cancelled), Status::Cancelled);
    fixture.assert_clean();
}

// --- regressions for review 6018851502 ------------------------------------------

#[test]
fn a_sweep_during_checkout_creation_keeps_the_new_checkout() {
    let fixture = fixture();
    let paused = fixture.root.join("paused");
    let resume = fixture.root.join("resume");
    let hook = fixture.local.join(".git/hooks/post-checkout");
    std::fs::write(
        &hook,
        format!("#!/bin/sh\ntouch '{}'\nwhile [ ! -f '{}' ]; do sleep 0.02; done\n", paused.display(), resume.display()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    let job = claimed(&fixture, 7, &fixture.head);
    let preparing = {
        let (core, job) = (fixture.core.clone(), job.clone());
        std::thread::spawn(move || prepare(&core, &PrepareConfig::default(), &job, &CancelToken::new()))
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !paused.exists() {
        assert!(Instant::now() < deadline, "git worktree add never ran its hook");
        std::thread::sleep(Duration::from_millis(10));
    }
    // The checkout's directory exists now, mid-creation. A sweep runs.
    let sweeping = {
        let core = fixture.core.clone();
        std::thread::spawn(move || reap(&core))
    };
    std::thread::sleep(Duration::from_millis(300));
    assert!(!sweeping.is_finished(), "the sweep waits for the checkout being cut");
    std::fs::write(&resume, "go").unwrap();
    let prepared = preparing.join().unwrap().unwrap();
    sweeping.join().unwrap().unwrap();
    std::fs::remove_file(&hook).unwrap();

    let checkout = prepared.worktree.clone().unwrap();
    assert!(checkout.join("new.txt").exists(), "the sweep did not remove the new checkout");
    assert_eq!(fixture.registered(), std::slice::from_ref(&checkout));
}

#[test]
fn a_superseded_attempts_guard_leaves_the_replacement_context() {
    let fixture = fixture();
    let job = claimed(&fixture, 7, &fixture.head);
    let first = prepare_job(&fixture, &job).unwrap();
    let again = reclaimed(&fixture, &job);
    let second = prepare_job(&fixture, &again).unwrap();
    let (second_head, _) = attempt_refs(&again);
    let (first_head, _) = attempt_refs(&job);
    drop(first);
    let context = second.context.clone().unwrap();
    for file in ["pr.json", "diff.patch", "files.json", "threads.json"] {
        assert!(context.join(file).exists(), "attempt 2's {file} survives attempt 1's guard");
    }
    assert!(second.worktree.as_ref().unwrap().join("new.txt").exists());
    assert_eq!(git::resolve_commit(&fixture.local, &second_head).unwrap(), fixture.head);
    assert!(git::resolve_commit(&fixture.local, &first_head).is_err(), "attempt 1 removed only its own refs");
    drop(second);
    fixture.assert_clean();
}

#[test]
fn a_type_change_keeps_both_halves_with_its_file() {
    let fixture = fixture();
    git(&fixture.local, &["checkout", "-q", "-b", "typechange"]);
    std::fs::remove_file(fixture.local.join("a.txt")).unwrap();
    std::os::unix::fs::symlink("zz.txt", fixture.local.join("a.txt")).unwrap();
    std::fs::write(fixture.local.join("zz.txt"), "last\nand more\n").unwrap();
    let head = commit(&fixture.local, "a.txt becomes a link; zz.txt changes");
    git(&fixture.local, &["push", "-q", "origin", "HEAD:refs/pull/9/head"]);
    git(&fixture.local, &["checkout", "-q", "main"]);

    let job = claimed(&fixture, 9, &head);
    let prepared = prepare_job(&fixture, &job).unwrap();
    let context = prepared.context.clone().unwrap();
    let files = read_json(&context.join("files.json"));
    let statuses: Vec<(String, String)> = files
        .as_array()
        .unwrap()
        .iter()
        .map(|file| (file["path"].as_str().unwrap().to_owned(), file["status"].as_str().unwrap().to_owned()))
        .collect();
    assert_eq!(statuses, [("a.txt".into(), "typechange".into()), ("zz.txt".into(), "modified".into())]);
    let patch = std::fs::read_to_string(context.join("diff.patch")).unwrap();
    assert_eq!(patch.matches("diff --git a/a.txt b/a.txt").count(), 2, "both halves of the type change");
    assert!(patch.contains("+and more"), "zz.txt is not dropped:\n{patch}");
    let pr = read_json(&context.join("pr.json"));
    assert_eq!(pr["omittedFiles"], serde_json::json!([]));
    assert_eq!(pr["diffTruncated"], false);
}

#[test]
fn a_reclaimed_attempt_counts_against_the_budget_until_it_cuts_its_checkout() {
    let fixture = fixture();
    {
        let db = fixture.core.db.lock().unwrap();
        crate::reviews::settings::save(&db, &wire::ReviewJobSettings { max_review_worktrees: 1, ..Default::default() }).unwrap();
    }
    let job = claimed(&fixture, 7, &fixture.head);
    let first = prepare_job(&fixture, &job).unwrap();
    // The runner dies; its guard runs as the process unwinds, removing the
    // checkout; then the job is claimed again.
    {
        let db = fixture.core.db.lock().unwrap();
        store::reclaim_expired(&db, chrono::Utc::now() + chrono::Duration::seconds(store::LEASE_SECONDS + 1)).unwrap();
    }
    drop(first);
    assert!(fixture.checkouts().is_empty());
    let again = reclaimed_queued(&fixture, &job);
    assert_eq!(again.worktree_path, None, "the old attempt's checkout is not the new one's");
    assert!(!has_capacity(&fixture.core), "the replacement holds the only slot before it cuts its checkout");
    let prepared = prepare_job(&fixture, &again).unwrap();
    assert!(!has_capacity(&fixture.core));
    drop(prepared);
}

/// Claim a job a reclaim already put back in the queue.
fn reclaimed_queued(fixture: &Fixture, job: &JobRow) -> JobRow {
    let db = fixture.core.db.lock().unwrap();
    let later = chrono::Utc::now() + chrono::Duration::seconds(store::LEASE_SECONDS + 2);
    let again = store::claim_next(&db, "test", later).unwrap().expect("the job is claimed again");
    assert_eq!(again.id, job.id);
    again
}

#[test]
fn metadata_and_threads_come_from_the_jobs_repository() {
    let fixture = fixture();
    // The job was queued for fixture/project; by the time it runs, gh points
    // the workspace at a fork.
    std::fs::write(fixture.gh.path().join("default-repository"), "someone/fork").unwrap();
    let job = claimed(&fixture, 7, &fixture.head);
    prepare_job(&fixture, &job).unwrap();
    let invocations = fixture.invocations();
    let view = invocations.lines().find(|line| line.starts_with("pr view 7")).expect("a pr view");
    assert!(view.contains("--repo fixture/project"), "{view}");
    let threads = invocations.lines().find(|line| line.starts_with("api graphql")).expect("a threads read");
    assert!(threads.contains("owner=fixture") && threads.contains("name=project"), "{threads}");
    assert!(!invocations.contains("someone/fork --json") && !invocations.contains("owner=someone"));
}

#[test]
fn concurrent_jobs_on_the_same_pr_number_fetch_into_their_own_refs() {
    let fixture = fixture();
    let first = claimed_with(&fixture, 7, &fixture.head, "claude");
    let first_prepared = prepare_job(&fixture, &first).unwrap();
    // The PR moves on before the second job fetches.
    git(&fixture.local, &["checkout", "-q", "-b", "moved", &fixture.head]);
    std::fs::write(fixture.local.join("new.txt"), "moved on\n").unwrap();
    let moved = commit(&fixture.local, "a second PR commit");
    git(&fixture.local, &["push", "-q", "-f", "origin", "HEAD:refs/pull/7/head"]);
    git(&fixture.local, &["checkout", "-q", "main"]);
    let second = claimed_with(&fixture, 7, &fixture.head, "codex");
    let second_prepared = prepare_job(&fixture, &second).unwrap();

    let (first_ref, _) = attempt_refs(&first);
    let (second_ref, _) = attempt_refs(&second);
    assert_ne!(first_ref, second_ref);
    assert_eq!(git::resolve_commit(&fixture.local, &first_ref).unwrap(), fixture.head, "the first job keeps what it fetched");
    assert_eq!(git::resolve_commit(&fixture.local, &second_ref).unwrap(), moved);
    assert_eq!(first_prepared.head_sha, fixture.head);
    assert_eq!(second_prepared.head_sha, moved);
}

#[test]
fn a_shallow_clone_is_deepened_to_find_the_merge_base() {
    let fixture = fixture();
    let shallow = fixture.root.join("shallow");
    let url = format!("file://{}", fixture.remote.display());
    git(&fixture.root, &["clone", "-q", "--depth=1", "--branch", "main", &url, shallow.to_str().unwrap()]);
    assert!(git::is_shallow(&shallow));
    fixture.core.db.lock().unwrap().execute("UPDATE workspaces SET path=?1", params![shallow.to_string_lossy()]).unwrap();
    let job = claimed(&fixture, 7, &fixture.head);
    let prepared = prepare_job(&fixture, &job).unwrap();
    let pr = read_json(&prepared.context.clone().unwrap().join("pr.json"));
    let expected = git(&fixture.local, &["merge-base", "main", &fixture.head]);
    assert_eq!(pr["mergeBase"], expected.as_str(), "the real merge base, found by deepening");
    assert_eq!(pr["changedFiles"], 5);
}
