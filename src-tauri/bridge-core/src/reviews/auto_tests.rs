use super::*;
use rusqlite::params;

fn pr(number: u64, head: &str, draft: bool, author: &str) -> OpenPullRequestHead {
    OpenPullRequestHead { number, head_sha: head.into(), is_draft: draft, author: Some(author.into()) }
}

fn all() -> Filters {
    Filters { mode: ReviewAutoMode::All, skip_drafts: true, viewer: Some("me".into()) }
}

fn never(_: u64, _: &str) -> bool {
    false
}

#[test]
fn the_first_listing_is_a_baseline_and_a_new_head_waits_for_the_debounce() {
    let start = Instant::now();
    let mut watch = Watch::default();
    observe(&mut watch, &[pr(7, "a", false, "me")], true, &all(), start, never);
    assert!(due(&mut watch, start + DEBOUNCE * 10).is_empty(), "turning it on reviews nothing at once");

    observe(&mut watch, &[pr(7, "b", false, "me")], true, &all(), start + LIST_EVERY, never);
    assert!(due(&mut watch, start + LIST_EVERY + DEBOUNCE - Duration::from_secs(1)).is_empty());
    assert_eq!(due(&mut watch, start + LIST_EVERY + DEBOUNCE), [(7, "b".to_owned())]);
    assert!(due(&mut watch, start + LIST_EVERY * 10).is_empty(), "handed out once");
}

#[test]
fn two_pushes_within_a_minute_start_one_review_at_the_last_head() {
    let start = Instant::now();
    let mut watch = Watch::default();
    observe(&mut watch, &[pr(7, "a", false, "me")], true, &all(), start, never);
    observe(&mut watch, &[pr(7, "b", false, "me")], true, &all(), start + LIST_EVERY, never);
    // The second push lands before the first one's quiet period ends.
    observe(&mut watch, &[pr(7, "c", false, "me")], true, &all(), start + LIST_EVERY * 2, never);
    assert!(due(&mut watch, start + LIST_EVERY + DEBOUNCE).is_empty(), "the debounce restarts on the second push");
    assert_eq!(due(&mut watch, start + LIST_EVERY * 2 + DEBOUNCE), [(7, "c".to_owned())]);
}

#[test]
fn a_pr_that_opens_after_the_baseline_is_reviewed_and_a_closed_one_is_forgotten() {
    let start = Instant::now();
    let mut watch = Watch::default();
    observe(&mut watch, &[pr(7, "a", false, "me")], true, &all(), start, never);
    observe(&mut watch, &[pr(7, "a", false, "me"), pr(8, "x", false, "me")], true, &all(), start + LIST_EVERY, never);
    assert_eq!(due(&mut watch, start + LIST_EVERY + DEBOUNCE), [(8, "x".to_owned())]);
    observe(&mut watch, &[pr(8, "y", false, "me")], true, &all(), start + LIST_EVERY * 2, never);
    assert!(!watch.prs.contains_key(&7), "closed or merged: forgotten");
    // Reopened later, it is new again.
    observe(&mut watch, &[pr(7, "a", false, "me"), pr(8, "y", false, "me")], true, &all(), start + LIST_EVERY * 3, never);
    assert!(watch.prs[&7].pending_since.is_some());
}

#[test]
fn on_the_baseline_only_a_pr_pushed_while_bridge_was_closed_is_reviewed() {
    let start = Instant::now();
    let mut watch = Watch::default();
    // #7's last review was a push review of an older head; #8's was manual.
    observe(&mut watch, &[pr(7, "new", false, "me"), pr(8, "new", false, "me")], true, &all(), start, |number, _| number == 7);
    assert_eq!(due(&mut watch, start + DEBOUNCE), [(7, "new".to_owned())]);
}

