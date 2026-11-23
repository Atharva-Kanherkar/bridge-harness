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

// --- posting through `gh`, with the real runner --------------------------------

use crate::reviews::prepare::LiveSteps;
use crate::reviews::runner::{Runner, RunnerConfig};
use crate::reviews::steps::{DraftFinding, PassOutcome, PassRequest, Prepared, ReviewSteps};
use crate::runtime::BridgeCore;
use rusqlite::params;
use std::path::Path;
use std::time::Duration;

/// A `gh` that keeps the PR's reviews in files: a POST stores the review
/// it was given (so a later list finds its marker) unless told to refuse.
const STUB: &str = r#"#!/bin/sh
root=$(dirname "$0")
printf '%s\n' "$*" >> "$root/invocations.log"
if [ "$1 $2" = "auth status" ]; then exit 0; fi
if [ "$1" != "api" ]; then exit 1; fi
endpoint="$6"
if [ "$5" = "POST" ]; then
n=$(( $(cat "$root/posts" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$root/posts"
cp "$8" "$root/posted-$n.json"
if [ -f "$root/reject" ]; then cat "$root/reject" >&2; exit 1; fi
marker=$(grep -o '<!-- bridge-review:[^ ]* -->' "$8" | head -1)
url="https://github.com/octo/repo/pull/7#pullrequestreview-555"
printf '[{"id":555,"html_url":"%s","body":"Earlier.\\n\\n%s"}]' "$url" "$marker" > "$root/reviews.json"
if [ -f "$root/accept-then-time-out" ]; then
    rm "$root/accept-then-time-out"
    echo 'HTTP 504: Gateway Timeout' >&2
    exit 1
fi
printf '{"id":555,"html_url":"%s","body":"posted"}' "$url"
exit 0
fi
case "$endpoint" in
*/reviews/*/comments*) cat "$root/comments.json" 2>/dev/null || echo '[]' ;;
*/reviews*) cat "$root/reviews.json" 2>/dev/null || echo '[]' ;;
*) exit 1 ;;
esac
"#;

/// Real posting; preparation and the reviewer are scripted.
struct Offline {
    live: LiveSteps,
    findings: Vec<DraftFinding>,
}

impl ReviewSteps for Offline {
    fn prepare(&self, job: &JobRow, _cancel: &CancelToken) -> Result<Prepared, StepError> {
        Ok(Prepared { head_sha: job.head_sha.clone(), ..Default::default() })
    }

    fn review(&self, _request: PassRequest<'_>, _cancel: &CancelToken) -> Result<PassOutcome, StepError> {
        Ok(PassOutcome { summary: "One real problem.".into(), findings: self.findings.clone(), ..Default::default() })
    }

    fn find_posted(&self, job: &JobRow) -> Result<Option<PostedReview>, StepError> {
        self.live.find_posted(job)
    }

    fn post(&self, job: &JobRow, cancel: &CancelToken) -> Result<PostedReview, StepError> {
        self.live.post(job, cancel)
    }

    fn cleanup(&self, _job_id: &str) {}
}

struct Fixture {
    _dir: tempfile::TempDir,
    gh: std::path::PathBuf,
    core: Arc<BridgeCore>,
    runner: Runner,
}

impl Fixture {
    fn posts(&self) -> usize {
        std::fs::read_to_string(self.gh.join("posts")).map(|text| text.trim().parse().unwrap()).unwrap_or(0)
    }

    fn posted(&self, n: usize) -> Value {
        serde_json::from_slice(&std::fs::read(self.gh.join(format!("posted-{n}.json"))).unwrap()).unwrap()
    }

    fn invocations(&self) -> Vec<String> {
        std::fs::read_to_string(self.gh.join("invocations.log")).unwrap_or_default().lines().map(str::to_owned).collect()
    }

    fn drive(&self) {
        for handle in self.runner.tick().unwrap() {
            handle.join().unwrap();
        }
    }

    fn job(&self, id: &str) -> JobRow {
        store::require_job(&self.core.db.lock().unwrap(), id).unwrap()
    }

    fn findings(&self, id: &str) -> Vec<wire::ReviewFinding> {
        let mut found = store::findings(&self.core.db.lock().unwrap(), id).unwrap();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found
    }

    fn start(&self) -> String {
        let params = wire::StartReviewParams {
            workspace_id: "w".into(),
            number: 7,
            harnesses: Some(vec!["claude".into()]),
            trigger: None,
        };
        let head = crate::reviews::PrHead {
            repo_owner: "octo".into(),
            repo_name: "repo".into(),
            head_sha: HEAD.into(),
            base_sha: None,
        };
        crate::reviews::start_with(&self.core, &params, head).unwrap().id
    }
}

fn draft(path: &str, line: u32, side: ReviewSide, outside_diff: bool) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line,
        start_line: None,
        side,
        severity: ReviewSeverity::Major,
        title: format!("Problem in {path}"),
        body: format!("Why {path} is wrong."),
        confidence: 0.9,
        fingerprint: path.into(),
        outside_diff,
    }
}

