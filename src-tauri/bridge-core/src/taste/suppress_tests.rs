use super::*;
use crate::reviews::store;
use crate::taste::replay::{self, fixture, Outcome, RepeatedDrops};
use bridge_protocol::messages::{ReviewFindingDecision as Decision, ReviewPostMode, ReviewSeverity};

fn open() -> (tempfile::TempDir, Connection) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    (scratch, db)
}

fn candidate(path: &str, severity: &str, title: &str) -> Candidate {
    Candidate {
        finding_id: "new".into(),
        job_id: "job".into(),
        scope_key: "project:octo/repo".into(),
        pr_number: 200,
        reviewed_at: "2026-11-01T00:00:00.000Z".into(),
        path: path.into(),
        severity: severity.into(),
        category: signals::category_of(title, "").into(),
        title: title.into(),
        body: String::new(),
        fingerprint: "fp".into(),
        outcome: Outcome { decision: "pending", posted: false, fixed: false },
    }
}

fn signal(id: &str, verdict: &str, pr: u64, day: u32, path: &str, title: &str) -> Signal {
    Signal {
        id: id.into(),
        at: format!("2026-10-{day:02}T00:00:00.000Z"),
        scope_key: "project:octo/repo".into(),
        source: signals::SOURCE_REVIEW_DECISION.into(),
        verdict: verdict.into(),
        subject_id: id.into(),
        harness: Some("claude".into()),
        path: Some(path.into()),
        language: signals::language_of(path).map(str::to_owned),
        severity: Some("nit".into()),
        category: Some(signals::category_of(title, "").into()),
        title: Some(title.into()),
        body: None,
        edited_body: None,
        fingerprint: None,
        job_id: None,
        pr_number: Some(pr),
        session_id: None,
        detail: None,
        superseded_by: None,
    }
}

fn hides(past: &[Signal], candidate: &Candidate) -> bool {
    matches!(Suppressor::default().judge(past, candidate), Judgment::Hide { .. })
}

#[test]
fn alike_needs_the_same_kind_language_and_close_titles_or_shared_code() {
    let nit = candidate("src/helper.rs", "nit", fixture::NIT);
    let past = |path: &str, title: &str| signal("s", "dropped", 1, 1, path, title);
    assert!(alike(&nit, &past("src/other.rs", "Missing doc comment on `helper`")), "the same ask, worded apart");
    assert!(alike(&nit, &past("src/parse.rs", "Add a doc comment to `parse`")), "the same ask about another function");
    assert!(!alike(&nit, &past("src/api.rs", fixture::NEAR_NIT)), "a doc comment on a public type is another matter");
    assert!(!alike(&nit, &past("web/helper.ts", fixture::NIT)), "another language");
    assert!(!alike(&nit, &past("src/helper.rs", "Rename `helper` to `format_title`")), "another kind");
    let unwrap = candidate("src/a.rs", "major", "Avoid `unwrap()` on `config.get(key)`");
    let mut shared = past("src/b.rs", "Handle the error from `config.get(key)`");
    shared.category = Some("error-handling".into());
    assert!(alike(&unwrap, &shared), "related titles quoting the same code");
}

#[test]
fn the_fixture_hides_what_was_always_dropped_and_nothing_that_was_wanted() {
    let (_scratch, db) = open();
    let built = fixture::build(&db);
    let report = replay::replay(&db, &Suppressor::default()).unwrap();
    let hidden = |ids: &[String]| -> Vec<bool> {
        ids.iter().map(|id| report.calls.iter().find(|(call, _)| call == id).unwrap().1).collect()
    };
    assert_eq!(hidden(&built.nit), [false, false, false, true, true]);
    assert_eq!(hidden(&built.critical), [false; 4]);
    assert_eq!(hidden(&[built.near_nit.clone(), built.other_repo_nit.clone(), built.off_by_one.clone()]), [false; 3]);
    assert!(report.wrongly_hidden.is_empty() && report.critical_hidden == 0);
    let reference = replay::replay(&db, &RepeatedDrops::default()).unwrap();
    assert_eq!(report.calls, reference.calls, "on the fixture, as the exact-title reference");
}