#[test]
fn the_filters_decide_which_prs_are_reviewed() {
    let mine = Filters { mode: ReviewAutoMode::Mine, skip_drafts: true, viewer: Some("Me".into()) };
    let cases = [
        // (filters, draft, author, taken)
        (all(), false, "someone", true),
        (all(), true, "someone", false),
        (Filters { skip_drafts: false, ..all() }, true, "someone", true),
        (mine.clone(), false, "me", true),
        (mine.clone(), false, "someone", false),
        (mine.clone(), true, "me", false),
        (Filters { skip_drafts: false, ..mine.clone() }, true, "me", true),
        (Filters { viewer: None, ..mine.clone() }, false, "me", false),
        (Filters { mode: ReviewAutoMode::Off, ..all() }, false, "me", false),
    ];
    for (index, (filters, draft, author, taken)) in cases.into_iter().enumerate() {
        assert_eq!(filters.takes(&pr(1, "a", draft, author)), taken, "case {index}");
    }
}

#[test]
fn a_draft_marked_ready_counts_as_a_change() {
    let start = Instant::now();
    let mut watch = Watch::default();
    observe(&mut watch, &[pr(7, "a", true, "me")], true, &all(), start, never);
    observe(&mut watch, &[pr(7, "b", true, "me")], true, &all(), start + LIST_EVERY, never);
    assert!(due(&mut watch, start + LIST_EVERY * 10).is_empty(), "a draft is skipped");
    observe(&mut watch, &[pr(7, "b", false, "me")], true, &all(), start + LIST_EVERY * 11, never);
    assert_eq!(due(&mut watch, start + LIST_EVERY * 11 + DEBOUNCE), [(7, "b".to_owned())]);
}

// --- the tick, against the fake `gh` and a real store --------------------------

struct Fixture {
    scratch: tempfile::TempDir,
    gh: std::path::PathBuf,
    core: Arc<BridgeCore>,
}