fn fixture(mode: ReviewPostMode) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let gh = dir.path().join("gh-bin");
    std::fs::create_dir(&gh).unwrap();
    let script = gh.join("gh");
    std::fs::write(&script, STUB).unwrap();
    let mut permissions = std::fs::metadata(&script).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
    std::fs::set_permissions(&script, permissions).unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();

    let mut core = BridgeCore::for_tests(&dir.path().join("data"));
    core.github_surface = crate::github_surface::GithubSurface::discover_on_path(&gh);
    let core = Arc::new(core);
    {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo',?1,'now')", params![workspace.to_string_lossy()])
            .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Oslo','Demo','main',?1,'ready','now')",
            params![workspace.to_string_lossy()],
        )
        .unwrap();
        let mut settings = crate::reviews::settings::load(&db).unwrap();
        settings.post_mode = mode;
        crate::reviews::settings::save(&db, &settings).unwrap();
    }
    core.reviews.set_steps(Arc::new(Offline {
        live: LiveSteps::new(&core),
        findings: vec![
            draft("a.rs", 2, ReviewSide::Right, false),
            draft("b.rs", 5, ReviewSide::Left, false),
            draft("c.rs", 40, ReviewSide::Right, true),
        ],
    }));
    let config = RunnerConfig {
        renew_every: Duration::from_millis(30),
        retry_backoff: Duration::from_millis(1),
        step_retries: 3,
        poll: Duration::from_millis(10),
    };
    let runner = Runner::with(core.clone(), config, Arc::new(chrono::Utc::now));
    Fixture { _dir: dir, gh, core, runner }
}

fn decide(fixture: &Fixture, job_id: &str) {
    let ids: Vec<String> = fixture.findings(job_id).into_iter().map(|finding| finding.id).collect();
    let decision = |id: &str, decision, edited: Option<&str>| wire::ReviewFindingDecisionInput {
        finding_id: id.into(),
        decision,
        edited_body: edited.map(str::to_owned),
    };
    crate::reviews::decide(
        &fixture.core,
        &wire::DecideReviewParams {
            job_id: job_id.into(),
            decisions: vec![
                decision(&ids[0], ReviewFindingDecision::Kept, None),
                decision(&ids[1], ReviewFindingDecision::Dropped, None),
                decision(&ids[2], ReviewFindingDecision::Edited, Some("Put it in plainer words.")),
            ],
        },
    )
    .unwrap();
}

