//! Compiled only under #[cfg(test)] from post.rs.

use super::*;
use bridge_protocol::messages::{ReviewJobStatus as Status, ReviewTrigger};
use serde_json::{json, Value};

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

fn job(mode: ReviewPostMode) -> JobRow {
    JobRow {
        id: "job-1".into(),
        workspace_id: "w".into(),
        repo_owner: "octo".into(),
        repo_name: "repo".into(),
        number: 7,
        head_sha: HEAD.into(),
        base_sha: None,
        incremental_from_sha: None,
        trigger: ReviewTrigger::Manual,
        post_mode: mode,
        harnesses: vec!["claude".into()],
        publication_key: "job-1".into(),
        status: Status::Posting,
        error: None,
        summary: Some("Two problems in the parser.".into()),
        posted_review_id: None,
        posted_review_url: None,
        retry_of: None,
        attempts: 1,
        lease_owner: Some("runner/x".into()),
        lease_expires_at: None,
        created_at: "now".into(),
        started_at: None,
        finished_at: None,
        worktree_path: None,
    }
}

fn finding(id: &str, decision: ReviewFindingDecision) -> wire::ReviewFinding {
    wire::ReviewFinding {
        id: id.into(),
        job_id: "job-1".into(),
        pass_id: "pass-1".into(),
        path: format!("src/{id}.rs"),
        line: 12,
        start_line: None,
        side: ReviewSide::Right,
        severity: ReviewSeverity::Major,
        title: format!("Title {id}"),
        body: format!("Body {id}"),
        confidence: 0.8,
        fingerprint: id.into(),
        agreed_by: 1,
        outside_diff: false,
        decision,
        edited_body: None,
        decided_at: None,
        github_comment_id: None,
        posted_at: None,
    }
}

fn input<'a>(job: &'a JobRow, findings: Vec<wire::ReviewFinding>) -> PostInput<'a> {
    PostInput {
        job,
        findings: findings.into_iter().map(|finding| (finding, "claude".to_owned())).collect(),
        harnesses: vec!["claude".into()],
        hide_attribution: false,
        current_head: Some(HEAD.into()),
    }
}

fn wire_of(plan: &PostPlan) -> Value {
    serde_json::to_value(&plan.payload).unwrap()
}

#[test]
fn every_payload_says_comment_and_nothing_else() {
    let mut ranged = finding("r", ReviewFindingDecision::Kept);
    ranged.start_line = Some(10);
    let mut left = finding("l", ReviewFindingDecision::Edited);
    left.side = ReviewSide::Left;
    left.edited_body = Some("edited".into());
    let mut outside = finding("o", ReviewFindingDecision::Kept);
    outside.outside_diff = true;
    let sets = [vec![], vec![ranged.clone()], vec![left.clone(), outside.clone()], vec![ranged, left, outside]];
    for mode in [ReviewPostMode::Draft, ReviewPostMode::Auto] {
        let job = job(mode);
        for findings in &sets {
            for hide in [false, true] {
                for moved in [None, Some("f".repeat(40))] {
                    let mut built = input(&job, findings.clone());
                    built.hide_attribution = hide;
                    built.current_head = moved;
                    let value = wire_of(&build(&built));
                    assert_eq!(value["event"], json!("COMMENT"));
                    assert_eq!(value["commit_id"], json!(HEAD), "always the reviewed commit");
                    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
                    assert_eq!(keys.len(), 4, "commit_id, event, body, comments and nothing else: {keys:?}");
                }
            }
        }
    }
    assert_eq!(serde_json::to_value(CommentEvent).unwrap(), json!("COMMENT"));
}

#[test]
fn no_review_source_names_another_event() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reviews");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).unwrap();
        let source = source.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["APPROVE", "REQUEST_CHANGES", "--approve", "--request-changes", "pr review", "pr merge"] {
            assert!(!source.contains(forbidden), "{name} names {forbidden}");
        }
    }
}

#[test]
fn only_kept_and_edited_findings_post_in_draft_mode() {
    let job = job(ReviewPostMode::Draft);
    let mut edited = finding("e", ReviewFindingDecision::Edited);
    edited.edited_body = Some("The person's words.".into());
    let plan = build(&input(
        &job,
        vec![
            finding("k", ReviewFindingDecision::Kept),
            edited,
            finding("p", ReviewFindingDecision::Pending),
            finding("d", ReviewFindingDecision::Dropped),
        ],
    ));
    assert_eq!(plan.inline, ["k", "e"]);
    let bodies: Vec<&str> = plan.payload.comments.iter().map(|comment| comment.body.as_str()).collect();
    assert!(bodies[0].contains("Body k"));
    assert!(bodies[1].contains("The person's words.") && !bodies[1].contains("Body e"), "{}", bodies[1]);
    assert!(!plan.payload.body.contains("Body p") && !plan.payload.body.contains("Body d"));
}