impl Fixture {
    fn new() -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let repo = scratch.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        for args in [vec!["init", "-q", "-b", "main"], vec!["remote", "add", "origin", "https://github.com/fixture/project.git"]] {
            assert!(crate::git::git_command(&repo).args(&args).status().unwrap().success());
        }
        // A fixtures folder of its own, so the listing can change per step.
        let shared = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/github").canonicalize().unwrap();
        let fixtures = scratch.path().join("fixtures");
        std::fs::create_dir(&fixtures).unwrap();
        for entry in std::fs::read_dir(&shared).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), fixtures.join(entry.file_name())).unwrap();
        }
        let gh = scratch.path().join("gh-bin");
        std::fs::create_dir(&gh).unwrap();
        std::fs::write(gh.join("fixture-root"), fixtures.to_str().unwrap()).unwrap();
        std::fs::write(gh.join("auth-exit"), "0").unwrap();
        std::fs::write(gh.join("default-repository"), "fixture/project").unwrap();
        std::fs::write(gh.join("pr-list-fixture"), "heads.json").unwrap();
        std::os::unix::fs::symlink(shared.join("fake-gh.sh"), gh.join("gh")).unwrap();
        let mut core = BridgeCore::for_tests(&scratch.path().join("data"));
        core.github_surface = crate::github_surface::GithubSurface::discover_on_path(&gh);
        let core = Arc::new(core);
        {
            let db = core.db.lock().unwrap();
            db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','P',?1,'now')", params![repo.to_string_lossy()]).unwrap();
            db.execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w','p','c','t','main',?1,'ready','now')",
                params![repo.to_string_lossy()],
            )
            .unwrap();
        }
        core.reviews.auto.expose("w", repo);
        Fixture { gh, core, scratch }
    }

    /// A second workspace: another checkout of the same repository.
    fn second_workspace(&self) {
        let repo = self.scratch.path().join("repo-2");
        std::fs::create_dir(&repo).unwrap();
        for args in [vec!["init", "-q", "-b", "main"], vec!["remote", "add", "origin", "https://github.com/fixture/project.git"]] {
            assert!(crate::git::git_command(&repo).args(&args).status().unwrap().success());
        }
        self.core
            .db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES('w2','p','c','t2','main',?1,'ready','now')",
                params![repo.to_string_lossy()],
            )
            .unwrap();
        self.core.reviews.auto.expose("w2", repo);
    }

    fn fixtures(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(std::fs::read_to_string(self.gh.join("fixture-root")).unwrap())
    }

    /// Every job of #103, from any workspace.
    fn all_jobs(&self) -> i64 {
        self.core.db.lock().unwrap().query_row("SELECT COUNT(*) FROM review_jobs WHERE pr_number=103", [], |row| row.get(0)).unwrap()
    }

    fn calls(&self, prefix: &str) -> usize {
        std::fs::read_to_string(self.gh.join("invocations.log")).unwrap_or_default().lines().filter(|line| line.starts_with(prefix)).count()
    }

    /// A head change on #103 seen at `start + LIST_EVERY`, due at `start + LIST_EVERY + DEBOUNCE`.
    fn pending_change(&self, start: Instant) {
        self.heads(&[(103, "old-sha", false, "review-author")]);
        tick(&self.core, start);
        self.heads(&[(103, "current-head-sha", false, "review-author")]);
        tick(&self.core, start + LIST_EVERY);
    }

    /// What `gh pr list` answers from now on.
    fn heads(&self, heads: &[(u64, &str, bool, &str)]) {
        let listing: Vec<_> = heads
            .iter()
            .map(|(number, head, draft, author)| {
                serde_json::json!({"number": number, "headRefOid": head, "isDraft": draft, "author": {"login": author}})
            })
            .collect();
        let fixtures = std::fs::read_to_string(self.gh.join("fixture-root")).unwrap();
        std::fs::write(std::path::Path::new(&fixtures).join("heads.json"), serde_json::to_string(&listing).unwrap()).unwrap();
    }

    fn mode(&self, mode: ReviewAutoMode) {
        let db = self.core.db.lock().unwrap();
        let settings = super::super::settings::load(&db).unwrap();
        let default_harnesses = vec!["claude".into()];
        super::super::settings::save(&db, &ReviewJobSettings { auto_review: mode, default_harnesses, ..settings }).unwrap();
    }

    fn jobs(&self) -> Vec<super::super::store::JobRow> {
        super::super::store::list(&self.core.db.lock().unwrap(), "w", Some(103)).unwrap()
    }

    fn lists(&self) -> usize {
        std::fs::read_to_string(self.gh.join("invocations.log")).unwrap_or_default().lines().filter(|line| line.starts_with("pr list")).count()
    }
}

#[test]
fn a_burst_of_pushes_becomes_one_push_review_at_the_last_head() {
    let fixture = Fixture::new();
    fixture.mode(ReviewAutoMode::All);
    let start = Instant::now();
    fixture.heads(&[(103, "first-sha", false, "review-author")]);
    tick(&fixture.core, start);
    fixture.heads(&[(103, "second-sha", false, "review-author")]);
    tick(&fixture.core, start + LIST_EVERY);
    // `gh pr view` reports the PR's head as `current-head-sha`.
    fixture.heads(&[(103, "current-head-sha", false, "review-author")]);
    tick(&fixture.core, start + LIST_EVERY * 2);
    tick(&fixture.core, start + LIST_EVERY + DEBOUNCE);
    assert!(fixture.jobs().is_empty(), "still inside the quiet period of the last push");
    tick(&fixture.core, start + LIST_EVERY * 2 + DEBOUNCE);
    let jobs = fixture.jobs();
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    assert_eq!((jobs[0].head_sha.as_str(), jobs[0].trigger), ("current-head-sha", wire::ReviewTrigger::Push));
    // Nothing new: nothing more.
    tick(&fixture.core, start + LIST_EVERY * 10);
    assert_eq!(fixture.jobs().len(), 1);
}

