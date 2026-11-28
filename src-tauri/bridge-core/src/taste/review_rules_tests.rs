use super::*;
use rusqlite::params;

const SCOPE: &str = "project:octo/repo";

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

fn rule(db: &Connection, scope: &str, body: &str, globs: Option<&[&str]>, score: f64, status: &str) -> String {
    let globs: Option<Vec<String>> = globs.map(|globs| globs.iter().map(|glob| glob.to_string()).collect());
    let record = crate::memory_ledger::save_taste_rule(db, scope, "review_avoid", body, globs.as_deref()).unwrap();
    db.execute("UPDATE memory_records SET outcome_score=?2, status=?3 WHERE id=?1", params![record.id, score, status]).unwrap();
    record.id
}

fn bodies(rules: &[MemoryRecord]) -> Vec<&str> {
    rules.iter().map(|rule| rule.body.as_str()).collect()
}

#[test]
fn a_review_gets_its_repositorys_rules_that_fit_its_paths_the_specific_first() {
    let (_scratch, db) = open();
    rule(&db, SCOPE, "Everywhere, trusted", None, 3.0, "active");
    rule(&db, SCOPE, "Everywhere, new", None, 0.0, "active");
    rule(&db, SCOPE, "Only in src", Some(&["src/**"]), 0.0, "active");
    rule(&db, SCOPE, "Only in web", Some(&["web/**"]), 9.0, "active");
    rule(&db, "account:local", "Every repository", None, 1.0, "active");
    rule(&db, "project:octo/other", "Another repository", None, 9.0, "active");
    rule(&db, SCOPE, "Waiting for approval", None, 9.0, "proposed");
    let chosen = for_review(&db, SCOPE, &["src/lib.rs".into(), "README.md".into()]).unwrap();
    assert_eq!(bodies(&chosen), ["Only in src", "Everywhere, trusted", "Every repository", "Everywhere, new"]);
    assert!(for_review(&db, SCOPE, &["./web/app.ts".into()]).unwrap().iter().all(|rule| rule.body != "Only in src"));
    assert_eq!(bodies(&for_review(&db, SCOPE, &["web/app.ts".into()]).unwrap())[0], "Only in web");
}

#[test]
fn at_most_twelve_rules_and_eight_thousand_characters_go_to_a_review() {
    let (_scratch, db) = open();
    for index in 0..20 {
        rule(&db, SCOPE, &format!("Short rule {index}"), None, f64::from(index), "active");
    }
    let chosen = for_review(&db, SCOPE, &[]).unwrap();
    assert_eq!(chosen.len(), MAX_RULES_PER_REVIEW);
    assert_eq!(chosen[0].body, "Short rule 19", "the most trusted first");

    let (_scratch, db) = open();
    for index in 0..3 {
        rule(&db, SCOPE, &format!("{index} {}", "long ".repeat(700)), None, 0.0, "active");
    }
    assert_eq!(for_review(&db, SCOPE, &[]).unwrap().len(), 2, "the third would pass 8000 characters");
}

#[test]
fn the_context_file_names_each_rule_and_a_pass_reads_back_the_ids_it_was_given() {
    let (scratch, db) = open();
    let id = rule(&db, SCOPE, "No unwrap outside tests", Some(&["src/**"]), 0.0, "active");
    let chosen = for_review(&db, SCOPE, &["src/a.rs".into()]).unwrap();
    let json = context_json(&chosen);
    assert_eq!(json, serde_json::json!([{"id": id, "kind": "review_avoid", "rule": "No unwrap outside tests", "pathGlobs": ["src/**"]}]));
    std::fs::write(scratch.path().join(TASTE_FILE), serde_json::to_vec(&json).unwrap()).unwrap();
    assert_eq!(given(scratch.path()), [id]);
    assert!(given(&scratch.path().join("nowhere")).is_empty());
    std::fs::write(scratch.path().join(TASTE_FILE), b"not json").unwrap();
    assert!(given(scratch.path()).is_empty(), "an unreadable file gives nothing");
}

#[test]
fn the_prompt_names_the_rules_only_when_there_are_some() {
    let pr = serde_json::json!({"number": 7, "title": "Add get", "changedFiles": 1});
    let plain = crate::reviews::session::prompt(&pr, ".bridge-review-x");
    let mut none = pr.clone();
    none["tasteRules"] = serde_json::json!(0);
    assert_eq!(crate::reviews::session::prompt(&none, ".bridge-review-x"), plain, "no rules, the same prompt");
    let mut some = pr;
    some["tasteRules"] = serde_json::json!(2);
    let with = crate::reviews::session::prompt(&some, ".bridge-review-x");
    let line = prompt_line(".bridge-review-x");
    assert!(with.contains(&line), "{with}");
    assert!(with.find(&line).unwrap() < with.find("Read the diff").unwrap(), "listed with the other context files");
    assert_eq!(with.replace(&format!("\n{line}"), ""), plain);
}

#[test]
fn a_findings_rules_and_a_passs_rules_are_stored_and_read_back() {
    use crate::reviews::steps::PassOutcome;
    use crate::reviews::store::{self, NewJob};
    use bridge_protocol::messages::{ReviewJobStatus as Status, ReviewPostMode, ReviewSeverity, ReviewTrigger};
    let (_scratch, db) = open();
    let at = crate::taste::replay::fixture::day(1.0);
    let new = NewJob {
        workspace_id: "w".into(), repo_host: Some("github.com".into()), repo_owner: "octo".into(), repo_name: "repo".into(),
        number: 1, head_sha: "h".into(), base_sha: None, incremental_from_sha: None, trigger: ReviewTrigger::Manual,
        post_mode: ReviewPostMode::Draft, harnesses: vec!["claude".into()], retry_of: None, publication_key: None,
    };
    let (job, _) = store::insert(&db, &new, at).unwrap();
    let token = store::claim_next(&db, "t", at).unwrap().unwrap().lease_owner.unwrap();
    assert!(store::advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
    let pass = store::begin_pass(&db, &job.id, &token, 0, "claude", at).unwrap().unwrap();
    let mut shaped = crate::taste::replay::fixture::finding("src/a.rs", ReviewSeverity::Major, "Unwrap on input");
    shaped.rule_ids = vec!["rule-1".into()];
    let plain = crate::taste::replay::fixture::finding("src/b.rs", ReviewSeverity::Minor, "Unclear name");
    let outcome = PassOutcome { findings: vec![shaped, plain], taste_rule_ids: vec!["rule-1".into(), "rule-2".into()], ..PassOutcome::default() };
    assert!(store::complete_pass(&db, &job.id, &token, &pass, &outcome, at).unwrap());
    let mut found: Vec<(String, Vec<String>)> = store::findings(&db, &job.id).unwrap().into_iter().map(|finding| (finding.title, finding.rule_ids)).collect();
    found.sort();
    assert_eq!(found, [("Unclear name".to_string(), Vec::new()), ("Unwrap on input".to_string(), vec!["rule-1".to_string()])]);
    assert_eq!(store::passes(&db, &job.id).unwrap()[0].taste_rule_ids, ["rule-1", "rule-2"]);
}