#[test]
fn wording_that_changes_between_reviews_is_still_recognised() {
    let titles = ["Add a doc comment to `helper`", "Missing doc comment on `helper`", "Add doc comment for `helper`"];
    let past: Vec<Signal> =
        titles.iter().enumerate().map(|(index, title)| signal(&format!("d{index}"), "dropped", index as u64 + 1, index as u32 + 1, "src/helper.rs", title)).collect();
    assert!(hides(&past, &candidate("src/helper.rs", "nit", "Doc comment missing on `helper`")));
    let Judgment::Hide { reason, evidence } = Suppressor::default().judge(&past, &candidate("src/helper.rs", "nit", "Missing doc comment for `helper`")) else {
        panic!("hidden")
    };
    assert_eq!(reason, "You dropped 3 like this, last on #3.");
    assert_eq!(evidence, ["d0", "d1", "d2"]);
}

#[test]
fn the_floor_needs_three_drops_on_two_prs_and_never_hides_critical_or_security() {
    let nit = candidate("src/helper.rs", "nit", fixture::NIT);
    let one_pr: Vec<Signal> = (0..4).map(|day| signal(&format!("d{day}"), "dropped", 7, day + 1, "src/helper.rs", fixture::NIT)).collect();
    assert!(!hides(&one_pr, &nit), "four drops on one PR is one PR's taste");
    let two: Vec<Signal> = (0..2).map(|day| signal(&format!("d{day}"), "dropped", u64::from(day), day + 1, "src/helper.rs", fixture::NIT)).collect();
    assert!(!hides(&two, &nit), "two drops is not yet a pattern");
    let three: Vec<Signal> = (0..3).map(|day| signal(&format!("d{day}"), "dropped", u64::from(day), day + 1, "src/x.rs", "SQL injection in `search`")).collect();
    assert!(!hides(&three, &candidate("src/x.rs", "nit", "SQL injection in `search`")), "security, whatever the severity");
    let critical: Vec<Signal> = (0..3).map(|day| signal(&format!("d{day}"), "dropped", u64::from(day), day + 1, "src/x.rs", "Off-by-one in `get`")).collect();
    assert!(!hides(&critical, &candidate("src/x.rs", "critical", "Off-by-one in `get`")));
    assert!(hides(&critical, &candidate("src/x.rs", "major", "Off-by-one in `get`")), "the same at major is hidden");
}

#[test]
fn a_keep_since_the_last_drop_wins_and_a_kept_hidden_one_counts_double() {
    let nit = candidate("src/helper.rs", "nit", fixture::NIT);
    let mut past: Vec<Signal> = (0..3).map(|day| signal(&format!("d{day}"), "dropped", u64::from(day), day + 1, "src/helper.rs", fixture::NIT)).collect();
    past.push(signal("k", "kept", 9, 5, "src/helper.rs", fixture::NIT));
    assert!(!hides(&past, &nit), "kept after the last drop");

    // drop, keep, keep (after a hide), drop, drop: three against, and the
    // kept hidden one counts twice, so three for.
    let mut back = vec![
        signal("a", "dropped", 1, 1, "src/helper.rs", fixture::NIT),
        signal("b", "kept", 2, 2, "src/helper.rs", fixture::NIT),
        signal("c", "kept", 3, 3, "src/helper.rs", fixture::NIT),
        signal("d", "dropped", 4, 4, "src/helper.rs", fixture::NIT),
        signal("e", "dropped", 5, 5, "src/helper.rs", fixture::NIT),
    ];
    assert!(hides(&back, &nit), "three against two");
    back[2].detail = Some(serde_json::json!({"wasHidden": true}));
    assert!(!hides(&back, &nit), "three against three");
}

