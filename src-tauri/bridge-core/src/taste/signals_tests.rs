use super::*;
use crate::reviews::steps::{DraftFinding, PassOutcome};
use crate::reviews::store::{self, NewJob};
use bridge_protocol::messages::{
    self as wire, ReviewFindingDecision, ReviewJobStatus as Status, ReviewPostMode, ReviewSeverity, ReviewSide, ReviewTrigger,
};

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-23T10:00:00Z").unwrap().with_timezone(&Utc) + Duration::seconds(seconds)
}

fn draft(path: &str, title: &str, body: &str) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line: 4,
        start_line: None,
        side: ReviewSide::Right,
        severity: ReviewSeverity::Minor,
        title: title.into(),
        body: body.into(),
        confidence: 0.5,
        fingerprint: format!("fp-{path}"),
        outside_diff: false,
    }
}

/// A drafted review of `Octo/Repo#number` at `sha` by Claude, with the given
/// findings; returns the job and the finding ids in path order.
fn drafted(db: &Connection, number: u64, sha: &str, findings: Vec<DraftFinding>, fixed: Vec<String>) -> (String, Vec<String>) {
    let new = NewJob {
        workspace_id: "w1".into(),
        repo_host: Some("github.com".into()),
        repo_owner: "Octo".into(),
        repo_name: "Repo".into(),
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
    let (job, _) = store::insert(db, &new, at(0)).unwrap();
    let claimed = store::claim_next(db, "runner", at(0)).unwrap().unwrap();
    assert_eq!(claimed.id, job.id);
    let token = claimed.lease_owner.unwrap();
    assert!(store::advance(db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = store::begin_pass(db, &job.id, &token, 0, "claude", at(1)).unwrap().unwrap();
    let outcome = PassOutcome { findings, fixed, ..Default::default() };
    assert!(store::complete_pass(db, &job.id, &token, &pass, &outcome, at(2)).unwrap());
    assert!(store::finish_review(db, &job.id, &token, "summary", None, true).unwrap());
    let mut ids: Vec<(String, String)> =
        store::findings(db, &job.id).unwrap().into_iter().map(|finding| (finding.path, finding.id)).collect();
    ids.sort();
    (job.id, ids.into_iter().map(|(_, id)| id).collect())
}

fn decide(db: &Connection, job: &str, choices: &[(&str, ReviewFindingDecision, Option<&str>)]) -> Result<(), BridgeError> {
    let decisions: Vec<wire::ReviewFindingDecisionInput> = choices
        .iter()
        .map(|(id, decision, body)| wire::ReviewFindingDecisionInput {
            finding_id: id.to_string(),
            decision: *decision,
            edited_body: body.map(str::to_owned),
        })
        .collect();
    store::decide(db, job, &decisions, at(10))
}

fn count(db: &Connection) -> i64 {
    db.query_row("SELECT COUNT(*) FROM taste_signals", [], |row| row.get(0)).unwrap()
}

const SCOPE: &str = "project:octo/repo";

#[test]
fn the_log_exists_with_one_live_signal_per_subject() {
    let (_scratch, db) = open();
    let sql: String = db
        .query_row("SELECT sql FROM sqlite_master WHERE name='idx_taste_signals_live'", [], |row| row.get(0))
        .unwrap();
    assert!(sql.contains("UNIQUE") && sql.contains("superseded_by IS NULL"), "{sql}");
}

#[test]
fn deciding_findings_writes_one_signal_each_in_the_repository_scope() {
    let (_scratch, db) = open();
    let (job, ids) = drafted(
        &db,
        7,
        "sha1",
        vec![
            draft("src/a.rs", "Off-by-one in get", "Panics when index == len."),
            draft("src/b.ts", "Add a doc comment", "Private helper has no docs."),
            draft("tests/c.py", "No tests for the parser", "Mentions the off-by-one bug they would catch."),
        ],
        vec![],
    );
    decide(
        &db,
        &job,
        &[
            (&ids[0], ReviewFindingDecision::Kept, None),
            (&ids[1], ReviewFindingDecision::Dropped, None),
            (&ids[2], ReviewFindingDecision::Edited, Some("Add a test for index == len.")),
        ],
    )
    .unwrap();
    let mut signals = live(&db, SCOPE, 10).unwrap();
    signals.sort_by(|a, b| a.path.cmp(&b.path));
    let summary: Vec<_> = signals
        .iter()
        .map(|signal| (signal.verdict.as_str(), signal.language.as_deref(), signal.category.as_deref()))
        .collect();
    assert_eq!(
        summary,
        [("kept", Some("rust"), Some("correctness")), ("dropped", Some("typescript"), Some("docs")), ("edited", Some("python"), Some("tests"))]
    );
    let edited = &signals[2];
    assert_eq!(edited.edited_body.as_deref(), Some("Add a test for index == len."));
    assert_eq!(edited.source, SOURCE_REVIEW_DECISION);
    assert_eq!((edited.harness.as_deref(), edited.pr_number, edited.job_id.as_deref()), (Some("claude"), Some(7), Some(job.as_str())));
    assert_eq!(edited.subject_id, ids[2]);
    assert_eq!(signals[0].edited_body, None, "only an edit carries a body");
}

#[test]
fn a_changed_decision_supersedes_and_the_same_one_adds_nothing() {
    let (_scratch, db) = open();
    let (job, ids) = drafted(&db, 8, "sha1", vec![draft("src/a.rs", "Prefer matches!", "b")], vec![]);
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Kept, None)]).unwrap();
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Kept, None)]).unwrap();
    assert_eq!(count(&db), 1, "the same decision twice is one signal");
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Dropped, None)]).unwrap();
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Edited, Some("first"))]).unwrap();
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Edited, Some("second"))]).unwrap();
    let history = history(&db, SOURCE_REVIEW_DECISION, &ids[0]).unwrap();
    let verdicts: Vec<_> = history.iter().map(|signal| (signal.verdict.as_str(), signal.edited_body.as_deref())).collect();
    assert_eq!(verdicts, [("kept", None), ("dropped", None), ("edited", Some("first")), ("edited", Some("second"))]);
    for pair in history.windows(2) {
        assert_eq!(pair[0].superseded_by.as_deref(), Some(pair[1].id.as_str()), "each points at its successor");
    }
    assert_eq!(live(&db, SCOPE, 10).unwrap().len(), 1, "one live signal per finding");
}

