use super::*;
use crate::taste::replay::fixture;
use bridge_protocol::messages::{ReviewFindingDecision as Decision, ReviewSeverity};

const SCOPE: &str = "project:octo/repo";

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

/// An approved learned rule of `kind`, created at `at`.
fn learned(db: &Connection, kind: &str, body: &str, at: DateTime<Utc>, evidence: &[String]) -> String {
    let weights: Vec<(String, f64)> = evidence.iter().map(|id| (id.clone(), 1.0)).collect();
    let id = crate::memory_ledger::insert_taste_proposal(db, SCOPE, kind, body, None, None, &weights, &at.to_rfc3339()).unwrap();
    crate::memory_ledger::review_batch(db, SCOPE, &[id.clone()], &[]).unwrap();
    db.execute("UPDATE memory_records SET created_at=?2, valid_from=?2 WHERE id=?1", params![id, at.to_rfc3339()]).unwrap();
    id
}

/// A review on `day` with one finding citing `rules`, decided `decision`.
fn shaped(db: &Connection, number: u64, day: f64, rules: &[&str], decision: Decision) -> String {
    let mut finding = fixture::finding("src/a.rs", ReviewSeverity::Minor, &format!("Finding {number}"));
    finding.rule_ids = rules.iter().map(|rule| rule.to_string()).collect();
    let (job, ids) = fixture::review(db, "repo", number, &format!("sha-{number}"), fixture::day(day), vec![finding], vec![]);
    let edit = (decision == Decision::Edited).then_some("Reworded by the person.");
    fixture::decide(db, &job, fixture::day(day) + Duration::hours(1), &[(ids[0].as_str(), decision, edit)]);
    ids[0].clone()
}

fn status(db: &Connection, id: &str) -> (String, Option<String>, f64) {
    db.query_row("SELECT status, paused_reason, outcome_score FROM memory_records WHERE id=?1", params![id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })
    .unwrap()
}

#[test]
fn trust_rises_with_what_a_rule_shaped_and_kept_and_halves_every_sixty_days() {
    let (_scratch, db) = open();
    let rule = learned(&db, "review_prefer", "Flag bounds checks that use <=", fixture::day(0.0), &[]);
    shaped(&db, 1, 1.0, &[&rule], Decision::Kept);
    shaped(&db, 2, 2.0, &[&rule], Decision::Edited);
    shaped(&db, 3, 3.0, &[&rule], Decision::Dropped);
    let events = events(&db).unwrap();
    let deltas: Vec<f64> = events.iter().filter(|event| event.rule_id == rule).map(|event| event.delta).collect();
    assert_eq!(deltas, [1.0, 1.0, -1.0]);
    let now = fixture::day(3.0) + Duration::hours(1);
    recompute(&db, now).unwrap();
    let (state, _, value) = status(&db, &rule);
    assert_eq!(state, "active");
    assert!((value - 1.0).abs() < 0.05, "about +1 right after: {value}");
    let one: Vec<&Event> = vec![&events[0]];
    let later = events[0].at + Duration::days(60);
    assert!((score(&one, later) - 0.5).abs() < 1e-9, "half after sixty days");
}

#[test]
fn a_learned_rule_that_keeps_getting_dropped_pauses_itself_and_yours_never_do() {
    let (_scratch, db) = open();
    let learned_rule = learned(&db, "review_prefer", "Prefer early returns", fixture::day(0.0), &[]);
    let pinned_rule = learned(&db, "review_prefer", "Prefer small functions", fixture::day(0.0), &[]);
    set_state(&db, &pinned_rule, None, Some(true)).unwrap();
    let yours = crate::memory_ledger::save_taste_rule(&db, SCOPE, "review_prefer", "Flag magic numbers", None).unwrap().id;
    for (number, day) in [(1, 1.0), (2, 2.0), (3, 3.0)] {
        shaped(&db, number, day, &[&learned_rule, &pinned_rule, &yours], Decision::Dropped);
    }
    let done = recompute(&db, fixture::day(3.5)).unwrap();
    assert_eq!(done.paused, [learned_rule.clone()]);
    let (state, reason, value) = status(&db, &learned_rule);
    assert_eq!(state, "paused");
    assert!(value <= PAUSE_SCORE + 0.05, "{value}");
    assert!(reason.unwrap().starts_with("Paused:"));
    assert_eq!(status(&db, &pinned_rule).0, "active", "pinned");
    assert_eq!(status(&db, &yours).0, "active", "written by you");
    assert!(crate::taste::review_rules::for_review(&db, SCOPE, &[]).unwrap().iter().all(|rule| rule.id != learned_rule), "a paused rule is not given");
}