#[test]
fn an_ignored_thread_counts_half_a_drop() {
    let nit = candidate("src/helper.rs", "nit", fixture::NIT);
    let mut past = vec![
        signal("a", "dropped", 1, 1, "src/helper.rs", fixture::NIT),
        signal("b", "dropped", 2, 2, "src/helper.rs", fixture::NIT),
        signal("c", "ignored", 3, 3, "src/helper.rs", fixture::NIT),
    ];
    assert!(!hides(&past, &nit), "two and a half");
    past.push(signal("d", "ignored", 4, 4, "src/helper.rs", fixture::NIT));
    assert!(hides(&past, &nit), "three");
}

/// The next review of `octo/repo` after the fixture's history.
fn next_review(db: &Connection) -> (String, Vec<String>) {
    fixture::review(
        db,
        "repo",
        106,
        "sha-106",
        fixture::day(6.0),
        vec![
            fixture::finding("src/helper.rs", ReviewSeverity::Nit, "Missing doc comment on `helper`"),
            fixture::finding("src/search.rs", ReviewSeverity::Critical, fixture::CRITICAL),
        ],
        vec![],
    )
}

#[test]
fn a_new_review_drafts_with_the_repeat_hidden_and_the_reason_in_your_words() {
    let (_scratch, db) = open();
    fixture::build(&db);
    let (job, ids) = next_review(&db);
    let findings = store::findings(&db, &job).unwrap();
    let nit = findings.iter().find(|finding| finding.id == ids[0]).unwrap();
    assert_eq!(nit.hidden_reason.as_deref(), Some("You dropped 5 like this, last on #105."));
    assert_eq!(nit.decision, Decision::Pending, "hidden is not decided");
    let critical = findings.iter().find(|finding| finding.id == ids[1]).unwrap();
    assert_eq!(critical.hidden_reason, None);
    let evidence: String =
        db.query_row("SELECT hidden_by_signals FROM review_findings WHERE id=?1", params![ids[0]], |row| row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<Vec<String>>(&evidence).unwrap().len(), 5);

    assert!(!crate::reviews::post::chosen(ReviewPostMode::Auto, nit), "auto-post skips what was hidden");
    assert!(crate::reviews::post::chosen(ReviewPostMode::Auto, critical));
    fixture::decide(&db, &job, fixture::day(6.1), &[(ids[0].as_str(), Decision::Kept, None)]);
    let kept = store::findings(&db, &job).unwrap().into_iter().find(|finding| finding.id == ids[0]).unwrap();
    assert!(crate::reviews::post::chosen(ReviewPostMode::Auto, &kept), "kept, it posts");
    let signal = &signals::history(&db, signals::SOURCE_REVIEW_DECISION, &ids[0]).unwrap()[0];
    assert_eq!(signal.detail, Some(serde_json::json!({"wasHidden": true})), "and it counts against hiding next time");
}

#[test]
fn turning_the_setting_off_hides_nothing() {
    let (_scratch, db) = open();
    fixture::build(&db);
    let settings = crate::reviews::settings::load(&db).unwrap();
    assert!(settings.hide_repeat_drops, "on by default");
    crate::reviews::settings::save(&db, &bridge_protocol::messages::ReviewJobSettings { hide_repeat_drops: false, ..settings }).unwrap();
    let (job, _) = next_review(&db);
    assert!(store::findings(&db, &job).unwrap().iter().all(|finding| finding.hidden_reason.is_none()));
}

#[test]
fn a_sweep_of_suppression_thresholds_on_the_fixture() {
    let (_scratch, db) = open();
    fixture::build(&db);
    let rows = replay::sweep(&db, &[(2, 1), (3, 2), (4, 2), (5, 2)]).unwrap();
    let shape: Vec<_> = rows.iter().map(|row| (row.min_drops, row.min_prs, row.hidden, row.wrongly_hidden, row.critical_hidden)).collect();
    assert_eq!(shape, [(2, 1, 3, 0, 0), (3, 2, 2, 0, 0), (4, 2, 1, 0, 0), (5, 2, 0, 0, 0)]);
}
