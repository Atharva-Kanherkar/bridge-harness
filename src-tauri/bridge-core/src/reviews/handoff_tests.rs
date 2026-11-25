use super::*;
use std::sync::Mutex;

use bridge_protocol::messages::{ReviewPostMode, ReviewSeverity, ReviewSide, ReviewTrigger};

use crate::reviews::steps::{CancelToken, DraftFinding, PassOutcome, PassRequest, PostedReview, Prepared, ReviewSteps, StepError};

/// Records every message handed to a chat; refuses when told to.
#[derive(Default)]
struct Sender {
    sent: Mutex<Vec<(String, String)>>,
    refuse: Mutex<Option<String>>,
}

impl ReviewSteps for Sender {
    fn prepare(&self, _job: &JobRow, _cancel: &CancelToken) -> Result<Prepared, StepError> {
        Err(StepError::Fatal("unused".into()))
    }
    fn review(&self, _request: PassRequest<'_>, _cancel: &CancelToken) -> Result<PassOutcome, StepError> {
        Err(StepError::Fatal("unused".into()))
    }
    fn find_posted(&self, _job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        Ok(None)
    }
    fn post(&self, _job: &JobRow, _cancel: &CancelToken) -> Result<PostedReview, StepError> {
        Err(StepError::Fatal("unused".into()))
    }
    fn cleanup(&self, _job_id: &str) {}
    fn hand_off(&self, session_id: &str, text: &str) -> Result<(), String> {
        if let Some(error) = self.refuse.lock().unwrap().clone() {
            return Err(error);
        }
        self.sent.lock().unwrap().push((session_id.to_owned(), text.to_owned()));
        Ok(())
    }
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-25T10:00:00Z").unwrap().with_timezone(&Utc)
}

/// A project with its main checkout `w-main` (where reviews run) and a task
/// worktree `w-task` on `feat/cache`, holding an older chat, a newer chat, a
/// worker under the newer one, and an archived chat.
fn fixture(mode: ReviewHandoff) -> (tempfile::TempDir, Arc<BridgeCore>, Arc<Sender>) {
    let scratch = tempfile::tempdir().unwrap();
    let core = Arc::new(BridgeCore::for_tests(scratch.path()));
    {
        let db = core.db.lock().unwrap();
        let path = scratch.path().to_string_lossy().into_owned();
        db.execute_batch(&format!(
            "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','{path}','now');
             INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at) VALUES
               ('w-main','p','Oslo','Demo','main','{path}','ready','now'),
               ('w-task','p','Rome','Cache','feat/cache','{path}/task','ready','now');
             INSERT INTO sessions(id,workspace_id,harness,label,status) VALUES
               ('older','w-task','claude','Older chat','ready'),
               ('newer','w-task','claude','Add the cache','ready'),
               ('archived','w-task','claude','Archived chat','ready');
             INSERT INTO sessions(id,workspace_id,harness,label,status,parent_session_id) VALUES
               ('worker','w-task','codex','Worker','ready','newer');
             UPDATE sessions SET archived_at='now' WHERE id='archived';
             UPDATE sessions SET started_at='2026-11-25T09:00:00Z' WHERE id='older';
             UPDATE sessions SET started_at='2026-11-25T09:30:00Z' WHERE id IN ('newer','worker','archived');"
        ))
        .unwrap();
        let mut settings = crate::reviews::settings::load(&db).unwrap();
        settings.handoff = mode;
        crate::reviews::settings::save(&db, &settings).unwrap();
    }
    let sender = Arc::new(Sender::default());
    core.reviews.set_steps(sender.clone());
    (scratch, core, sender)
}

fn finding(path: &str, severity: ReviewSeverity, title: &str) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line: 12,
        start_line: Some(10),
        side: ReviewSide::Right,
        severity,
        title: title.into(),
        body: format!("Why {title} matters."),
        confidence: 0.8,
        fingerprint: format!("fp-{title}"),
        outside_diff: false,
    }
}

/// A drafted review of PR 7 on `feat/cache` by Claude with `findings`.
fn drafted(core: &Arc<BridgeCore>, number: u64, findings: &[DraftFinding]) -> String {
    let db = core.db.lock().unwrap();
    let (job, _) = store::insert(
        &db,
        &store::NewJob {
            workspace_id: "w-main".into(),
            repo_host: Some("github.com".into()),
            repo_owner: "octo".into(),
            repo_name: "app".into(),
            number,
            head_sha: format!("sha-{number}-{}", uuid::Uuid::new_v4()),
            base_sha: None,
            incremental_from_sha: None,
            trigger: ReviewTrigger::Manual,
            post_mode: ReviewPostMode::Draft,
            harnesses: vec!["claude".into()],
            retry_of: None,
            publication_key: None,
        },
        now(),
    )
    .unwrap();
    let pass = store::insert_pass(&db, &job.id, 0, 1, "claude", now()).unwrap();
    store::finish_pass(&db, &pass, bridge_protocol::messages::ReviewPassStatus::Done, None, Some(&PassOutcome::default()), now()).unwrap();
    store::insert_findings(&db, &job.id, &pass, findings, now()).unwrap();
    store::set_pr_meta(&db, &job.id, "Cache the session lookup", Some("someone"), Some("feat/cache")).unwrap();
    db.execute("UPDATE review_jobs SET status='drafted' WHERE id=?1", params![job.id]).unwrap();
    job.id
}

fn job(core: &Arc<BridgeCore>, id: &str) -> JobRow {
    store::require_job(&core.db.lock().unwrap(), id).unwrap()
}

#[test]
fn off_sends_nothing() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Off);
    let id = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
    assert_eq!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Skipped);
    assert!(sender.sent.lock().unwrap().is_empty());
}

