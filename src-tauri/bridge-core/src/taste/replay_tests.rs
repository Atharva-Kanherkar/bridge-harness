use super::fixture::{self, Fixture};
use super::*;

fn open() -> (tempfile::TempDir, Connection, Fixture) {
    let scratch = tempfile::tempdir().unwrap();
    let db = crate::store::open(&scratch.path().join("bridge.db")).unwrap();
    let built = fixture::build(&db);
    (scratch, db, built)
}

fn hidden(report: &Report, ids: &[String]) -> Vec<bool> {
    ids.iter()
        .map(|id| report.calls.iter().find(|(call, _)| call == id).map(|(_, hidden)| *hidden).expect("judged"))
        .collect()
}

#[test]
fn the_fixture_hides_the_nit_once_it_was_dropped_three_times_and_nothing_else() {
    let (_scratch, db, built) = open();
    let report = replay(&db, &RepeatedDrops::default()).unwrap();
    assert_eq!(hidden(&report, &built.nit), [false, false, false, true, true], "hidden from the fourth review, after three drops");
    assert_eq!(hidden(&report, &built.critical), [false; 4], "a critical finding is never hidden, however often dropped");
    assert_eq!(hidden(&report, &[built.near_nit.clone(), built.other_repo_nit.clone()]), [false, false], "nor one merely near it, nor another repo's");
    assert_eq!(hidden(&report, &built.unwrap), [false, false]);
    assert_eq!(hidden(&report, &[built.off_by_one.clone()]), [false]);
}

#[test]
fn every_metric_on_the_fixture() {
    let (_scratch, db, _) = open();
    let baseline = replay(&db, &NeverHide).unwrap();
    assert_eq!((baseline.reviews, baseline.findings, baseline.decided), (6, 14, 14), "the re-review that only fixed has no findings");
    assert_eq!((baseline.hidden, baseline.reviews_affected, baseline.critical_hidden), (0, 0, 0));
    assert_eq!(baseline.keep_rate_all, Some(6.0 / 14.0));
    assert_eq!(baseline.keep_rate_visible, baseline.keep_rate_all);
    assert_eq!((baseline.posted, baseline.address_rate), (1, Some(1.0)), "the posted off-by-one was fixed");

    let taste = replay(&db, &RepeatedDrops::default()).unwrap();
    assert_eq!((taste.hidden, taste.reviews_affected, taste.critical_hidden), (2, 2, 0));
    assert!(taste.wrongly_hidden.is_empty(), "{:?}", taste.wrongly_hidden);
    assert_eq!(taste.keep_rate_visible, Some(6.0 / 12.0), "what stays visible is kept more often");
    assert!(taste.keep_rate_visible > baseline.keep_rate_visible);
}

/// A judge that hides everything: every wanted finding is a miss.
struct HideAll;

impl Judge for HideAll {
    fn judge(&self, _past: &[Signal], _candidate: &Candidate) -> Judgment {
        Judgment::Hide { reason: "everything".into(), evidence: Vec::new() }
    }
}

#[test]
fn every_wanted_finding_hidden_is_listed_and_the_report_never_prints_a_body() {
    let (_scratch, db, built) = open();
    let report = replay(&db, &HideAll).unwrap();
    assert_eq!(report.critical_hidden, 4);
    assert_eq!(report.keep_rate_visible, None, "nothing visible");
    let mut missed: Vec<&str> = report.wrongly_hidden.iter().map(|miss| miss.finding_id.as_str()).collect();
    missed.sort();
    let mut wanted = vec![
        built.critical[3].as_str(),
        built.unwrap[0].as_str(),
        built.unwrap[1].as_str(),
        built.off_by_one.as_str(),
        built.near_nit.as_str(),
        built.other_repo_nit.as_str(),
    ];
    wanted.sort();
    assert_eq!(missed, wanted);
    let text = render(&report);
    assert!(text.contains(fixture::OFF_BY_ONE) && text.contains("critical hidden 4"), "{text}");
    assert!(!text.contains("Body of:"), "titles only: {text}");
}