#[test]
fn a_draft_post_reaches_github_once_with_the_kept_findings() {
    let fixture = fixture(ReviewPostMode::Draft);
    std::fs::write(
        fixture.gh.join("comments.json"),
        r#"[{"id":901,"path":"a.rs","line":2,"original_line":2,"side":"RIGHT","body":"anything"}]"#,
    )
    .unwrap();
    let id = fixture.start();
    fixture.drive();
    assert_eq!(fixture.job(&id).status, Status::Drafted);
    assert_eq!(fixture.posts(), 0, "nothing reaches GitHub until a person posts");

    decide(&fixture, &id);
    let posting = crate::reviews::post(&fixture.core, &id).unwrap();
    assert_eq!(posting.status, Status::Posting);
    fixture.drive();

    let job = fixture.job(&id);
    assert_eq!(job.status, Status::Posted, "{:?}", job.error);
    assert_eq!(job.posted_review_url.as_deref(), Some("https://github.com/octo/repo/pull/7#pullrequestreview-555"));
    assert_eq!(fixture.posts(), 1);
    let payload = fixture.posted(1);
    assert_eq!(payload["event"], "COMMENT");
    assert_eq!(payload["commit_id"], HEAD);
    let comments = payload["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1, "the dropped finding never posts: {comments:?}");
    assert_eq!((comments[0]["path"].as_str(), comments[0]["line"].as_u64()), (Some("a.rs"), Some(2)));
    let body = payload["body"].as_str().unwrap();
    assert!(body.contains("Put it in plainer words.") && !body.contains("Why c.rs is wrong."), "edited as edited: {body}");
    assert!(!body.contains("b.rs"));
    assert!(body.ends_with(&post_marker(&id)));

    let log = fixture.invocations();
    let list = log.iter().position(|line| line.contains("--method GET") && line.ends_with("/reviews?per_page=100 --paginate"));
    let post = log.iter().position(|line| line.contains("--method POST"));
    assert!(list.unwrap() < post.unwrap(), "the marker is checked before posting: {log:?}");

    let findings = fixture.findings(&id);
    assert_eq!(findings[0].github_comment_id.as_deref(), Some("901"));
    assert!(findings[0].posted_at.is_some());
    assert!(findings[1].posted_at.is_none(), "dropped");
    assert!(findings[2].posted_at.is_some() && findings[2].github_comment_id.is_none(), "outside the diff, in the body");
}

#[test]
fn a_post_that_timed_out_after_github_took_it_is_not_posted_twice() {
    let fixture = fixture(ReviewPostMode::Draft);
    std::fs::write(fixture.gh.join("accept-then-time-out"), "").unwrap();
    let id = fixture.start();
    fixture.drive();
    decide(&fixture, &id);
    crate::reviews::post(&fixture.core, &id).unwrap();
    fixture.drive();
    let job = fixture.job(&id);
    assert_eq!(job.status, Status::Posted, "{:?}", job.error);
    assert_eq!(job.posted_review_id.as_deref(), Some("555"), "the review GitHub already had");
    assert_eq!(fixture.posts(), 1, "one review on GitHub");
    let findings = fixture.findings(&id);
    assert!(findings[2].posted_at.is_none(), "the found review's body does not hold the outside finding");
}

#[test]
fn an_auto_job_posts_without_anyone_deciding() {
    let fixture = fixture(ReviewPostMode::Auto);
    let id = fixture.start();
    fixture.drive();
    let job = fixture.job(&id);
    assert_eq!(job.status, Status::Posted, "{:?}", job.error);
    assert_eq!(fixture.posts(), 1);
    let payload = fixture.posted(1);
    assert_eq!(payload["event"], "COMMENT");
    let comments = payload["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1]["side"], "LEFT");
    assert!(payload["body"].as_str().unwrap().contains("Why c.rs is wrong."));
}

#[test]
fn a_rejected_post_returns_the_draft_with_the_reason() {
    let fixture = fixture(ReviewPostMode::Draft);
    std::fs::write(fixture.gh.join("reject"), "HTTP 422: Unprocessable Entity (line must be part of the diff)").unwrap();
    let id = fixture.start();
    fixture.drive();
    decide(&fixture, &id);
    crate::reviews::post(&fixture.core, &id).unwrap();
    fixture.drive();
    let job = fixture.job(&id);
    assert_eq!(job.status, Status::Drafted);
    assert!(job.error.as_deref().unwrap().starts_with("GitHub refused the review"), "{:?}", job.error);
    assert_eq!(fixture.posts(), 1, "a refusal is not retried");
    assert_eq!(fixture.findings(&id)[0].decision, ReviewFindingDecision::Kept, "the triage stays");

    std::fs::remove_file(fixture.gh.join("reject")).unwrap();
    crate::reviews::post(&fixture.core, &id).unwrap();
    fixture.drive();
    assert_eq!(fixture.job(&id).status, Status::Posted, "and posts once fixed");
    assert_eq!(fixture.posts(), 2);
}

fn gh_text(gh: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new(gh).args(args).output().unwrap();
    assert!(output.status.success(), "gh {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A live core whose `gh` is the real one, and a posting job of the sandbox
/// PR holding a single-line, a range, a LEFT and an outside-diff finding, all
/// kept.
fn live_posting_job(
    gh: &Path,
    (owner, name, number, head): (&str, &str, u64, &str),
    (range_path, range): (&str, (u32, u32)),
    (left_path, left_line): (&str, u32),
) -> (tempfile::TempDir, Arc<BridgeCore>, JobRow) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let mut core = BridgeCore::for_tests(&dir.path().join("data"));
    core.github_surface = crate::github_surface::GithubSurface::discover_on_path(gh.parent().unwrap());
    let core = Arc::new(core);
    let job_id = {
        let db = core.db.lock().unwrap();
        db.execute("INSERT INTO projects(id,name,path,created_at) VALUES('p','Live',?1,'now')", params![workspace.to_string_lossy()])
            .unwrap();
        db.execute(
            "INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Oslo','Live','main',?1,'ready','now')",
            params![workspace.to_string_lossy()],
        )
        .unwrap();
        let (job, _) = store::insert(
            &db,
            &store::NewJob {
                workspace_id: "w".into(),
                repo_owner: owner.into(),
                repo_name: name.into(),
                number,
                head_sha: head.into(),
                base_sha: None,
                incremental_from_sha: None,
                trigger: ReviewTrigger::Manual,
                post_mode: ReviewPostMode::Draft,
                harnesses: vec!["claude".into()],
                retry_of: None,
                publication_key: None,
            },
            chrono::Utc::now(),
        )
        .unwrap();
        let token = store::claim_next(&db, "live", chrono::Utc::now()).unwrap().unwrap().lease_owner.unwrap();
        assert!(store::advance(&db, &job.id, &token, Status::Preparing, Status::Reviewing, false).unwrap());
        let pass = store::begin_pass(&db, &job.id, &token, 0, "claude", chrono::Utc::now()).unwrap().unwrap();
        let mut single = draft(range_path, range.1, ReviewSide::Right, false);
        single.title = "Live test: one line".into();
        let mut ranged = draft(range_path, range.1, ReviewSide::Right, false);
        ranged.start_line = Some(range.0);
        ranged.title = "Live test: a range".into();
        let mut left = draft(left_path, left_line, ReviewSide::Left, false);
        left.title = "Live test: an old line".into();
        let mut outside = draft(range_path, 99_999, ReviewSide::Right, true);
        outside.title = "Live test: outside the diff".into();
        let outcome = PassOutcome {
            summary: "Bridge live posting test. Safe to ignore.".into(),
            findings: vec![single, ranged, left, outside],
            ..Default::default()
        };
        assert!(store::complete_pass(&db, &job.id, &token, &pass, &outcome, chrono::Utc::now()).unwrap());
        assert!(store::finish_review(&db, &job.id, &token, "Bridge live posting test. Safe to ignore.", true).unwrap());
        let decisions: Vec<wire::ReviewFindingDecisionInput> = store::findings(&db, &job.id)
            .unwrap()
            .into_iter()
            .map(|finding| wire::ReviewFindingDecisionInput {
                finding_id: finding.id,
                decision: ReviewFindingDecision::Kept,
                edited_body: None,
            })
            .collect();
        store::decide(&db, &job.id, &decisions, chrono::Utc::now()).unwrap();
        store::request_post(&db, &job.id).unwrap();
        job.id
    };
    let job = store::claim_next(&core.db.lock().unwrap(), "live", chrono::Utc::now()).unwrap().unwrap();
    assert_eq!(job.id, job_id);
    (dir, core, job)
}

/// Posts one `COMMENT` review to a real pull request with a single-line,
/// a multi-line and a LEFT comment, checks GitHub placed each where it was
/// sent, and checks a second post finds the marker instead of posting
/// again. Run only against a sandbox repository:
/// `BRIDGE_REVIEW_LIVE_REPO=owner/name BRIDGE_REVIEW_LIVE_PR=n cargo test
/// -p bridge-core --lib live_post_on_a_sandbox_pr -- --ignored --nocapture`.
/// The PR needs a hunk with two added lines and a hunk with an old line.
#[test]
#[ignore = "posts to a real sandbox pull request"]
fn live_post_on_a_sandbox_pr() {
    let repository = std::env::var("BRIDGE_REVIEW_LIVE_REPO").expect("BRIDGE_REVIEW_LIVE_REPO=owner/name");
    let number: u64 = std::env::var("BRIDGE_REVIEW_LIVE_PR").expect("BRIDGE_REVIEW_LIVE_PR=n").parse().unwrap();
    let (owner, name) = repository.split_once('/').expect("owner/name");
    let gh = which::which("gh").expect("gh on PATH");
    let n = number.to_string();
    let head = gh_text(&gh, &["pr", "view", &n, "--repo", &repository, "--json", "headRefOid", "-q", ".headRefOid"]);
    let diff = gh_text(&gh, &["pr", "diff", &n, "--repo", &repository]);
    let hunks = crate::reviews::contract::hunks(&diff);
    let (range_path, range) = hunks
        .iter()
        .flat_map(|(path, hunks)| hunks.iter().map(move |hunk| (path, hunk)))
        .find(|(_, hunk)| hunk.new.len() >= 2)
        .map(|(path, hunk)| (path.clone(), (hunk.new[0], hunk.new[1])))
        .expect("a hunk with two lines on the new side");
    let (left_path, left_line) = hunks
        .iter()
        .flat_map(|(path, hunks)| hunks.iter().map(move |hunk| (path, hunk)))
        .find(|(_, hunk)| !hunk.old.is_empty())
        .map(|(path, hunk)| (path.clone(), hunk.old[0]))
        .expect("a hunk with an old line");

    let (_dir, core, job) =
        live_posting_job(&gh, (owner, name, number, &head), (&range_path, range), (&left_path, left_line));

    let posted = crate::reviews::post::post(&core, &job, &CancelToken::new()).unwrap();
    println!("posted {}", posted.url);
    let review: Value = serde_json::from_str(&gh_text(
        &gh,
        &["api", &format!("repos/{repository}/pulls/{number}/reviews/{}", posted.review_id)],
    ))
    .unwrap();
    assert_eq!(review["state"], "COMMENTED", "{review}");
    assert_eq!(review["commit_id"], json!(head));
    let comments: Value = serde_json::from_str(&gh_text(
        &gh,
        &["api", &format!("repos/{repository}/pulls/{number}/reviews/{}/comments", posted.review_id)],
    ))
    .unwrap();
    let placed: Vec<(String, u64, String, Option<u64>)> = comments
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| {
            (
                comment["path"].as_str().unwrap().to_owned(),
                comment["original_line"].as_u64().unwrap(),
                comment["side"].as_str().unwrap().to_owned(),
                comment["original_start_line"].as_u64(),
            )
        })
        .collect();
    println!("placed {placed:?}");
    assert!(placed.contains(&(range_path.clone(), u64::from(range.1), "RIGHT".into(), None)), "{placed:?}");
    assert!(placed.contains(&(range_path.clone(), u64::from(range.1), "RIGHT".into(), Some(u64::from(range.0)))), "{placed:?}");
    assert!(placed.contains(&(left_path.clone(), u64::from(left_line), "LEFT".into(), None)), "{placed:?}");
    assert_eq!(posted.findings.iter().filter(|finding| finding.comment_id.is_some()).count(), 3, "{posted:?}");

    let found = crate::reviews::post::find_posted(&core, &job).unwrap().expect("the marker finds the review");
    assert_eq!(found.review_id, posted.review_id, "a retry would post nothing new");
    assert_eq!(found.findings.len(), 4, "the found review holds every finding: {found:?}");
}