#[test]
fn drafted_sends_every_open_finding_to_the_branch_chat() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Drafted);
    let id = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires"), finding("src/b.rs", ReviewSeverity::Nit, "Unused import")]);
    core.db.lock().unwrap().execute("UPDATE review_findings SET decision='dropped' WHERE title='Unused import'", []).unwrap();
    // The newer chat started last; the worker and the archived chat started as late, and do not count.
    assert_eq!(hand_off(&core, &id, Moment::Posted, now()).unwrap(), Outcome::Skipped, "drafted mode ignores the post");
    let outcome = hand_off(&core, &id, Moment::Drafted, now()).unwrap();
    assert_eq!(outcome, Outcome::Sent { session_id: "newer".into(), chat: "Add the cache".into() });
    let sent = sender.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let (session, text) = &sent[0];
    assert_eq!(session, "newer");
    assert!(text.starts_with("Bridge reviewed pull request #7 (Cache the session lookup) with Claude. 1 finding came back. Nobody has triaged them yet"), "{text}");
    assert!(text.contains("1. [major] src/a.rs:10-12 Cache never expires\nWhy Cache never expires matters."), "{text}");
    assert!(!text.contains("Unused import"), "a dropped finding is not sent");
    let stored = job(&core, &id);
    assert_eq!((stored.handoff_session_id.as_deref(), stored.handoff_chat.as_deref()), (Some("newer"), Some("Add the cache")));
    assert!(stored.handoff_at.is_some());
    let wire = store::to_wire(&core.db.lock().unwrap(), &stored).unwrap();
    assert_eq!(serde_json::to_value(&wire).unwrap()["handoffChat"], "Add the cache");
}

#[test]
fn posted_sends_only_what_was_posted_with_edits() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Posted);
    let id = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires"), finding("src/b.rs", ReviewSeverity::Minor, "Pending one")]);
    {
        let db = core.db.lock().unwrap();
        db.execute("UPDATE review_findings SET decision='edited', edited_body='Expire entries after an hour.' WHERE title='Cache never expires'", []).unwrap();
        db.execute("UPDATE review_jobs SET status='posted', posted_review_url='https://github.com/octo/app/pull/7#pullrequestreview-1' WHERE id=?1", params![id]).unwrap();
    }
    assert_eq!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Skipped, "posted mode waits for the post");
    assert!(matches!(hand_off(&core, &id, Moment::Posted, now()).unwrap(), Outcome::Sent { .. }));
    let text = sender.sent.lock().unwrap()[0].1.clone();
    assert!(text.contains("and posted 1 finding on GitHub"), "{text}");
    assert!(text.contains("Expire entries after an hour."), "the person's edit, not the reviewer's text");
    assert!(!text.contains("Pending one"), "a finding that was not posted is not sent");
    assert!(text.ends_with("The review is at https://github.com/octo/app/pull/7#pullrequestreview-1."), "{text}");
}

#[test]
fn nothing_to_send_sends_nothing() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Drafted);
    let id = drafted(&core, 7, &[]);
    assert_eq!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Empty);
    assert!(sender.sent.lock().unwrap().is_empty());
    assert!(job(&core, &id).handoff_note.is_none());
}

#[test]
fn no_chat_on_the_branch_is_noted() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Drafted);
    let id = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
    core.db.lock().unwrap().execute("UPDATE review_jobs SET pr_branch='feat/other' WHERE id=?1", params![id]).unwrap();
    let outcome = hand_off(&core, &id, Moment::Drafted, now()).unwrap();
    assert_eq!(outcome, Outcome::Held("Not sent: no chat in this project works on the branch feat/other.".into()));
    assert!(sender.sent.lock().unwrap().is_empty());
    assert_eq!(job(&core, &id).handoff_note.as_deref(), Some("Not sent: no chat in this project works on the branch feat/other."));
}

#[test]
fn a_fourth_handoff_in_a_day_is_held() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Drafted);
    for _ in 0..MAX_HANDOFFS_PER_DAY {
        let id = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
        assert!(matches!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Sent { .. }));
    }
    let fourth = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
    assert!(matches!(hand_off(&core, &fourth, Moment::Drafted, now()).unwrap(), Outcome::Held(note) if note.contains("already went to its chat today")));
    assert_eq!(sender.sent.lock().unwrap().len(), MAX_HANDOFFS_PER_DAY as usize);
    // Another PR is its own count, and a day later the PR's count is clear.
    let other = drafted(&core, 8, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
    assert!(matches!(hand_off(&core, &other, Moment::Drafted, now()).unwrap(), Outcome::Sent { .. }));
    let later = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
    assert!(matches!(hand_off(&core, &later, Moment::Drafted, now() + Duration::hours(25)).unwrap(), Outcome::Sent { .. }));
}

#[test]
fn a_job_hands_off_once_and_a_refusal_is_noted() {
    let (_scratch, core, sender) = fixture(ReviewHandoff::Drafted);
    let id = drafted(&core, 7, &[finding("src/a.rs", ReviewSeverity::Major, "Cache never expires")]);
    *sender.refuse.lock().unwrap() = Some("The chat is closed".into());
    assert_eq!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Held("Not sent to Add the cache: The chat is closed".into()));
    *sender.refuse.lock().unwrap() = None;
    assert!(matches!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Sent { .. }), "a refused handoff can go later");
    assert_eq!(hand_off(&core, &id, Moment::Drafted, now()).unwrap(), Outcome::Skipped, "but only once");
    assert_eq!(sender.sent.lock().unwrap().len(), 1);
    assert!(job(&core, &id).handoff_note.is_none(), "a sent review clears its note");
}