#[test]
fn a_judge_never_sees_the_future() {
    let (_scratch, db, _) = open();
    let every = signals::all(&db).unwrap();
    let candidates = candidates(&db).unwrap();
    // A judge as sensitive to the past as one can be: one drop hides.
    let eager = RepeatedDrops { min_drops: 1, min_prs: 1 };
    let cutoff = fixture::day(4.0).to_rfc3339();
    let earlier = |report: &Report| -> Vec<(String, bool)> {
        report
            .calls
            .iter()
            .filter(|(id, _)| candidates.iter().any(|candidate| &candidate.finding_id == id && !before(&cutoff, &candidate.reviewed_at)))
            .cloned()
            .collect()
    };
    let before_change = score(&every, &candidates, &eager);
    // Rewrite everything from the cutoff on: flip every verdict, move it,
    // and add drops of every title.
    let mut future: Vec<Signal> = every.clone();
    for signal in future.iter_mut().filter(|signal| !before(&signal.at, &cutoff)) {
        signal.verdict = if signal.verdict == "dropped" { "kept".into() } else { "dropped".into() };
        signal.at = fixture::day(9.0).to_rfc3339();
    }
    for (index, candidate) in candidates.iter().enumerate() {
        let mut extra = every[0].clone();
        extra.id = format!("future-{index}");
        extra.subject_id = format!("future-{index}");
        extra.scope_key = candidate.scope_key.clone();
        extra.title = Some(candidate.title.clone());
        extra.verdict = "dropped".into();
        extra.at = signals::instant(&cutoff);
        future.push(extra);
    }
    let after_change = score(&future, &candidates, &eager);
    assert!(!earlier(&before_change).is_empty());
    assert_eq!(earlier(&after_change), earlier(&before_change), "no judgment before the cutoff moved");
    assert_ne!(after_change.calls, before_change.calls, "the change did reach the later reviews");
}

#[test]
fn a_decision_changed_later_counts_as_it_stood() {
    let (_scratch, db, built) = open();
    let review_of_102 = fixture::day(2.0);
    // The person goes back to PR 101's draft and keeps its nit after PR 102
    // was reviewed.
    let decision = bridge_protocol::messages::ReviewFindingDecisionInput {
        finding_id: built.nit[0].clone(),
        decision: bridge_protocol::messages::ReviewFindingDecision::Kept,
        edited_body: None,
    };
    crate::reviews::store::decide(&db, &built.jobs[0], &[decision], fixture::day(2.5)).unwrap();
    let every = signals::all(&db).unwrap();
    let then = as_of(&every, "project:octo/repo", &review_of_102.to_rfc3339());
    let nit_then: Vec<&str> = then.iter().filter(|signal| signal.subject_id == built.nit[0]).map(|signal| signal.verdict.as_str()).collect();
    assert_eq!(nit_then, ["dropped"], "as it stood when PR 102 was reviewed");
    let now = as_of(&every, "project:octo/repo", &fixture::day(3.0).to_rfc3339());
    let nit_now: Vec<&str> = now.iter().filter(|signal| signal.subject_id == built.nit[0]).map(|signal| signal.verdict.as_str()).collect();
    assert_eq!(nit_now, ["kept"], "and as it stood after the change");
}

#[test]
fn a_sweep_reports_one_row_per_threshold_pair() {
    let (_scratch, db, _) = open();
    let rows = sweep(&db, &[(1, 1), (3, 2), (5, 2)]).unwrap();
    let shape: Vec<_> = rows.iter().map(|row| (row.min_drops, row.min_prs, row.hidden, row.wrongly_hidden, row.critical_hidden)).collect();
    assert_eq!(shape, [(1, 1, 4, 0, 0), (3, 2, 2, 0, 0), (5, 2, 0, 0, 0)]);
    let json = serde_json::to_value(&rows[1]).unwrap();
    for key in ["minDrops", "minPrs", "hidden", "wronglyHidden", "criticalHidden", "keepRateVisible", "reviewsAffected"] {
        assert!(json.get(key).is_some(), "{key} in {json}");
    }
}

/// Replay a real ledger by hand. Reads only; prints counts and titles.
///
/// ```text
/// BRIDGE_TASTE_REPLAY_DB=<copy of bridge.db> \
///   cargo test -p bridge-core --lib taste::replay::tests::replay_a_real_ledger -- --ignored --nocapture
/// ```
#[test]
#[ignore = "by hand: set BRIDGE_TASTE_REPLAY_DB to a copy of a ledger at schema 106 or later"]
fn replay_a_real_ledger() {
    let path = std::env::var("BRIDGE_TASTE_REPLAY_DB").expect("BRIDGE_TASTE_REPLAY_DB");
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    eprintln!("taste off:\n{}", render(&replay(&db, &NeverHide).unwrap()));
    eprintln!("repeated drops (3, 2):\n{}", render(&replay(&db, &RepeatedDrops::default()).unwrap()));
    for row in sweep(&db, &[(2, 1), (2, 2), (3, 2), (4, 2), (5, 3)]).unwrap() {
        eprintln!("{}", serde_json::to_string(&row).unwrap());
    }
}