#[test]
fn a_decide_that_fails_writes_no_signal() {
    let (_scratch, db) = open();
    let (job, ids) = drafted(&db, 9, "sha1", vec![draft("src/a.rs", "t", "b")], vec![]);
    let refused = decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Kept, None), ("missing", ReviewFindingDecision::Dropped, None)]);
    assert!(refused.is_err());
    assert_eq!(count(&db), 0, "the batch rolled back with its signals");
}

#[test]
fn a_secret_in_an_edit_is_stored_redacted() {
    let (_scratch, db) = open();
    let (job, ids) = drafted(&db, 10, "sha1", vec![draft("src/a.rs", "Hard-coded key", "b")], vec![]);
    let key = format!("sk-ant-{}", "a1".repeat(20));
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Edited, Some(&format!("Move {key} to the vault.")))]).unwrap();
    let signal = &live(&db, SCOPE, 1).unwrap()[0];
    let body = signal.edited_body.as_deref().unwrap();
    assert!(!body.contains(&key), "{body}");
    assert!(body.starts_with("Move ") && body.ends_with(" to the vault."), "{body}");
    assert_eq!(signal.category.as_deref(), Some("security"));
}

#[test]
fn backfill_copies_what_was_decided_before_the_log_once() {
    let (_scratch, db) = open();
    let (job, ids) = drafted(&db, 11, "sha1", vec![draft("src/a.rs", "t1", "b"), draft("src/b.rs", "t2", "b")], vec![]);
    // Decisions as they were stored before the log existed.
    db.execute("UPDATE review_findings SET decision='kept', decided_at='2026-11-20T00:00:00Z' WHERE id=?1", params![ids[0]]).unwrap();
    db.execute("UPDATE review_findings SET decision='edited', edited_body='better' WHERE id=?1", params![ids[1]]).unwrap();
    db.execute("UPDATE review_findings SET fixed_in_sha='sha9' WHERE id=?1", params![ids[0]]).unwrap();
    assert_eq!(count(&db), 0);
    assert_eq!(backfill(&db).unwrap(), 3, "two decisions and one fix");
    assert_eq!(backfill(&db).unwrap(), 0, "running again adds nothing");
    let kept = history(&db, SOURCE_REVIEW_DECISION, &ids[0]).unwrap();
    assert_eq!((kept[0].verdict.as_str(), kept[0].at.as_str()), ("kept", "2026-11-20T00:00:00.000Z"), "dated when it was decided");
    let fixed = history(&db, SOURCE_REVIEW_FIXED, &ids[0]).unwrap();
    assert_eq!(fixed[0].detail, Some(serde_json::json!({"fixedInSha": "sha9"})));
    assert_eq!(history(&db, SOURCE_REVIEW_DECISION, &ids[1]).unwrap()[0].edited_body.as_deref(), Some("better"));
    // A live decision after the backfill is not a duplicate.
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Kept, None)]).unwrap();
    assert_eq!(history(&db, SOURCE_REVIEW_DECISION, &ids[0]).unwrap().len(), 1);
}