#[test]
fn auto_mode_posts_everything_not_dropped() {
    let job = job(ReviewPostMode::Auto);
    let plan = build(&input(
        &job,
        vec![finding("p", ReviewFindingDecision::Pending), finding("d", ReviewFindingDecision::Dropped)],
    ));
    assert_eq!(plan.inline, ["p"]);
}

#[test]
fn ranges_and_left_side_comments_carry_their_fields() {
    let job = job(ReviewPostMode::Draft);
    let mut ranged = finding("r", ReviewFindingDecision::Kept);
    ranged.start_line = Some(9);
    let mut left = finding("l", ReviewFindingDecision::Kept);
    left.side = ReviewSide::Left;
    left.line = 4;
    let mut not_a_range = finding("n", ReviewFindingDecision::Kept);
    not_a_range.start_line = Some(12);
    let value = wire_of(&build(&input(&job, vec![ranged, left, not_a_range])));
    assert_eq!(
        value["comments"][0],
        json!({"path": "src/r.rs", "line": 12, "side": "RIGHT", "start_line": 9, "start_side": "RIGHT",
               "body": value["comments"][0]["body"]})
    );
    assert_eq!(value["comments"][1]["side"], "LEFT");
    assert_eq!(value["comments"][1]["line"], 4);
    assert!(value["comments"][1].get("start_line").is_none());
    assert!(value["comments"][2].get("start_line").is_none(), "a start on the line itself is one line");
    assert!(value["comments"][2].get("start_side").is_none());
}

#[test]
fn outside_diff_findings_fold_into_the_body() {
    let job = job(ReviewPostMode::Draft);
    let mut outside = finding("o", ReviewFindingDecision::Kept);
    outside.outside_diff = true;
    outside.line = 300;
    let plan = build(&input(&job, vec![outside, finding("i", ReviewFindingDecision::Kept)]));
    assert_eq!(plan.inline, ["i"]);
    assert_eq!(plan.payload.comments.len(), 1);
    assert_eq!(plan.outside.len(), 1);
    let body = &plan.payload.body;
    assert!(body.starts_with("Two problems in the parser."), "{body}");
    assert!(body.contains("### Outside the diff\n\n**Major** · `src/o.rs:300` · Title o\n\nBody o"), "{body}");
    assert!(body.ends_with("<!-- bridge-review:job-1 -->"), "the marker is last");
}

#[test]
fn the_footer_names_the_harness_only_when_attribution_is_shown() {
    let job = job(ReviewPostMode::Draft);
    let mut shown = input(&job, vec![finding("k", ReviewFindingDecision::Kept)]);
    shown.harnesses = vec!["claude".into(), "codex".into()];
    let plan = build(&shown);
    assert!(plan.payload.comments[0].body.ends_with("<sub>Bridge review · claude</sub>"));
    assert!(plan.payload.body.contains("<sub>Bridge review · claude, codex</sub>"));

    shown.hide_attribution = true;
    let hidden = build(&shown);
    let all = format!("{}{}", hidden.payload.body, hidden.payload.comments[0].body);
    for named in ["claude", "codex", "Bridge review", "<sub>"] {
        assert!(!all.contains(named), "{named} in {all}");
    }
    assert!(hidden.payload.body.ends_with("<!-- bridge-review:job-1 -->"), "the marker stays: it is how a retry finds the post");
}

#[test]
fn a_moved_head_is_named_in_the_body() {
    let job = job(ReviewPostMode::Draft);
    let mut moved = input(&job, vec![]);
    moved.current_head = Some("fedcba9876543210fedcba9876543210fedcba98".into());
    let plan = build(&moved);
    assert_eq!(plan.payload.commit_id, HEAD);
    assert!(plan.payload.body.contains("Reviewed at `0123456789ab`. The pull request has moved on to `fedcba987654`"), "{}", plan.payload.body);
    for unchanged in [Some(HEAD.to_owned()), None] {
        let mut still = input(&job, vec![]);
        still.current_head = unchanged;
        assert!(!build(&still).payload.body.contains("moved on"));
    }
    let mut summary_only = job.clone();
    summary_only.summary = None;
    assert!(build(&input(&summary_only, vec![])).payload.body.starts_with("No issues found."));
}

