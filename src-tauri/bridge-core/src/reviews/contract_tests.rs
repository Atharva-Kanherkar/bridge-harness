use super::*;
use serde_json::json;

fn fenced(value: serde_json::Value) -> String {
    format!("Here is my review.\n\n```json\n{value}\n```\n")
}

fn finding(path: &str, line: u32, severity: &str, title: &str) -> serde_json::Value {
    json!({"path": path, "line": line, "side": "RIGHT", "severity": severity, "title": title, "body": "b", "confidence": 0.5})
}

/// Two hunks in one file: lines 2 to 4 old and new around a change, and a
/// later hunk with a deleted line.
const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -2,3 +2,4 @@ fn top() {
 keep two
-old three
+new three
+new four
 keep four
@@ -20,3 +21,2 @@ fn bottom() {
 keep twenty
-gone twenty-one
 keep twenty-two
diff --git a/docs/b.md b/docs/b.md
new file mode 100644
--- /dev/null
+++ b/docs/b.md
@@ -0,0 +1,2 @@
+hello
+world
diff --git a/old.txt b/old.txt
deleted file mode 100644
--- a/old.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
";

fn changed() -> HashSet<String> {
    ["src/a.rs", "docs/b.md", "old.txt", "bin.dat"].into_iter().map(String::from).collect()
}

#[test]
fn the_json_block_is_found_inside_prose() {
    let message = format!(
        "I read the diff and the threads.\n\nSome notes in ```inline code``` first.\n\n```json\n{}\n```\n\nThanks.",
        json!({"summary": "Fine.", "findings": [finding("src/a.rs", 3, "major", "Bug")]})
    );
    let review = parse(&message).unwrap();
    assert_eq!(review.summary, "Fine.");
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.findings[0].side, ReviewSide::Right);
}

#[test]
fn a_missing_or_doubled_block_is_an_error_a_repair_turn_can_quote() {
    assert!(parse("No JSON at all.").unwrap_err().contains("no fenced ```json block"));
    let two = format!("{}{}", fenced(json!({"summary": "a", "findings": []})), fenced(json!({"summary": "b", "findings": []})));
    assert!(parse(&two).unwrap_err().contains("2 fenced"));
    assert!(parse("```json\n{\"summary\": \"unclosed\"").unwrap_err().contains("no fenced"));
}

#[test]
fn fields_are_strict() {
    let cases = [
        (json!({"findings": []}), "summary"),
        (json!({"summary": "s"}), "findings"),
        (json!({"summary": "s", "findings": [{"path": "a", "line": "3", "severity": "major", "title": "t", "body": "b", "confidence": 1}]}), "invalid type"),
        (json!({"summary": "s", "findings": [{"path": "a", "line": 3, "severity": "blocker", "title": "t", "body": "b", "confidence": 1}]}), "unknown variant"),
        (json!({"summary": "s", "findings": [{"path": "a", "line": 3, "side": "MIDDLE", "severity": "nit", "title": "t", "body": "b", "confidence": 1}]}), "unknown variant"),
        (json!({"summary": "s", "findings": [{"path": "a", "line": -1, "severity": "nit", "title": "t", "body": "b", "confidence": 1}]}), "invalid value"),
        (json!({"summary": "s", "findings": [{"path": "a", "severity": "nit", "title": "t", "body": "b", "confidence": 1}]}), "line"),
        (json!({"summary": "s", "findings": [], "verdict": "approve"}), "unknown field"),
    ];
    for (value, expected) in cases {
        let error = parse(&fenced(value.clone())).unwrap_err();
        assert!(error.contains(expected), "{value}: {error}");
    }
    assert!(parse("```json\n{not json}\n```").unwrap_err().contains("does not match the contract"));
}

#[test]
fn findings_inside_a_hunk_are_inline_and_the_rest_move_to_the_summary() {
    let review = parse(&fenced(json!({"summary": " Overall fine. ", "findings": [
        finding("src/a.rs", 3, "major", "Added line"),
        finding("src/a.rs", 5, "minor", "Context line"),
        {"path": "src/a.rs", "line": 21, "side": "LEFT", "severity": "minor", "title": "Deleted line", "body": "b", "confidence": 0.4},
        // New line 5 is in the first hunk; old line 5 is not.
        {"path": "src/a.rs", "line": 5, "side": "LEFT", "severity": "minor", "title": "A new line on the old side", "body": "b", "confidence": 0.4},
        finding("src/a.rs", 12, "major", "Between hunks"),
        {"path": "src/a.rs", "line": 4, "startLine": 2, "side": "RIGHT", "severity": "nit", "title": "Range in one hunk", "body": "b", "confidence": 0.1},
        {"path": "src/a.rs", "line": 21, "startLine": 3, "side": "RIGHT", "severity": "nit", "title": "Range across hunks", "body": "b", "confidence": 0.1},
        finding("docs/b.md", 2, "minor", "New file"),
        {"path": "old.txt", "line": 1, "side": "LEFT", "severity": "minor", "title": "Deleted file", "body": "b", "confidence": 0.3},
        finding("bin.dat", 1, "minor", "Binary"),
        finding("src/untouched.rs", 1, "critical", "Not in the PR"),
    ]})))
    .unwrap();
    let validated = validate(review, &changed(), &hunks(PATCH));
    assert_eq!(validated.summary, "Overall fine.");
    let inline: Vec<&str> = validated.findings.iter().filter(|finding| !finding.outside_diff).map(|finding| finding.title.as_str()).collect();
    let outside: Vec<&str> = validated.findings.iter().filter(|finding| finding.outside_diff).map(|finding| finding.title.as_str()).collect();
    for title in ["Added line", "Context line", "Deleted line", "Range in one hunk", "New file", "Deleted file"] {
        assert!(inline.contains(&title), "{title} should be inline: {inline:?}");
    }
    for title in ["A new line on the old side", "Between hunks", "Range across hunks", "Binary", "Not in the PR"] {
        assert!(outside.contains(&title), "{title} should be outside the diff: {outside:?}");
    }
    let first_outside = validated.findings.iter().position(|finding| finding.outside_diff).unwrap();
    assert!(validated.findings[first_outside..].iter().all(|finding| finding.outside_diff), "inline findings lead");
    let range = validated.findings.iter().find(|finding| finding.title == "Range in one hunk").unwrap();
    assert_eq!((range.start_line, range.line), (Some(2), 4));
}

#[test]
fn hunks_track_both_sides_line_numbers() {
    let files = hunks(PATCH);
    let a = &files["src/a.rs"];
    assert_eq!(a.len(), 2);
    assert_eq!(a[0].new, [2, 3, 4, 5]);
    assert_eq!(a[0].old, [2, 3, 4]);
    assert_eq!(a[1].old, [20, 21, 22]);
    assert_eq!(a[1].new, [21, 22]);
    assert!(a[1].text.starts_with("@@ -20,3 +21,2 @@") && a[1].text.contains("-gone twenty-one"));
    assert_eq!(files["docs/b.md"][0].new, [1, 2]);
    assert_eq!(files["old.txt"][0].old, [1], "a deleted file is keyed by its old path");
    let quoted = "diff --git \"a/sp\\tace.txt\" \"b/sp\\tace.txt\"\n--- \"a/sp\\tace.txt\"\n+++ \"b/sp\\tace.txt\"\n@@ -1 +1 @@\n-x\n+y\n";
    assert!(hunks(quoted).contains_key("sp\tace.txt"));
}

#[test]
fn duplicates_collapse_and_ranking_is_severity_then_confidence() {
    let mut findings = vec![
        json!({"path": "src/a.rs", "line": 3, "severity": "minor", "title": "Same issue", "body": "b", "confidence": 0.9}),
        json!({"path": "src/a.rs", "line": 3, "severity": "major", "title": "same issue!", "body": "b", "confidence": 0.2}),
        json!({"path": "src/a.rs", "line": 4, "severity": "critical", "title": "Low confidence", "body": "b", "confidence": 0.1}),
        json!({"path": "src/a.rs", "line": 5, "severity": "critical", "title": "High confidence", "body": "b", "confidence": 3.5}),
    ];
    for index in 0..60 {
        findings.push(json!({"path": "src/a.rs", "line": 2, "severity": "nit", "title": format!("Nit {index}"), "body": "b", "confidence": 0.5}));
    }
    let validated = validate(parse(&fenced(json!({"summary": "s", "findings": findings}))).unwrap(), &changed(), &hunks(PATCH));
    assert_eq!(validated.duplicates, 1, "same path, line and normalized title");
    assert_eq!(validated.findings.len(), MAX_FINDINGS);
    assert_eq!(validated.over_limit, 63 - MAX_FINDINGS);
    assert_eq!(validated.findings[0].title, "High confidence");
    assert_eq!(validated.findings[0].confidence, 1.0, "clamped");
    assert_eq!(validated.findings[1].title, "Low confidence");
    assert_eq!(validated.findings[2].title, "Same issue", "the first of the duplicates is kept");
    assert!(validated.findings[3..].iter().all(|finding| finding.severity == ReviewSeverity::Nit));
}

#[test]
fn titles_are_cut_and_fingerprints_follow_the_hunk_not_the_wording() {
    let long = "x".repeat(300);
    let validated = validate(
        parse(&fenced(json!({"summary": "s", "findings": [
            finding("src/a.rs", 3, "major", &long),
            finding("src/a.rs", 3, "major", "Unchecked  UNWRAP."),
            finding("src/a.rs", 4, "major", "unchecked unwrap"),
            finding("src/a.rs", 21, "major", "unchecked unwrap"),
        ]})))
        .unwrap(),
        &changed(),
        &hunks(PATCH),
    );
    assert_eq!(validated.findings[0].title.chars().count(), MAX_TITLE_CHARS);
    let prints: Vec<&str> = validated.findings[1..].iter().map(|finding| finding.fingerprint.as_str()).collect();
    assert_eq!(prints[0], prints[1], "same hunk and normalized title, whatever the line or casing");
    assert_ne!(prints[0], prints[2], "another hunk is another finding");
    assert_eq!(prints[0].len(), 64);
    assert_eq!(fingerprint("src/a.rs", "Unchecked unwrap", "h"), fingerprint("src/a.rs", "unchecked, unwrap", "h"));
}

#[test]
fn oversized_and_empty_reviews_validate() {
    let empty = validate(parse(&fenced(json!({"summary": "Nothing to raise.", "findings": []}))).unwrap(), &changed(), &hunks(PATCH));
    assert!(empty.findings.is_empty());
    let body = "word ".repeat(20_000);
    let big = parse(&fenced(json!({"summary": body, "findings": [finding("src/a.rs", 3, "nit", "t")]}))).unwrap();
    assert_eq!(validate(big, &changed(), &hunks(PATCH)).findings.len(), 1);
    let blank_title = parse(&fenced(json!({"summary": "s", "findings": [finding("src/a.rs", 3, "nit", "   ")]}))).unwrap();
    assert!(validate(blank_title, &changed(), &hunks(PATCH)).findings.is_empty(), "a finding with no title is dropped");
}

#[test]
fn header_like_lines_inside_a_hunk_are_content() {
    // Deleting SQL comments `-- one` and `-- two`, and adding `++ counter;`.
    let patch = "diff --git a/schema.sql b/schema.sql
--- a/schema.sql
+++ b/schema.sql
@@ -1,3 +1,2 @@
--- one
--- two
+++ counter;
 keep
";
    let files = hunks(patch);
    let hunk = &files["schema.sql"][0];
    assert_eq!(hunk.old, [1, 2, 3], "both deleted comment lines and the context line");
    assert_eq!(hunk.new, [1, 2]);
    assert!(hunk.text.contains("--- two") && hunk.text.contains("+++ counter;"));
    let review = parse(&fenced(json!({"summary": "s", "findings": [
        {"path": "schema.sql", "line": 2, "side": "LEFT", "severity": "minor", "title": "Second comment", "body": "b", "confidence": 0.5},
    ]})))
    .unwrap();
    let changed = ["schema.sql".to_owned()].into_iter().collect();
    assert!(!validate(review, &changed, &files).findings[0].outside_diff, "a finding on the second deleted line is inline");
}

#[test]
fn octal_quoted_names_decode_to_the_real_path() {
    let patch = "diff --git \"a/\\346\\227\\245.txt\" \"b/\\346\\227\\245.txt\"
--- \"a/\\346\\227\\245.txt\"
+++ \"b/\\346\\227\\245.txt\"
@@ -1 +1 @@
-old
+new
";
    let files = hunks(patch);
    assert!(files.contains_key("日.txt"), "{:?}", files.keys().collect::<Vec<_>>());
    assert_eq!(unquote("a\\\"b\\\\c\\td"), "a\"b\\c\td");
}

#[test]
fn real_git_output_round_trips_through_the_parser() {
    // The same names git prints with its default core.quotePath.
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let output = crate::git::git_command(dir.path()).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@example.invalid"]);
    run(&["config", "user.name", "T"]);
    std::fs::write(dir.path().join("日.txt"), "old\n").unwrap();
    std::fs::write(dir.path().join("q.sql"), "-- one\n-- two\nkeep\n").unwrap();
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "base"]);
    std::fs::write(dir.path().join("日.txt"), "new\n").unwrap();
    std::fs::write(dir.path().join("q.sql"), "keep\n").unwrap();
    let patch = run(&["diff"]);
    let files = hunks(&patch);
    assert_eq!(files["日.txt"][0].new, [1]);
    assert_eq!(files["q.sql"][0].old, [1, 2, 3]);
}