#[test]
fn a_re_review_that_fixes_a_kept_finding_records_the_fix() {
    let (_scratch, db) = open();
    let (first, ids) = drafted(&db, 12, "sha1", vec![draft("src/a.rs", "Divide by zero", "b")], vec![]);
    decide(&db, &first, &[(&ids[0], ReviewFindingDecision::Kept, None)]).unwrap();
    drafted(&db, 12, "sha2", vec![], vec![ids[0].clone()]);
    let fixed = history(&db, SOURCE_REVIEW_FIXED, &ids[0]).unwrap();
    assert_eq!(fixed.len(), 1);
    assert_eq!((fixed[0].verdict.as_str(), fixed[0].scope_key.as_str()), ("fixed", SCOPE));
    assert_eq!(fixed[0].detail, Some(serde_json::json!({"fixedInSha": "sha2"})));
}

#[test]
fn thread_outcomes_are_resolved_or_ignored_and_nothing_else() {
    assert_eq!(thread_outcome(true, false, 3, false, false), Some(ThreadOutcome::Resolved));
    assert_eq!(thread_outcome(true, true, 0, true, true), Some(ThreadOutcome::Resolved), "resolved wins");
    assert_eq!(thread_outcome(false, true, 0, false, false), Some(ThreadOutcome::Ignored));
    assert_eq!(thread_outcome(false, true, 1, false, false), None, "a reply is not ignoring it");
    assert_eq!(thread_outcome(false, true, 0, true, false), None, "fixed is its own signal");
    assert_eq!(thread_outcome(false, true, 0, false, true), None, "dropped already says it");
    assert_eq!(thread_outcome(false, false, 0, false, false), None, "an open thread says nothing yet");

    let (_scratch, db) = open();
    let (_, ids) = drafted(&db, 13, "sha1", vec![draft("src/a.rs", "t", "b")], vec![]);
    let at = at(20).to_rfc3339();
    assert!(record_thread(&db, &ids[0], ThreadOutcome::Ignored, &at).unwrap().is_some());
    assert!(record_thread(&db, &ids[0], ThreadOutcome::Ignored, &at).unwrap().is_none(), "seen again on the next re-review");
    assert!(record_thread(&db, &ids[0], ThreadOutcome::Resolved, &at).unwrap().is_some(), "then resolved");
    let history = history(&db, SOURCE_REVIEW_THREAD, &ids[0]).unwrap();
    assert_eq!(history.iter().map(|signal| signal.verdict.as_str()).collect::<Vec<_>>(), ["ignored", "resolved"]);
}