#[test]
fn bodies_stay_under_the_github_limit() {
    let mut long = job(ReviewPostMode::Draft);
    long.summary = Some("s".repeat(100_000));
    let mut huge = finding("h", ReviewFindingDecision::Edited);
    huge.edited_body = Some("x".repeat(200_000));
    let mut findings = vec![huge];
    for index in 0..40 {
        let mut outside = finding(&format!("o{index}"), ReviewFindingDecision::Kept);
        outside.outside_diff = true;
        outside.body = "y".repeat(5_000);
        findings.push(outside);
    }
    let plan = build(&input(&long, findings));
    assert!(plan.payload.comments[0].body.chars().count() <= MAX_COMMENT_CHARS);
    assert!(plan.payload.comments[0].body.contains("cut to fit"));
    assert!(plan.payload.comments[0].body.ends_with("<sub>Bridge review · claude</sub>"), "the footer survives the cut");
    assert!(plan.payload.body.chars().count() <= 65_536);
    assert!(plan.outside.len() < 40, "not every outside finding fits");
    assert!(plan.payload.body.contains(&format!("_{} more outside the diff did not fit", 40 - plan.outside.len())));
    assert!(plan.payload.body.ends_with("<!-- bridge-review:job-1 -->"));
}

fn returned(id: &str, path: &str, line: u32, body: &str) -> ReturnedComment {
    ReturnedComment { id: id.into(), path: path.into(), line: Some(line), side: Some(ReviewSide::Right), body: body.into() }
}

#[test]
fn returned_comments_map_back_to_findings() {
    let job = job(ReviewPostMode::Draft);
    // Two findings on one line: the exact body decides which is which.
    let first = finding("a", ReviewFindingDecision::Kept);
    let mut second = finding("b", ReviewFindingDecision::Kept);
    second.path = first.path.clone();
    let mut outside = finding("o", ReviewFindingDecision::Kept);
    outside.outside_diff = true;
    let plan = build(&input(&job, vec![first, second, outside]));
    let comments = vec![
        returned("902", "src/a.rs", 12, &plan.payload.comments[1].body),
        returned("901", "src/a.rs", 12, &plan.payload.comments[0].body),
    ];
    let posted = posted_now(&plan, &comments);
    assert_eq!(
        posted,
        [
            PostedFinding { finding_id: "a".into(), comment_id: Some("901".into()) },
            PostedFinding { finding_id: "b".into(), comment_id: Some("902".into()) },
            PostedFinding { finding_id: "o".into(), comment_id: None },
        ]
    );
    // Comments GitHub did not return still posted: the review was created
    // with all of them or none.
    let unlisted = posted_now(&plan, &[]);
    assert_eq!(unlisted.len(), 3);
    assert!(unlisted.iter().all(|finding| finding.comment_id.is_none()));
}

#[test]
fn a_found_review_marks_only_what_it_holds() {
    let job = job(ReviewPostMode::Draft);
    let mut outside = finding("o", ReviewFindingDecision::Kept);
    outside.outside_diff = true;
    let mut gone = finding("g", ReviewFindingDecision::Kept);
    gone.outside_diff = true;
    let plan = build(&input(&job, vec![finding("a", ReviewFindingDecision::Kept), finding("b", ReviewFindingDecision::Kept), outside.clone(), gone]));
    // An earlier attempt posted `a` (with another footer) and `o`, not `b` or `g`.
    let found = FoundReview {
        id: "5".into(),
        url: "https://github.com/octo/repo/pull/7#pullrequestreview-5".into(),
        body: format!("Earlier.\n\n{}\n\nBody o\n\n<!-- bridge-review:job-1 -->", outside_heading(&outside)),
    };
    let held = held_by(&plan, &found, &[returned("77", "src/a.rs", 12, "**Major** · Title a\n\nBody a")]);
    assert_eq!(
        held,
        [
            PostedFinding { finding_id: "a".into(), comment_id: Some("77".into()) },
            PostedFinding { finding_id: "o".into(), comment_id: None },
        ]
    );
}

#[test]
fn gh_errors_map_to_retry_or_fail() {
    let failed = |stderr: &str| GithubSurfaceError::CommandFailed { operation: "post review", stderr: stderr.into() };
    for passing in [
        "timed out after 30s and was killed",
        "HTTP 502: Bad Gateway",
        "HTTP 429: secondary rate limit",
        "error connecting to api.github.com",
        "dial tcp: lookup api.github.com: could not resolve host",
    ] {
        assert!(matches!(gh_error(failed(passing)), StepError::Transient(_)), "{passing}");
    }
    for refused in [
        "HTTP 422: Unprocessable Entity (pull_request_review_thread.line must be part of the diff)",
        "HTTP 401: Bad credentials",
        "HTTP 403: Resource not accessible by integration",
        "HTTP 404: Not Found",
        "something else entirely",
    ] {
        assert!(matches!(gh_error(failed(refused)), StepError::Fatal(_)), "{refused}");
    }
    let StepError::Fatal(message) = gh_error(failed("HTTP 422: Unprocessable Entity")) else { unreachable!() };
    assert!(message.starts_with("GitHub refused the review"), "{message}");
    assert!(matches!(gh_error(GithubSurfaceError::Io(std::io::Error::other("pipe"))), StepError::Transient(_)));
    assert!(matches!(
        gh_error(GithubSurfaceError::MalformedResponse { resource: "review", detail: "x".into() }),
        StepError::Fatal(_)
    ));
}
