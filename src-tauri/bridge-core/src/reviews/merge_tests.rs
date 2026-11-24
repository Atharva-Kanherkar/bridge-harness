use super::*;
use bridge_protocol::messages::{ReviewSeverity, ReviewSide};

fn entry(id: &str, pass: &str, line: u32, title: &str) -> Entry {
    Entry {
        id: id.into(),
        pass_id: pass.into(),
        finding: DraftFinding {
            path: "src/lib.rs".into(),
            line,
            start_line: None,
            side: ReviewSide::Right,
            severity: ReviewSeverity::Minor,
            title: title.into(),
            body: format!("Body of {id}."),
            confidence: 0.5,
            fingerprint: format!("fp-{id}"),
            outside_diff: false,
        },
    }
}

fn with(mut entry: Entry, change: impl FnOnce(&mut DraftFinding)) -> Entry {
    change(&mut entry.finding);
    entry
}

fn pairs(merges: &[Merge]) -> Vec<(String, String)> {
    merges.iter().map(|merge| (merge.keep.clone(), merge.drop.clone())).collect()
}

#[test]
fn equal_fingerprints_merge() {
    let a = with(entry("a", "p1", 10, "Off by one in last"), |f| f.fingerprint = "same".into());
    let b = with(entry("b", "p2", 10, "Slice start is wrong"), |f| f.fingerprint = "same".into());
    let merges = merges(&[a, b]);
    assert_eq!(merges.len(), 1, "equal fingerprints are one finding whatever the titles say");
    assert_eq!(merges[0].pass_ids.iter().collect::<HashSet<_>>(), ["p1".to_owned(), "p2".to_owned()].iter().collect());
}

#[test]
fn near_lines_and_similar_titles_merge() {
    let a = entry("a", "p1", 10, "Division by zero on an empty slice");
    let b = with(entry("b", "p2", 15, "average divides by zero for empty slice"), |f| f.start_line = Some(13));
    assert!(title_overlap(&a.finding.title, &b.finding.title) >= MIN_TITLE_OVERLAP);
    assert_eq!(merges(&[a, b]).len(), 1, "ranges 3 lines apart with similar titles merge");
}

#[test]
fn four_lines_apart_stay_separate() {
    let a = entry("a", "p1", 10, "Division by zero on empty slice");
    let b = entry("b", "p2", 14, "Division by zero on empty slice");
    assert!(merges(&[a, b]).is_empty());
}

#[test]
fn low_title_overlap_stays_separate() {
    let a = entry("a", "p1", 10, "Division by zero on empty slice");
    let b = entry("b", "p2", 10, "Integer overflow in sum");
    assert!(title_overlap(&a.finding.title, &b.finding.title) < MIN_TITLE_OVERLAP);
    assert!(merges(&[a, b]).is_empty(), "two problems on one line stay two findings");
}

#[test]
fn other_paths_stay_separate() {
    let a = with(entry("a", "p1", 10, "Same"), |f| f.fingerprint = "same".into());
    let b = with(entry("b", "p2", 10, "Same"), |f| {
        f.fingerprint = "same".into();
        f.path = "src/other.rs".into();
    });
    assert!(merges(&[a, b]).is_empty());
}

#[test]
fn findings_of_one_pass_never_merge() {
    let a = entry("a", "p1", 10, "Division by zero on empty slice");
    let b = entry("b", "p1", 11, "Division by zero on empty slice");
    assert!(merges(&[a, b]).is_empty(), "one reviewer saying it twice is not agreement");
}

#[test]
fn higher_severity_and_confidence_are_kept() {
    let a = with(entry("a", "p1", 10, "Division by zero on empty slice"), |f| {
        f.severity = ReviewSeverity::Major;
        f.confidence = 0.6;
    });
    let b = with(entry("b", "p2", 10, "Division by zero on empty slice"), |f| {
        f.severity = ReviewSeverity::Minor;
        f.confidence = 0.7;
    });
    let merged = &merges(&[a, b])[0];
    assert_eq!(merged.severity, ReviewSeverity::Major);
    assert!((merged.confidence - 0.7).abs() < f64::EPSILON);
}

#[test]
fn longer_body_wins_unless_confidence_differs() {
    let short = with(entry("short", "p1", 10, "Division by zero on empty slice"), |f| f.body = "Divides by zero.".into());
    let long = with(entry("long", "p2", 10, "Division by zero on empty slice"), |f| {
        f.body = "`average` divides by `items.len()`, which is zero for an empty slice, so it panics.".into();
    });
    assert_eq!(pairs(&merges(&[short.clone(), long.clone()])), [("long".into(), "short".into())], "the clearer (longer) body");
    let sure = with(short, |f| f.confidence = 0.9);
    let unsure = with(long, |f| f.confidence = 0.6);
    assert_eq!(pairs(&merges(&[sure, unsure])), [("short".into(), "long".into())], "more than 0.2 surer wins over longer");
}

#[test]
fn each_finding_merges_once() {
    // p1's finding is close to two of p2's; the exact title wins it, and the
    // other p2 finding stays on its own.
    let a = entry("a", "p1", 10, "Division by zero on empty slice");
    let exact = entry("exact", "p2", 12, "Division by zero on empty slice");
    let close = entry("close", "p2", 10, "Division by zero for empty input slice");
    let merges = merges(&[a, close, exact]);
    assert_eq!(merges.len(), 1);
    assert!(pairs(&merges)[0] == ("a".into(), "exact".into()) || pairs(&merges)[0] == ("exact".into(), "a".into()));
}