fn chat(db: &Connection, id: &str, workspace: Option<&str>) {
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES(?1,?2,'codex','Chat','idle','estimated','direct')",
        params![id, workspace],
    )
    .unwrap();
}

#[test]
fn permission_decisions_are_allowed_or_declined_in_the_chats_scope() {
    let (_scratch, db) = open();
    db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','D','/tmp/taste','now')", []).unwrap();
    for workspace in ["w1", "w2"] {
        db.execute(
            "INSERT INTO workspaces(id,project_id,title,path,status,created_at) VALUES(?1,'p','T',?2,'idle','now')",
            params![workspace, format!("/tmp/taste-{workspace}")],
        )
        .unwrap();
    }
    drafted(&db, 14, "sha1", vec![], vec![]); // names w1's repository
    chat(&db, "reviewed", Some("w1"));
    chat(&db, "unreviewed", Some("w2"));
    chat(&db, "direct", None);
    let key = format!("sk-ant-{}", "b2".repeat(20));
    let bash = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": format!("ANTHROPIC_API_KEY={key} cargo test")}});
    let edit = serde_json::json!({"tool_name": "Edit", "tool_input": {"file_path": "src/lib.rs", "old_string": "a"}});
    let at = at(30).to_rfc3339();

    assert!(record_permission(&db, "reviewed", 5, "codex", &bash, "acceptForSession", &at).unwrap().is_some());
    assert!(record_permission(&db, "unreviewed", 6, "claude", &edit, "decline", &at).unwrap().is_some());
    assert!(record_permission(&db, "direct", 7, "claude", &bash, "accept", &at).unwrap().is_some());
    assert!(record_permission(&db, "direct", 8, "claude", &bash, "cancel", &at).unwrap().is_none(), "a cancel says nothing");
    assert!(record_permission(&db, "direct", 7, "claude", &bash, "accept", &at).unwrap().is_none(), "once per request");

    let reviewed = &live(&db, SCOPE, 5).unwrap()[0];
    assert_eq!((reviewed.verdict.as_str(), reviewed.title.as_deref(), reviewed.subject_id.as_str()), ("allowed", Some("Bash"), "reviewed:5"));
    let command = reviewed.body.as_deref().unwrap();
    assert!(command.ends_with(" cargo test") && !command.contains(&key), "{command}");
    assert_eq!(reviewed.detail, Some(serde_json::json!({"decision": "acceptForSession", "forSession": true})));
    let declined = &live(&db, "workspace:w2", 5).unwrap()[0];
    assert_eq!((declined.verdict.as_str(), declined.path.as_deref(), declined.language.as_deref()), ("declined", Some("src/lib.rs"), Some("rust")));
    assert_eq!(live(&db, ACCOUNT_SCOPE, 5).unwrap().len(), 1, "a chat with no project counts for the account");
}

const ACCOUNT_SCOPE: &str = bridge_protocol::messages::ACCOUNT_MEMORY_SCOPE;

#[test]
fn signals_older_than_a_year_are_swept() {
    let (_scratch, db) = open();
    let (job, ids) = drafted(&db, 15, "sha1", vec![draft("src/a.rs", "t", "b"), draft("src/b.rs", "t", "b")], vec![]);
    decide(&db, &job, &[(&ids[0], ReviewFindingDecision::Kept, None), (&ids[1], ReviewFindingDecision::Dropped, None)]).unwrap();
    db.execute("UPDATE taste_signals SET at='2025-01-01T00:00:00.000Z' WHERE subject_id=?1", params![ids[0]]).unwrap();
    assert_eq!(sweep(&db, at(0)).unwrap(), 1);
    assert_eq!(live(&db, SCOPE, 10).unwrap().iter().map(|signal| signal.subject_id.clone()).collect::<Vec<_>>(), [ids[1].clone()]);
}