#[test]
fn a_push_review_supersedes_queued_push_reviews_of_older_heads_but_not_a_persons() {
    let fixture = Fixture::new();
    fixture.mode(ReviewAutoMode::All);
    let queue = |head: &str, trigger: wire::ReviewTrigger, harness: &str| {
        let db = fixture.core.db.lock().unwrap();
        super::super::store::insert(
            &db,
            &super::super::store::NewJob {
                workspace_id: "w".into(),
                repo_owner: "fixture".into(),
                repo_name: "project".into(),
                number: 103,
                head_sha: head.into(),
                base_sha: None,
                incremental_from_sha: None,
                trigger,
                post_mode: Default::default(),
                harnesses: vec![harness.into()],
                retry_of: None,
                publication_key: None,
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .0
    };
    let older_push = queue("old-sha", wire::ReviewTrigger::Push, "claude");
    let manual = queue("old-sha", wire::ReviewTrigger::Manual, "codex");
    let start = Instant::now();
    fixture.heads(&[(103, "old-sha", false, "review-author")]);
    tick(&fixture.core, start);
    fixture.heads(&[(103, "current-head-sha", false, "review-author")]);
    tick(&fixture.core, start + LIST_EVERY);
    tick(&fixture.core, start + LIST_EVERY + DEBOUNCE);
    let db = fixture.core.db.lock().unwrap();
    let older = super::super::store::require_job(&db, &older_push.id).unwrap();
    assert_eq!(older.status, wire::ReviewJobStatus::Cancelled);
    assert_eq!(older.error.as_deref(), Some(super::super::store::SUPERSEDED_BY_PUSH));
    assert_eq!(super::super::store::require_job(&db, &manual.id).unwrap().status, wire::ReviewJobStatus::Queued, "a person's review stays");
}

#[test]
fn off_lists_nothing_and_starts_nothing_and_mine_skips_others() {
    let fixture = Fixture::new();
    let start = Instant::now();
    fixture.heads(&[(103, "a", false, "review-author")]);
    tick(&fixture.core, start);
    fixture.heads(&[(103, "current-head-sha", false, "review-author")]);
    tick(&fixture.core, start + LIST_EVERY);
    tick(&fixture.core, start + LIST_EVERY + DEBOUNCE);
    assert_eq!(fixture.lists(), 0, "off reads nothing");
    assert!(fixture.jobs().is_empty());

    // "My PRs" as `fixture-user`: #103 is someone else's.
    fixture.mode(ReviewAutoMode::Mine);
    tick(&fixture.core, start + LIST_EVERY * 5);
    fixture.heads(&[(103, "b", false, "review-author")]);
    tick(&fixture.core, start + LIST_EVERY * 6);
    tick(&fixture.core, start + LIST_EVERY * 6 + DEBOUNCE);
    assert!(fixture.jobs().is_empty(), "someone else's PR");

    // Who "me" is cannot be read: no baseline, so when it can be, nothing
    // of theirs looks new.
    fixture.mode(ReviewAutoMode::Off);
    tick(&fixture.core, start + LIST_EVERY * 9);
    std::fs::write(fixture.gh.join("viewer"), "not json").unwrap();
    fixture.mode(ReviewAutoMode::Mine);
    fixture.heads(&[(103, "c", false, "fixture-user")]);
    tick(&fixture.core, start + LIST_EVERY * 10);
    std::fs::remove_file(fixture.gh.join("viewer")).unwrap();
    tick(&fixture.core, start + LIST_EVERY * 11);
    tick(&fixture.core, start + LIST_EVERY * 11 + DEBOUNCE);
    assert!(fixture.jobs().is_empty(), "the first listing after the viewer is known is a baseline");

    // Turned off with a change pending: the change is forgotten.
    fixture.mode(ReviewAutoMode::All);
    tick(&fixture.core, start + LIST_EVERY * 20);
    fixture.heads(&[(103, "current-head-sha", false, "review-author")]);
    tick(&fixture.core, start + LIST_EVERY * 21);
    fixture.mode(ReviewAutoMode::Off);
    tick(&fixture.core, start + LIST_EVERY * 21 + DEBOUNCE);
    assert!(fixture.jobs().is_empty(), "off stops new reviews at once");
}

#[test]
fn a_partial_listing_keeps_what_it_left_out_and_only_new_numbers_are_new() {
    let start = Instant::now();
    let mut watch = Watch::default();
    observe(&mut watch, &[pr(5, "a", false, "me"), pr(9, "b", false, "me")], true, &all(), start, never);
    // A listing that hit the limit: #5 is past it, #7 was just outside before.
    observe(&mut watch, &[pr(9, "b", false, "me"), pr(7, "c", false, "me")], false, &all(), start + LIST_EVERY, never);
    assert!(watch.prs.contains_key(&5), "left out is not closed");
    assert_eq!(watch.prs[&7].pending_since, None, "an older number first seen is not a new PR");
    observe(&mut watch, &[pr(12, "d", false, "me"), pr(9, "b", false, "me")], false, &all(), start + LIST_EVERY * 2, never);
    assert!(watch.prs[&12].pending_since.is_some(), "a number above every PR seen is newly opened");
}

#[test]
fn saving_off_while_the_head_is_read_starts_nothing() {
    let fixture = Fixture::new();
    fixture.mode(ReviewAutoMode::All);
    let start = Instant::now();
    fixture.pending_change(start);
    // GitHub is slow to answer the head lookup; Off is saved meanwhile.
    std::fs::write(fixture.gh.join("pr-view-delay"), "2").unwrap();
    let core = fixture.core.clone();
    let ticking = std::thread::spawn(move || tick(&core, start + LIST_EVERY + DEBOUNCE));
    while fixture.calls("pr view") == 0 {
        std::thread::sleep(Duration::from_millis(20));
    }
    fixture.mode(ReviewAutoMode::Off);
    ticking.join().unwrap();
    assert_eq!(fixture.all_jobs(), 0, "the save returned before the insert, so no job");
}

#[test]
fn two_workspaces_of_one_repository_review_a_head_once() {
    let fixture = Fixture::new();
    fixture.second_workspace();
    fixture.mode(ReviewAutoMode::All);
    let start = Instant::now();
    fixture.pending_change(start);
    tick(&fixture.core, start + LIST_EVERY + DEBOUNCE);
    assert_eq!(fixture.all_jobs(), 1);
    assert_eq!(fixture.calls("pr list"), 3, "three rounds, one listing each: per repository, not per workspace");
}

#[test]
fn a_pr_that_turned_draft_is_not_started_from_a_stale_listing() {
    let fixture = Fixture::new();
    fixture.mode(ReviewAutoMode::All);
    let start = Instant::now();
    fixture.pending_change(start);
    // The listing then fails, and GitHub now says the PR is a draft.
    let mut detail: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture.fixtures().join("pr-detail.json")).unwrap()).unwrap();
    detail["isDraft"] = serde_json::json!(true);
    std::fs::write(fixture.fixtures().join("pr-detail-draft.json"), detail.to_string()).unwrap();
    std::fs::write(fixture.gh.join("pr-view-fixture"), "pr-detail-draft.json").unwrap();
    std::fs::write(fixture.fixtures().join("heads.json"), "not json").unwrap();
    tick(&fixture.core, start + LIST_EVERY + DEBOUNCE);
    assert_eq!(fixture.all_jobs(), 0);
}

#[test]
fn the_signed_in_login_is_read_again_after_an_hour() {
    let fixture = Fixture::new();
    fixture.mode(ReviewAutoMode::Mine);
    fixture.heads(&[(103, "a", false, "fixture-user")]);
    let start = Instant::now();
    for minute in 0..=61 {
        tick(&fixture.core, start + LIST_EVERY * minute);
    }
    assert_eq!(fixture.calls("api user"), 2, "read at the start and once the hour was up, not every minute and not never");
}