#[test]
fn mostly_against_lately_is_noisy_even_before_the_score_falls_far() {
    let (_scratch, db) = open();
    let rule = learned(&db, "review_prefer", "Prefer early returns", fixture::day(0.0), &[]);
    shaped(&db, 1, 1.0, &[&rule], Decision::Kept);
    shaped(&db, 2, 2.0, &[&rule], Decision::Dropped);
    shaped(&db, 3, 3.0, &[&rule], Decision::Dropped);
    recompute(&db, fixture::day(3.5)).unwrap();
    let (state, reason, value) = status(&db, &rule);
    assert!(value > PAUSE_SCORE, "{value}");
    assert_eq!(state, "paused");
    assert_eq!(reason.as_deref(), Some("Paused: 2 of its last 3 uses went against it."));
}

#[test]
fn a_learned_rule_nothing_confirms_for_ninety_days_expires() {
    let (_scratch, db) = open();
    let quiet = learned(&db, "code_prefer", "No unwrap outside tests", fixture::day(0.0), &[]);
    let confirmed = learned(&db, "review_prefer", "Flag bounds checks", fixture::day(0.0), &[]);
    shaped(&db, 1, 80.0, &[&confirmed], Decision::Kept);
    let done = recompute(&db, fixture::day(95.0)).unwrap();
    assert_eq!(done.expired, [quiet.clone()]);
    assert_eq!(status(&db, &quiet).0, "expired");
    assert_eq!(status(&db, &confirmed).0, "active", "kept at day 80");
}

#[test]
fn an_avoid_rule_earns_on_posted_reviews_and_loses_when_you_raise_what_it_avoids() {
    let (_scratch, db) = open();
    let built = fixture::build(&db);
    let nit_signals: Vec<String> = built
        .nit
        .iter()
        .flat_map(|id| crate::taste::signals::history(&db, crate::taste::signals::SOURCE_REVIEW_DECISION, id).unwrap())
        .map(|signal| signal.id)
        .collect();
    let rule = learned(&db, "review_avoid", "Skip doc comment nits on helper", fixture::day(5.5), &nit_signals);
    let post = |job: &str, at: f64| {
        db.execute("UPDATE review_jobs SET status='posted', finished_at=?2 WHERE id=?1", params![job, fixture::day(at).to_rfc3339()]).unwrap();
        db.execute("UPDATE review_passes SET taste_rule_ids=?2 WHERE job_id=?1", params![job, serde_json::json!([rule]).to_string()]).unwrap();
    };
    let (quiet, _) = fixture::review(&db, "repo", 301, "sha-301", fixture::day(6.0), vec![fixture::finding("src/x.rs", ReviewSeverity::Major, "Off-by-one in `x`")], vec![]);
    post(&quiet, 6.1);
    let (loud, ids) = fixture::review(&db, "repo", 302, "sha-302", fixture::day(7.0), vec![fixture::finding("src/helper.rs", ReviewSeverity::Nit, fixture::NIT)], vec![]);
    fixture::decide(&db, &loud, fixture::day(7.05), &[(ids[0].as_str(), Decision::Kept, None)]);
    post(&loud, 7.1);
    let deltas: Vec<f64> = events(&db).unwrap().into_iter().filter(|event| event.rule_id == rule).map(|event| event.delta).collect();
    assert_eq!(deltas.iter().filter(|delta| **delta == 0.5).count(), 1, "{deltas:?}");
    assert!(deltas.contains(&-1.0), "raised by hand: {deltas:?}");
}