#[test]
fn categories_read_the_title_first() {
    for (title, body, category) in [
        ("SQL injection in search", "", "security"),
        ("Hard-coded API key", "", "security"),
        ("Hard-coded timeout", "Use a named constant.", "other"),
        ("Token logged in plain text", "This leaks a secret to the logs.", "security"),
        ("Off-by-one in get", "", "correctness"),
        ("Race between save and load", "", "correctness"),
        ("unwrap() on user input", "", "error-handling"),
        ("No tests for the new functions", "A test would catch the off-by-one bug.", "tests"),
        ("Quadratic loop over findings", "", "performance"),
        ("Sum_All breaks naming conventions", "", "naming"),
        ("Add a doc comment", "", "docs"),
        ("Simplify this match", "", "style"),
        ("Consider this", "The PR author should check the bounds here.", "correctness"),
        ("Consider this", "Looks odd.", "other"),
    ] {
        assert_eq!(category_of(title, body), category, "{title} / {body}");
    }
    assert_eq!(category_of("Ask the author", ""), "other", "author is not auth");
}

#[test]
fn languages_come_from_the_extension() {
    assert_eq!(language_of("src/a.rs"), Some("rust"));
    assert_eq!(language_of("web/App.TSX"), Some("typescript"));
    assert_eq!(language_of("Makefile"), None);
    assert_eq!(language_of("notes.unknown"), None);
}

#[test]
fn a_person_declining_a_tool_request_in_a_chat_is_logged() {
    let scratch = tempfile::tempdir().unwrap();
    let core = std::sync::Arc::new(crate::runtime::BridgeCore::for_tests(scratch.path()));
    let sequence = {
        let db = core.db.lock().unwrap();
        chat(&db, "chat", None);
        let mut event = crate::agent::NormalizedEvent::new("permission.requested");
        event.data = serde_json::json!({"requestId": 1, "tool_name": "Bash", "tool_input": {"command": "git push --force"}});
        crate::store::session_event(&db, "chat", &event, &serde_json::json!({"adapter": "codex"})).unwrap().sequence
    };
    // No harness is running to receive the answer; the decision is still the
    // person's, and it is logged when it is claimed.
    if let Err(error) = crate::api::resolve_approval(&core, "chat", sequence, "decline", None) {
        eprintln!("no harness to answer, as expected: {error}");
    }
    let db = core.db.lock().unwrap();
    let signals = live(&db, ACCOUNT_SCOPE, 5).unwrap();
    assert_eq!(signals.len(), 1, "{signals:?}");
    assert_eq!((signals[0].verdict.as_str(), signals[0].title.as_deref(), signals[0].body.as_deref()), ("declined", Some("Bash"), Some("git push --force")));
    assert_eq!(signals[0].subject_id, format!("chat:{sequence}"));
}

#[test]
fn every_instant_in_the_log_has_one_format() {
    assert_eq!(instant("2026-11-23T10:00:00+00:00"), "2026-11-23T10:00:00.000Z");
    assert_eq!(instant("2026-11-23T15:30:00+05:30"), "2026-11-23T10:00:00.000Z");
    assert_eq!(instant("2026-11-23T10:00:00.250Z"), "2026-11-23T10:00:00.250Z");
    assert_eq!(instant("now"), "now", "not an instant: kept as given");
    let (_scratch, db) = open();
    chat(&db, "direct", None);
    let request = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "ls"}});
    record_permission(&db, "direct", 1, "claude", &request, "accept", "2026-11-23T15:30:00+05:30").unwrap();
    assert_eq!(live(&db, ACCOUNT_SCOPE, 1).unwrap()[0].at, "2026-11-23T10:00:00.000Z");
}
