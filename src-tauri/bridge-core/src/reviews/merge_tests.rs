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
fn seven_lines_apart_stay_separate() {
    let a = entry("a", "p1", 10, "Division by zero on empty slice");
    let b = entry("b", "p2", 17, "Division by zero on empty slice");
    assert!(merges(&[a, b]).is_empty());
    let c = entry("c", "p2", 16, "Division by zero on empty slice");
    assert_eq!(merges(&[entry("a", "p1", 10, "Division by zero on empty slice"), c]).len(), 1, "six apart is still near");
}

/// A live pair word for word: Codex and OpenCode (GPT-5.5) on one line, no
/// title word in common.
#[test]
fn quoted_code_in_the_bodies_merges() {
    let codex = with(entry("codex", "p1", 8, "Reject indices equal to the slice length"), |f| {
        f.body = "When `index == items.len()`, this condition allows `items[index]`, which panics instead of returning the documented `None`. This also makes `get(&[], 0)` panic. Change `<=` to `<`, or use `items.get(index).copied()`, and add tests for an empty slice.".into();
    });
    let gpt = with(entry("gpt", "p2", 8, "Fix off-by-one bounds check in get"), |f| {
        f.body = "This condition allows `index == items.len()`, but indexing with `items[index]` on the next line panics in that case. That violates the documented behavior of returning `None` when the index is out of range. Use `index < items.len()` or simply `items.get(index).copied()`.".into();
    });
    let similarity = Similarity::of(&codex.finding, &gpt.finding);
    assert_eq!(similarity.title, 0.0);
    assert!(similarity.code >= MIN_CODE_OVERLAP, "{similarity:?}");
    assert_eq!(merges(&[codex, gpt]).len(), 1);
}

/// A live pair: Claude points at the end of the `if` (line 12), Codex at the
/// condition (line 8).
#[test]
fn four_lines_apart_merge_when_the_bodies_agree() {
    let claude = with(entry("claude", "p1", 12, "Off-by-one in `get`: `index <= items.len()` panics when index == len"), |f| {
        f.body = "When `index == items.len()` the condition is true and `items[index]` panics with an out-of-bounds error. The doc comment promises `None` for out-of-range indices. Use `<` instead, or simply `items.get(index).copied()`.".into();
    });
    let codex = with(entry("codex", "p2", 8, "Reject indices equal to the slice length"), |f| {
        f.body = "The `<=` check allows `index == items.len()`, so `get(&[1], 1)` and `get(&[], 0)` reach `items[index]` and panic instead of returning the documented `None`. Use `<` or replace the implementation with `items.get(index).copied()`.".into();
    });
    assert_eq!(merges(&[claude, codex]).len(), 1);
}

/// Live negatives: a finding that the PR has no tests, three lines from the
/// bug, and the naming finding four lines away, stay their own findings.
#[test]
fn a_test_gap_finding_near_the_bug_stays_separate() {
    let tests = with(entry("tests", "p1", 11, "No tests for the new functions"), |f| {
        f.body = "Nothing tests either function. A test for `get` at `index == len` would have caught the panic. Add tests for the empty slice, in-range, `index == len` and far-out-of-range cases. Add one for `Sum_All` with values near `i32::MAX`.".into();
    });
    let bug = with(entry("bug", "p2", 8, "Fix off-by-one bounds check in get"), |f| {
        f.body = "`index <= items.len()` allows `index == items.len()`, but `items[index]` panics in that case. This violates the documented behavior of returning `None` for out-of-range indexes. Use `index < items.len()` or simply `items.get(index).copied()`.".into();
    });
    assert!(!Similarity::of(&tests.finding, &bug.finding).matches());
    assert!(merges(&[tests, bug]).is_empty());
}

#[test]
fn naming_and_the_bug_stay_separate() {
    let bug = with(entry("bug", "p1", 12, "Off-by-one in `get`: index == len panics instead of returning None"), |f| {
        f.body = "`index <= items.len()` admits `index == items.len()`, so `items[index]` panics with an out-of-bounds error. Use `index < items.len()`, or simply `items.get(index).copied()`.".into();
    });
    let naming = with(entry("naming", "p2", 16, "Sum_All violates Rust snake_case naming and triggers non_snake_case lint"), |f| {
        f.body = "`Sum_All` should be `sum_all`: Rust function names are snake_case, and this spelling fires the warn-by-default `non_snake_case` lint on every build.".into();
    });
    assert!(merges(&[bug, naming]).is_empty());
}

#[test]
fn low_title_overlap_stays_separate() {
    let a = with(entry("a", "p1", 10, "Division by zero on empty slice"), |f| f.body = "`average` divides by `items.len()`.".into());
    let b = with(entry("b", "p2", 10, "Integer overflow in sum"), |f| f.body = "`sum::<i32>()` overflows past `i32::MAX`.".into());
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