#[test]
fn keeping_a_finding_the_rules_evidence_hid_costs_it_two() {
    let (_scratch, db) = open();
    let built = fixture::build(&db);
    let signals: Vec<String> = built
        .nit
        .iter()
        .flat_map(|id| crate::taste::signals::history(&db, crate::taste::signals::SOURCE_REVIEW_DECISION, id).unwrap())
        .map(|signal| signal.id)
        .collect();
    let rule = learned(&db, "review_avoid", "Skip doc comment nits on helper", fixture::day(5.5), &signals);
    let (job, ids) = fixture::review(&db, "repo", 106, "sha-106", fixture::day(6.0), vec![fixture::finding("src/helper.rs", ReviewSeverity::Nit, fixture::NIT)], vec![]);
    let hidden: Option<String> = db.query_row("SELECT hidden_reason FROM review_findings WHERE id=?1", params![ids[0]], |row| row.get(0)).unwrap();
    assert!(hidden.is_some(), "suppression hid it");
    fixture::decide(&db, &job, fixture::day(6.1), &[(ids[0].as_str(), Decision::Kept, None)]);
    let deltas: Vec<f64> = events(&db).unwrap().into_iter().filter(|event| event.rule_id == rule).map(|event| event.delta).collect();
    assert!(deltas.contains(&-2.0), "{deltas:?}");
}

#[test]
fn a_rule_given_to_ten_reviews_and_cited_by_none_is_never_used_unless_it_avoids() {
    let (_scratch, db) = open();
    let prefer = learned(&db, "review_prefer", "Flag bounds checks", fixture::day(0.0), &[]);
    let avoid = learned(&db, "review_avoid", "Skip naming nits", fixture::day(0.0), &[]);
    for number in 1..=10 {
        let (job, _) = fixture::review(&db, "repo", number, &format!("sha-{number}"), fixture::day(f64::from(number as u32)), vec![], vec![]);
        db.execute("UPDATE review_passes SET taste_rule_ids=?2 WHERE job_id=?1", params![job, serde_json::json!([prefer, avoid]).to_string()]).unwrap();
    }
    let unused = never_used(&db).unwrap();
    assert!(unused.contains(&prefer) && !unused.contains(&avoid));
    shaped(&db, 50, 20.0, &[&prefer], Decision::Kept);
    assert!(!never_used(&db).unwrap().contains(&prefer), "cited once");
}

#[test]
fn you_can_pause_resume_and_pin_a_rule() {
    let (_scratch, db) = open();
    let rule = learned(&db, "code_prefer", "No unwrap outside tests", fixture::day(0.0), &[]);
    set_state(&db, &rule, Some("paused"), None).unwrap();
    assert_eq!(status(&db, &rule).0, "paused");
    assert_eq!(status(&db, &rule).1.as_deref(), Some("Paused by you."));
    assert_eq!(crate::memory_ledger::list_taste(&db, Some("paused")).unwrap().len(), 1);
    set_state(&db, &rule, Some("active"), Some(true)).unwrap();
    let record = crate::memory_ledger::load_record(&db, &rule).unwrap().unwrap();
    assert_eq!((record.status.as_str(), record.pinned, record.paused_reason), ("active", true, None));
    assert!(set_state(&db, "missing", Some("paused"), None).is_err());
    crate::memory_ledger::review_batch(&db, SCOPE, &[], &[]).unwrap();
    let rejected = crate::memory_ledger::insert_taste_proposal(&db, SCOPE, "code_prefer", "Prefer tabs", None, None, &[], "2026-10-01T00:00:00Z").unwrap();
    crate::memory_ledger::review_batch(&db, SCOPE, &[], &[rejected.clone()]).unwrap();
    assert!(set_state(&db, &rejected, Some("active"), None).is_err(), "a rejected rule is not resumed");
}
