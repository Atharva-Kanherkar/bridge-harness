use super::*;
use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::json;

use crate::reviews::steps::Prepared;
use bridge_protocol::messages::{ReviewSeverity, ReviewSide};

/// One thing a scripted session yields when read.
enum Step {
    Events(Vec<NormalizedEvent>),
    End,
}

/// A transport that plays scripted turns and records what the loop did.
#[derive(Default)]
struct Script {
    turns: Mutex<VecDeque<Vec<Step>>>,
    start_error: Mutex<Option<String>>,
    starts: Mutex<Vec<(String, Option<String>, Option<String>, String)>>,
    sent: Mutex<Vec<String>>,
    declined: Mutex<Vec<Value>>,
    rejected: Mutex<Vec<(Value, Option<String>)>>,
    stopped: Mutex<Vec<bool>>,
}

struct ScriptedSession {
    script: Arc<Script>,
    current: VecDeque<Step>,
}

struct ScriptedTransport(Arc<Script>);

impl Transport for ScriptedTransport {
    fn start(&self, harness: &str, start: &SessionStart<'_>) -> Result<Box<dyn Session>, String> {
        if let Some(error) = self.0.start_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.0.starts.lock().unwrap().push((
            harness.to_owned(),
            start.model.map(str::to_owned),
            start.effort.map(str::to_owned),
            start.instructions.to_owned(),
        ));
        Ok(Box::new(ScriptedSession { script: self.0.clone(), current: VecDeque::new() }))
    }
}

impl Session for ScriptedSession {
    fn send_turn(&mut self, text: &str) -> Result<(), String> {
        self.script.sent.lock().unwrap().push(text.to_owned());
        self.current = self.script.turns.lock().unwrap().pop_front().unwrap_or_default().into();
        Ok(())
    }

    fn next(&mut self, wait: Duration) -> Next {
        match self.current.pop_front() {
            Some(Step::Events(events)) => Next::Events(events),
            Some(Step::End) => Next::Ended,
            // Out of script: the provider is thinking, forever.
            None => {
                std::thread::sleep(wait.min(Duration::from_millis(10)));
                Next::Idle
            }
        }
    }

    fn decline(&mut self, request_id: Value) -> Result<(), String> {
        self.script.declined.lock().unwrap().push(request_id);
        Ok(())
    }

    fn reject_question(&mut self, request_id: Value, method: Option<&str>) -> Result<(), String> {
        self.script.rejected.lock().unwrap().push((request_id, method.map(str::to_owned)));
        Ok(())
    }

    fn stop(&mut self, completed: bool) {
        self.script.stopped.lock().unwrap().push(completed);
    }
}

fn event(kind: &str) -> NormalizedEvent {
    NormalizedEvent { kind: kind.into(), item_id: None, role: None, status: None, title: None, text: None, data: json!({}) }
}

fn said(item: &str, text: &str) -> NormalizedEvent {
    NormalizedEvent { role: Some("assistant".into()), item_id: Some(item.into()), text: Some(text.into()), ..event("message.completed") }
}

fn with_data(kind: &str, data: Value) -> NormalizedEvent {
    NormalizedEvent { data, ..event(kind) }
}

fn tool(name: &str) -> NormalizedEvent {
    NormalizedEvent { title: Some(name.into()), ..event("tool.started") }
}

fn answer(findings: Value) -> String {
    format!("Done.\n\n```json\n{}\n```", json!({"summary": "Looks good overall.", "findings": findings}))
}

fn good_turn() -> Vec<Step> {
    vec![
        Step::Events(vec![event("turn.started"), tool("Read"), tool("Grep")]),
        Step::Events(vec![said("m1", "Reading the diff first."), said("m2", &answer(json!([])))]),
        Step::Events(vec![
            with_data("usage.updated", json!({"usage": {"input_tokens": 1200, "output_tokens": 300}, "totalCostUsd": 0.0125})),
            event("turn.completed"),
        ]),
    ]
}

fn script(turns: Vec<Vec<Step>>) -> Arc<Script> {
    let script = Arc::new(Script::default());
    script.turns.lock().unwrap().extend(turns);
    script
}

fn start(cwd: &Path) -> SessionStart<'_> {
    SessionStart { cwd, model: Some("m"), effort: Some("high"), instructions: "stable" }
}

#[test]
fn a_clean_turn_returns_the_review_and_its_usage() {
    let script = script(vec![good_turn()]);
    let (result, usage) = run(&ScriptedTransport(script.clone()), "claude", &start(Path::new("/tmp")), "review it", &CancelToken::new(), Limits::default());
    let review = result.unwrap();
    assert_eq!(review.summary, "Looks good overall.");
    assert_eq!((usage.tokens_in, usage.tokens_out, usage.cost_microusd), (1200, 300, 12_500));
    assert_eq!(usage.tools, ["Read", "Grep"]);
    assert_eq!(script.sent.lock().unwrap().as_slice(), ["review it"], "one turn, no repair");
    assert_eq!(script.stopped.lock().unwrap().as_slice(), [true], "the session is always stopped");
}

#[test]
fn permission_requests_and_questions_are_declined_at_once_and_counted() {
    let script = script(vec![vec![
        Step::Events(vec![event("turn.started"), with_data("permission.requested", json!({"requestId": "p1", "tool": "bash"}))]),
        Step::Events(vec![with_data("approval.requested", json!({"requestId": 7}))]),
        Step::Events(vec![with_data("question.requested", json!({"requestId": "q1", "requestMethod": "mcpServer/elicitation/request"}))]),
        Step::Events(vec![said("m", &answer(json!([]))), event("turn.completed")]),
    ]]);
    let started = Instant::now();
    let (result, usage) = run(&ScriptedTransport(script.clone()), "opencode", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    assert!(result.is_ok());
    assert!(started.elapsed() < Duration::from_secs(2), "nothing waited for a person");
    assert_eq!(usage.denied_requests, 3);
    assert_eq!(script.declined.lock().unwrap().as_slice(), [json!("p1"), json!(7)]);
    assert_eq!(
        script.rejected.lock().unwrap().as_slice(),
        [(json!("q1"), Some("mcpServer/elicitation/request".to_owned()))],
        "the question is refused on its own channel"
    );
}

#[test]
fn a_broken_answer_gets_one_repair_turn() {
    let script = script(vec![
        vec![Step::Events(vec![event("turn.started"), said("m", "I think it is fine, no JSON."), event("turn.completed")])],
        vec![Step::Events(vec![event("turn.started"), said("r", &answer(json!([]))), event("turn.completed")])],
    ]);
    let (result, _) = run(&ScriptedTransport(script.clone()), "codex", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    assert!(result.is_ok());
    let sent = script.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent[1].contains("no fenced ```json block") && sent[1].contains("Do not use any tools"), "{}", sent[1]);
}

#[test]
fn an_answer_still_broken_after_the_repair_fails_the_pass() {
    let broken = || vec![Step::Events(vec![event("turn.started"), said("m", "```json\n{\"summary\": 1}\n```"), event("turn.completed")])];
    let script = script(vec![broken(), broken(), good_turn()]);
    let (result, _) = run(&ScriptedTransport(script.clone()), "codex", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    let error = result.unwrap_err();
    assert!(matches!(&error, SessionError::Contract(message) if message.contains("does not match the contract")), "{error:?}");
    assert!(matches!(error.into_step(), StepError::Fatal(message) if message.contains("findings contract")));
    assert_eq!(script.sent.lock().unwrap().len(), 2, "one repair turn, not more");
    assert_eq!(script.stopped.lock().unwrap().as_slice(), [false]);
}

#[test]
fn the_tool_cap_and_the_wall_clock_end_the_session() {
    let busy = script(vec![vec![Step::Events(vec![event("turn.started"), tool("Read"), tool("Read"), tool("Read")])]]);
    let limits = Limits { max_wall: Duration::from_secs(60), max_tool_calls: 2 };
    let (result, _) = run(&ScriptedTransport(busy.clone()), "claude", &start(Path::new("/tmp")), "go", &CancelToken::new(), limits);
    assert_eq!(result.unwrap_err(), SessionError::ToolLimit(2));
    assert_eq!(busy.stopped.lock().unwrap().as_slice(), [false]);

    let hung = script(vec![vec![Step::Events(vec![event("turn.started")])]]);
    let limits = Limits { max_wall: Duration::from_millis(300), max_tool_calls: 300 };
    let started = Instant::now();
    let (result, _) = run(&ScriptedTransport(hung.clone()), "opencode", &start(Path::new("/tmp")), "go", &CancelToken::new(), limits);
    assert!(matches!(result.unwrap_err(), SessionError::Deadline(_)));
    assert!(started.elapsed() < Duration::from_secs(2), "the wall clock is never exceeded by much");
    assert_eq!(hung.stopped.lock().unwrap().as_slice(), [false], "a hung session is stopped");
}

#[test]
fn a_cancel_stops_a_hung_session_quickly() {
    let hung = script(vec![vec![Step::Events(vec![event("turn.started")])]]);
    let cancel = CancelToken::new();
    let trip = {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        })
    };
    let started = Instant::now();
    let (result, _) = run(&ScriptedTransport(hung.clone()), "claude", &start(Path::new("/tmp")), "go", &cancel, Limits::default());
    trip.join().unwrap();
    assert_eq!(result.unwrap_err(), SessionError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(hung.stopped.lock().unwrap().as_slice(), [false], "the harness process is stopped");
}

#[test]
fn errors_tell_the_runner_whether_to_retry_or_fall_back() {
    assert!(matches!(SessionError::Provider("429 rate limit reached".into()).into_step(), StepError::Transient(_)));
    assert!(matches!(SessionError::Start("Overloaded, try again".into()).into_step(), StepError::Transient(_)));
    assert!(matches!(SessionError::Provider("Not logged in".into()).into_step(), StepError::Harness(_)));
    assert!(matches!(SessionError::Start("opencode is unavailable".into()).into_step(), StepError::Harness(message) if message.contains("could not start")));
    assert!(matches!(SessionError::Ended.into_step(), StepError::Harness(_)));
    assert!(matches!(SessionError::Deadline(Duration::from_secs(900)).into_step(), StepError::Harness(message) if message.contains("15-minute")));
    assert!(matches!(SessionError::ToolLimit(300).into_step(), StepError::Fatal(_)));
    assert_eq!(SessionError::Cancelled.into_step(), StepError::Cancelled);

    let failing = Arc::new(Script::default());
    *failing.start_error.lock().unwrap() = Some("Claude is not signed in".into());
    let (result, _) = run(&ScriptedTransport(failing), "claude", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    assert_eq!(result.unwrap_err(), SessionError::Start("Claude is not signed in".into()));
    let ended = script(vec![vec![Step::Events(vec![event("turn.started")]), Step::End]]);
    let (result, _) = run(&ScriptedTransport(ended), "codex", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    assert_eq!(result.unwrap_err(), SessionError::Ended);
    let errored = script(vec![vec![Step::Events(vec![
        NormalizedEvent { status: Some("retrying".into()), text: Some("Reconnecting 1/5".into()), ..event("error") },
        NormalizedEvent { text: Some("usage limit reached".into()), ..event("error") },
    ])]]);
    let (result, _) = run(&ScriptedTransport(errored), "codex", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    assert_eq!(result.unwrap_err(), SessionError::Provider("usage limit reached".into()), "a retrying error is not the end");
}

#[test]
fn the_stable_prefix_holds_nothing_that_varies_per_pr() {
    let stable = instructions(&["AGENTS.md"], "Only security.", true);
    assert!(stable.starts_with(crate::prompts::ATTRIBUTION_HIDING_RULE), "the hiding rule leads when it is on");
    assert!(stable.contains("`AGENTS.md`") && !stable.contains("`CLAUDE.md`"), "only the convention files that exist");
    assert!(stable.contains("`conventions/`") && stable.contains("as its base branch has them"));
    assert!(stable.contains("Only security."));
    assert!(stable.ends_with(OUTPUT_CONTRACT), "the contract closes the stable prefix");
    assert!(stable.contains("do not run gh"));
    let bare = instructions(&[], "x", false);
    assert!(!bare.contains(crate::prompts::ATTRIBUTION_HIDING_RULE));
    assert!(!bare.contains("`conventions/`"), "no conventions, no pointer to them");
    // The PR's own text never steers the review, conventions or not.
    for prefix in [&stable, &bare] {
        assert!(prefix.contains(PR_IS_DATA), "{prefix}");
    }

    let pr = json!({"number": 42, "title": "Add retries", "changedFiles": 3, "omittedFiles": ["big.lock"]});
    let turn = prompt(&pr, ".bridge-review-0a1b2c3d4e5f");
    assert!(turn.starts_with("Review pull request #42: Add retries"));
    assert!(turn.contains(".bridge-review-0a1b2c3d4e5f/diff.patch") && turn.contains("3 changed files"));
    assert!(!stable.contains(".bridge-review-0a1b2c3d4e5f"), "the folder name varies, so it stays out of the prefix");
    assert!(turn.contains("big.lock"), "the files the diff left out are named");
    assert!(!stable.contains("#42") && !stable.contains("Add retries"), "nothing per-PR in the cached prefix");
}

const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,2 +1,3 @@
 fn main() {
+    let value = input.unwrap();
 }
";

fn job(core: &BridgeCore) -> crate::reviews::store::JobRow {
    let db = super::super::lock_db(core);
    let (job, _) = crate::reviews::store::insert(
        &db,
        &crate::reviews::store::NewJob {
            workspace_id: "w".into(),
            repo_owner: "o".into(),
            repo_name: "r".into(),
            number: 42,
            head_sha: "h".into(),
            base_sha: None,
            incremental_from_sha: None,
            trigger: Default::default(),
            post_mode: Default::default(),
            harnesses: vec!["claude".into()],
            retry_of: None,
            publication_key: None,
        },
        chrono::Utc::now(),
    )
    .unwrap();
    job
}

#[test]
fn a_pass_reads_the_context_and_keeps_inline_and_outside_findings_apart() {
    let scratch = tempfile::tempdir().unwrap();
    let core = BridgeCore::for_tests(scratch.path());
    {
        let db = super::super::lock_db(&core);
        let settings = bridge_protocol::messages::ReviewerSettings { system_prompt: "Focus on PR {number}'s error handling.".into(), ..Default::default() };
        crate::reviewer_settings::save(&db, &settings).unwrap();
    }
    let checkout = scratch.path().join("checkout");
    let context = scratch.path().join("context");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(&context).unwrap();
    std::fs::write(context.join("pr.json"), json!({"number": 42, "title": "Parse input", "changedFiles": 2, "omittedFiles": ["huge.json"]}).to_string()).unwrap();
    std::fs::write(context.join("files.json"), json!([{"path": "src/a.rs", "status": "modified"}, {"path": "huge.json", "status": "added"}]).to_string()).unwrap();
    std::fs::write(context.join("diff.patch"), PATCH).unwrap();
    std::fs::write(context.join("threads.json"), "[]").unwrap();
    let job = job(&core);
    let prepared = Prepared { worktree: Some(checkout.clone()), context: Some(context), head_sha: "h".into(), ..Prepared::default() };

    let findings = json!([
        {"path": "src/a.rs", "line": 2, "side": "RIGHT", "severity": "major", "title": "Unwrap on user input", "body": "Panics on bad input.", "confidence": 0.9},
        {"path": "huge.json", "line": 1, "side": "RIGHT", "severity": "minor", "title": "Generated file checked in", "body": "b", "confidence": 0.5},
    ]);
    let script = script(vec![vec![Step::Events(vec![event("turn.started"), said("m", &answer(findings)), event("turn.completed")])]]);
    let meter = crate::reviews::steps::UsageMeter::default();
    let request = PassRequest { job: &job, prepared: &prepared, pass_id: "p", harness: "claude", usage: &meter };
    let outcome = review_pass_with(&core, &ScriptedTransport(script.clone()), request, &CancelToken::new(), Limits::default()).unwrap();

    let placed: Vec<_> = std::fs::read_dir(&checkout).unwrap().filter_map(Result::ok).map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    assert!(placed.len() == 1 && placed[0].starts_with(CONTEXT_PREFIX) && placed[0].len() == CONTEXT_PREFIX.len() + 12, "{placed:?}");
    assert!(checkout.join(&placed[0]).join("diff.patch").exists(), "the context sits inside the checkout");
    let inline: Vec<&str> = outcome.findings.iter().filter(|finding| !finding.outside_diff).map(|finding| finding.title.as_str()).collect();
    let outside: Vec<&str> = outcome.findings.iter().filter(|finding| finding.outside_diff).map(|finding| finding.title.as_str()).collect();
    assert_eq!(inline, ["Unwrap on user input"]);
    assert_eq!(outside, ["Generated file checked in"], "a file the diff left out cannot take an inline comment");
    assert!(outcome.summary.starts_with("Looks good overall.") && outcome.summary.contains("huge.json"), "{}", outcome.summary);
    assert_eq!(outcome.session_id, None, "a hidden session leaves no chat behind");
    assert_eq!(outcome.effort.as_deref(), Some("high"));

    let starts = script.starts.lock().unwrap();
    let (harness, _, effort, instructions) = &starts[0];
    assert_eq!((harness.as_str(), effort.as_deref()), ("claude", Some("high")));
    assert!(instructions.contains("Focus on PR 42's error handling."), "the person's own review rules, number filled in");
    assert!(script.sent.lock().unwrap()[0].starts_with("Review pull request #42: Parse input"));
}

/// A real harness reviews a fixture PR with one planted bug. Run by hand:
///
/// ```text
/// BRIDGE_REVIEW_LIVE_HARNESS=opencode BRIDGE_REVIEW_LIVE_MODEL=opencode/big-pickle \
///   cargo test -p bridge-core --lib reviews::session::tests::live_review_on_a_fixture_pr -- --ignored --nocapture
/// ```
///
/// OpenCode runs in a scratch `XDG_DATA_HOME` so it never touches the
/// person's own sessions; that leaves only OpenCode's free models. For a paid
/// one, `BRIDGE_REVIEW_LIVE_OPENCODE_SIGNED_IN=1` uses their sign-in and saves
/// the review in their OpenCode history. Claude and Codex use the signed-in
/// account. `BRIDGE_REVIEW_LIVE_WAIT_SECS` lengthens the catalog wait.
#[test]
#[ignore = "live: runs a real harness; set BRIDGE_REVIEW_LIVE_HARNESS"]
fn live_review_on_a_fixture_pr() {
    let Ok(harness) = std::env::var("BRIDGE_REVIEW_LIVE_HARNESS") else { return };
    let model = std::env::var("BRIDGE_REVIEW_LIVE_MODEL").ok();
    let scratch = tempfile::tempdir().unwrap();
    // OpenCode keeps its sign-ins in its own data folder, so a paid model
    // needs the person's (`BRIDGE_REVIEW_LIVE_OPENCODE_SIGNED_IN=1`), and the
    // review is then saved in their OpenCode history, as in the app. Without
    // it, a scratch folder keeps their history untouched (free models only).
    if harness == "opencode" && std::env::var_os("BRIDGE_REVIEW_LIVE_OPENCODE_SIGNED_IN").is_none() {
        std::env::set_var("XDG_DATA_HOME", scratch.path().join("xdg-data"));
    }
    let repo = std::fs::canonicalize(scratch.path()).unwrap().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let git = |args: &[&str]| {
        let output = crate::git::git_command(&repo).args(args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "user.name", "T"]);
    std::fs::write(repo.join("src/lib.rs"), "/// The last `n` items.\npub fn last(items: &[i32], n: usize) -> &[i32] {\n    &items[items.len() - n..]\n}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    // The planted bug: off by one, and it panics when n > len.
    std::fs::write(
        repo.join("src/lib.rs"),
        "/// The last `n` items.\npub fn last(items: &[i32], n: usize) -> &[i32] {\n    &items[items.len() - n - 1..]\n}\n\n/// The average of the items.\npub fn average(items: &[i32]) -> i32 {\n    items.iter().sum::<i32>() / items.len() as i32\n}\n",
    )
    .unwrap();
    git(&["commit", "-q", "-am", "Add average, adjust last"]);
    let head = git(&["rev-parse", "HEAD"]);

    let context = scratch.path().join("context");
    std::fs::create_dir_all(&context).unwrap();
    let patch = git(&["diff", "-M", &base, &head]) + "\n";
    std::fs::write(context.join("diff.patch"), &patch).unwrap();
    std::fs::write(context.join("files.json"), json!([{"path": "src/lib.rs", "status": "modified", "additions": 6, "deletions": 1}]).to_string()).unwrap();
    std::fs::write(context.join("pr.json"), json!({"number": 1, "title": "Add average, adjust last", "body": "", "changedFiles": 1, "omittedFiles": []}).to_string()).unwrap();
    std::fs::write(context.join("threads.json"), "[]").unwrap();
    let (placed, context_name) = place_context(&context, &repo).unwrap();

    let registry = Arc::new(AdapterRegistry::built_in().unwrap());
    // OpenCode discovers its catalog in the background; a signed-in provider
    // with many models can take longer than the default wait.
    let wait = std::env::var("BRIDGE_REVIEW_LIVE_WAIT_SECS").ok().and_then(|secs| secs.parse().ok()).unwrap_or(8);
    std::thread::sleep(Duration::from_secs(wait));
    if let Some(descriptor) = registry.descriptor(&harness) {
        let usable: Vec<&str> =
            descriptor.models.iter().filter(|model| model.available).map(|model| model.id.as_str()).take(8).collect();
        eprintln!("{harness} catalog: {} models, usable {usable:?}", descriptor.models.len());
    }
    let transport = RegistryTransport { registry };
    let instructions = instructions(&conventions_in(&placed), REVIEW_INSTRUCTIONS, false);
    let start = SessionStart { cwd: &repo, model: model.as_deref(), effort: Some("medium"), instructions: &instructions };
    let pr = read_json(&placed.join("pr.json")).unwrap();
    let started = Instant::now();
    let (result, usage) = run(&transport, &harness, &start, &prompt(&pr, &context_name), &CancelToken::new(), Limits::default());
    eprintln!("{harness}: {:?} in {:?}\nusage: {usage:#?}", result.as_ref().map(|review| review.findings.len()), started.elapsed());
    let review = result.expect("a valid findings set");
    let changed = ["src/lib.rs".to_owned()].into_iter().collect();
    let validated = contract::validate(review, &changed, &contract::hunks(&patch));
    eprintln!("{:#?}", validated.findings);
    assert!(!validated.findings.is_empty(), "the planted bug is found");
    if harness != "codex" {
        assert!(
            usage.tools.iter().all(|name| !name.to_lowercase().contains("bash") && !name.to_lowercase().contains("shell")),
            "no shell tool: {:?}",
            usage.tools
        );
    }
}

/// A real harness re-reviews a fixture PR (#1122's acceptance, by hand). The
/// first commit was reviewed: its divide-by-zero finding was kept and posted,
/// its integer-division finding dropped. The new commit fixes the first and
/// adds a slice that panics. Expect one new finding, the posted one in
/// `fixed`, and the dropped one not raised again. Same environment as
/// [`live_review_on_a_fixture_pr`]:
///
/// ```text
/// BRIDGE_REVIEW_LIVE_HARNESS=claude \
///   cargo test -p bridge-core --lib reviews::session::tests::live_incremental_review_on_a_fixture_pr -- --ignored --nocapture
/// ```
#[test]
#[ignore = "live: runs a real harness; set BRIDGE_REVIEW_LIVE_HARNESS"]
fn live_incremental_review_on_a_fixture_pr() {
    let Ok(harness) = std::env::var("BRIDGE_REVIEW_LIVE_HARNESS") else { return };
    let model = std::env::var("BRIDGE_REVIEW_LIVE_MODEL").ok();
    let scratch = tempfile::tempdir().unwrap();
    if harness == "opencode" && std::env::var_os("BRIDGE_REVIEW_LIVE_OPENCODE_SIGNED_IN").is_none() {
        std::env::set_var("XDG_DATA_HOME", scratch.path().join("xdg-data"));
    }
    let repo = std::fs::canonicalize(scratch.path()).unwrap().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let git = |args: &[&str]| {
        let output = crate::git::git_command(&repo).args(args).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "user.name", "T"]);
    let last = "/// The last `n` items, or all of them when there are fewer.\npub fn last(items: &[i32], n: usize) -> &[i32] {\n    &items[items.len().saturating_sub(n)..]\n}\n";
    std::fs::write(repo.join("src/lib.rs"), last).unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    // Reviewed and posted: `average` divides by zero on an empty slice.
    let average = "\n/// The average of the items.\npub fn average(items: &[i32]) -> i32 {\n    items.iter().sum::<i32>() / items.len() as i32\n}\n";
    std::fs::write(repo.join("src/lib.rs"), format!("{last}{average}")).unwrap();
    git(&["commit", "-q", "-am", "Add average"]);
    let reviewed = git(&["rev-parse", "HEAD"]);
    // The new commit: the empty case is handled, and `first` panics when
    // n is larger than the slice.
    let fixed = "\n/// The average of the items, or 0 for none.\npub fn average(items: &[i32]) -> i32 {\n    if items.is_empty() {\n        return 0;\n    }\n    items.iter().sum::<i32>() / items.len() as i32\n}\n\n/// The first `n` items.\npub fn first(items: &[i32], n: usize) -> &[i32] {\n    &items[..n]\n}\n";
    std::fs::write(repo.join("src/lib.rs"), format!("{last}{fixed}")).unwrap();
    git(&["commit", "-q", "-am", "Handle no items, add first"]);
    let head = git(&["rev-parse", "HEAD"]);

    let context = scratch.path().join("context");
    std::fs::create_dir_all(&context).unwrap();
    let patch = git(&["diff", "-M", &base, &head]) + "\n";
    std::fs::write(context.join("diff.patch"), &patch).unwrap();
    std::fs::write(context.join(INCREMENTAL_PATCH), git(&["diff", "-M", &reviewed, &head]) + "\n").unwrap();
    std::fs::write(context.join("files.json"), json!([{"path": "src/lib.rs", "status": "modified", "additions": 13, "deletions": 0}]).to_string()).unwrap();
    std::fs::write(
        context.join("pr.json"),
        json!({"number": 1, "title": "Add average and first", "body": "", "changedFiles": 1, "omittedFiles": [],
            "incremental": {"from": reviewed, "to": head, "fellBack": false}})
        .to_string(),
    )
    .unwrap();
    std::fs::write(context.join("threads.json"), "[]").unwrap();
    std::fs::write(
        context.join(PRIOR_FINDINGS),
        json!([
            {"id": "f-divide-by-zero", "reviewedHead": reviewed, "path": "src/lib.rs", "line": 8, "startLine": null, "side": "RIGHT",
             "severity": "major", "title": "average divides by zero on an empty slice",
             "body": "`items.len()` is 0 for an empty slice, so this panics. Return early or return an Option.",
             "fingerprint": "f1", "decision": "kept", "posted": true, "fixedInSha": null,
             "thread": {"resolved": false, "outdated": false, "replies": []}},
            {"id": "f-integer-division", "reviewedHead": reviewed, "path": "src/lib.rs", "line": 8, "startLine": null, "side": "RIGHT",
             "severity": "minor", "title": "Integer division truncates the average",
             "body": "The average of [1, 2] is 1, not 1.5. Consider returning f64.",
             "fingerprint": "f2", "decision": "dropped", "posted": false, "fixedInSha": null, "thread": null},
        ])
        .to_string(),
    )
    .unwrap();
    let (placed, context_name) = place_context(&context, &repo).unwrap();

    let registry = Arc::new(AdapterRegistry::built_in().unwrap());
    let wait = std::env::var("BRIDGE_REVIEW_LIVE_WAIT_SECS").ok().and_then(|secs| secs.parse().ok()).unwrap_or(8);
    std::thread::sleep(Duration::from_secs(wait));
    let transport = RegistryTransport { registry };
    let instructions = instructions(&conventions_in(&placed), REVIEW_INSTRUCTIONS, false);
    let start = SessionStart { cwd: &repo, model: model.as_deref(), effort: Some("medium"), instructions: &instructions };
    let pr = read_json(&placed.join("pr.json")).unwrap();
    let started = Instant::now();
    let (result, usage) = run(&transport, &harness, &start, &prompt(&pr, &context_name), &CancelToken::new(), Limits::default());
    eprintln!("{harness}: {:?} in {:?}\nusage: {usage:#?}", result.as_ref().map(|review| review.findings.len()), started.elapsed());
    let review = result.expect("a valid findings set");
    eprintln!("summary: {}\nfixed: {:?}", review.summary, review.fixed);
    let changed = ["src/lib.rs".to_owned()].into_iter().collect();
    let validated = contract::validate(review, &changed, &contract::hunks(&patch));
    let prior = read_prior(&placed).unwrap();
    let raised = validated.findings.len();
    let findings = without_repeats(validated.findings, &prior);
    let fixed = known_fixed(&validated.fixed, &prior);
    eprintln!("raised {raised}, kept after repeats {}, fixed {fixed:?}\n{findings:#?}", findings.len());
    assert!(fixed.contains(&"f-divide-by-zero".to_owned()), "the posted finding the new commit fixed");
    assert!(findings.iter().any(|finding| finding.title.to_lowercase().contains("first") || finding.body.contains("first")), "the new bug in `first`");
}

#[test]
fn a_planted_path_in_the_checkout_is_never_followed() {
    let scratch = tempfile::tempdir().unwrap();
    let checkout = scratch.path().join("checkout");
    let context = scratch.path().join("context");
    let outside = scratch.path().join("outside");
    for dir in [&checkout, &context, &outside] {
        std::fs::create_dir_all(dir).unwrap();
    }
    for file in ["pr.json", "diff.patch", "files.json", "threads.json"] {
        std::fs::write(context.join(file), "{}").unwrap();
    }
    std::fs::write(outside.join("pr.json"), "canary").unwrap();
    // The PR committed the folder name as a link out of the checkout.
    std::os::unix::fs::symlink(&outside, checkout.join(".bridge-review-planted")).unwrap();
    assert!(place_context_as(&context, &checkout, ".bridge-review-planted").is_err(), "an existing path is refused, not followed");
    // Or a real folder holding a link where a context file goes.
    std::fs::create_dir(checkout.join(".bridge-review-folder")).unwrap();
    std::os::unix::fs::symlink(outside.join("pr.json"), checkout.join(".bridge-review-folder/pr.json")).unwrap();
    assert!(place_context_as(&context, &checkout, ".bridge-review-folder").is_err());
    assert_eq!(std::fs::read_to_string(outside.join("pr.json")).unwrap(), "canary", "nothing outside the checkout was written");
    // A fresh name is a fresh folder every time.
    let (first, _) = place_context(&context, &checkout).unwrap();
    let (second, _) = place_context(&context, &checkout).unwrap();
    assert_ne!(first, second);
    assert!(std::fs::symlink_metadata(first.join("pr.json")).unwrap().file_type().is_file());
    assert!(conventions_in(&first).is_empty(), "no conventions folder when the base branch has none");
    // The base branch's conventions travel with the context.
    std::fs::create_dir(context.join("conventions")).unwrap();
    std::fs::write(context.join("conventions/AGENTS.md"), "base rules").unwrap();
    let (third, _) = place_context(&context, &checkout).unwrap();
    assert_eq!(std::fs::read_to_string(third.join("conventions/AGENTS.md")).unwrap(), "base rules");
    assert_eq!(conventions_in(&third), ["AGENTS.md"]);
}

#[test]
fn each_invocation_counts_once_and_codex_commands_count_too() {
    let running = |id: &str, status: &str| NormalizedEvent { item_id: Some(id.into()), status: Some(status.into()), title: Some("Read".into()), ..event("tool.started") };
    let command = |id: &str| NormalizedEvent { item_id: Some(id.into()), data: json!({"command": "sed -n 1,40p src/a.rs"}), ..event("command.started") };
    let mut events = vec![event("turn.started")];
    for index in 0..3 {
        let id = format!("toolu_{index}");
        events.push(running(&id, "preparing"));
        events.push(running(&id, "running"));
    }
    events.push(command("cmd_1"));
    events.push(command("cmd_2"));
    events.push(said("m", &answer(json!([]))));
    events.push(event("turn.completed"));
    let counted = script(vec![vec![Step::Events(events)]]);
    let (result, usage) = run(&ScriptedTransport(counted), "codex", &start(Path::new("/tmp")), "go", &CancelToken::new(), Limits::default());
    assert!(result.is_ok());
    assert_eq!(usage.tool_calls, 5, "three tools (two events each) and two commands");
    assert_eq!(usage.tools[3], "sed -n 1,40p src/a.rs");

    let many: Vec<NormalizedEvent> = (0..3).map(|index| command(&format!("cmd_{index}"))).collect();
    let codex = script(vec![vec![Step::Events([vec![event("turn.started")], many].concat())]]);
    let limits = Limits { max_wall: Duration::from_secs(60), max_tool_calls: 2 };
    let (result, _) = run(&ScriptedTransport(codex), "codex", &start(Path::new("/tmp")), "go", &CancelToken::new(), limits);
    assert_eq!(result.unwrap_err(), SessionError::ToolLimit(2), "Codex commands are under the cap");
}

#[test]
fn a_failed_pass_still_records_what_it_spent() {
    let scratch = tempfile::tempdir().unwrap();
    let core = BridgeCore::for_tests(scratch.path());
    let checkout = scratch.path().join("checkout");
    let context = scratch.path().join("context");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(&context).unwrap();
    std::fs::write(context.join("pr.json"), json!({"number": 1, "title": "t", "changedFiles": 0, "omittedFiles": []}).to_string()).unwrap();
    std::fs::write(context.join("files.json"), "[]").unwrap();
    std::fs::write(context.join("diff.patch"), "").unwrap();
    std::fs::write(context.join("threads.json"), "[]").unwrap();
    let job = job(&core);
    let prepared = Prepared { worktree: Some(checkout), context: Some(context), head_sha: "h".into(), ..Prepared::default() };
    let broken = || {
        vec![Step::Events(vec![
            event("turn.started"),
            said("m", "no json here"),
            with_data("usage.updated", json!({"usage": {"input_tokens": 100, "output_tokens": 10}, "totalCostUsd": 0.002})),
            event("turn.completed"),
        ])]
    };
    let script = script(vec![broken(), broken()]);
    let meter = crate::reviews::steps::UsageMeter::default();
    let request = PassRequest { job: &job, prepared: &prepared, pass_id: "p", harness: "claude", usage: &meter };
    let failed = review_pass_with(&core, &ScriptedTransport(script), request, &CancelToken::new(), Limits::default());
    assert!(matches!(failed, Err(StepError::Fatal(_))));
    assert_eq!(
        meter.total(),
        PassUsage { tokens_in: 200, tokens_out: 20, cost_microusd: 4_000, denied_requests: 0 },
        "both turns of a failed pass are on its meter"
    );
}

// --- re-reviews ------------------------------------------------------------------

fn prior(id: &str, title: &str, decision: &str, posted: bool, resolved: Option<bool>) -> Prior {
    serde_json::from_value(json!({
        "id": id, "path": "src/a.rs", "title": title, "severity": "major", "fingerprint": format!("fp-{id}"),
        "decision": decision, "posted": posted, "fixedInSha": null,
        "thread": resolved.map(|resolved| json!({"resolved": resolved, "outdated": false, "replies": []})),
    }))
    .unwrap()
}

fn draft(path: &str, title: &str, fingerprint: &str, severity: ReviewSeverity) -> DraftFinding {
    DraftFinding {
        path: path.into(),
        line: 2,
        start_line: None,
        side: ReviewSide::Right,
        severity,
        title: title.into(),
        body: "b".into(),
        confidence: 0.8,
        fingerprint: fingerprint.into(),
        outside_diff: false,
    }
}

#[test]
fn a_re_review_leaves_out_what_was_settled_unless_it_got_worse() {
    use ReviewSeverity::{Critical, Major, Nit};
    let priors = [
        prior("dropped", "Dropped thing", "dropped", false, None),
        prior("posted", "Posted thing", "kept", true, Some(false)),
        prior("resolved", "Resolved thing", "edited", true, Some(true)),
        prior("drafted", "Waiting thing", "kept", false, None),
    ];
    // (finding, kept)
    let cases = [
        (draft("src/a.rs", "Other words", "fp-dropped", Major), false),
        (draft("src/a.rs", "Dropped  thing!", "fp-new", Major), false),
        (draft("src/b.rs", "Dropped thing", "fp-new", Major), true),
        (draft("src/a.rs", "Posted thing", "fp-new", Major), false),
        (draft("src/a.rs", "Posted thing", "fp-new", Nit), false),
        (draft("src/a.rs", "Posted thing", "fp-new", Critical), true),
        (draft("src/a.rs", "Resolved thing", "fp-resolved", Major), false),
        (draft("src/a.rs", "Waiting thing", "fp-drafted", Major), true),
        (draft("src/a.rs", "Brand new", "fp-brand-new", Major), true),
    ];
    for (index, (finding, kept)) in cases.into_iter().enumerate() {
        let left = without_repeats(vec![finding.clone()], &priors);
        assert_eq!(left.len() == 1, kept, "case {index}: {finding:?}");
    }
    assert_eq!(without_repeats(vec![draft("src/a.rs", "Brand new", "fp-x", Major)], &[]).len(), 1, "a full review has no memory");
}

#[test]
fn only_earlier_findings_that_could_be_fixed_are_marked_fixed() {
    let mut fixed_already = prior("old", "Old", "kept", true, None);
    fixed_already.fixed_in_sha = Some("abc".into());
    let priors = [
        prior("kept", "Kept", "kept", false, None),
        prior("edited", "Edited", "edited", false, None),
        prior("posted", "Posted", "pending", true, Some(false)),
        prior("dropped", "Dropped", "dropped", false, None),
        prior("pending", "Pending", "pending", false, None),
        fixed_already,
    ];
    let said = ["kept", "dropped", "unknown", "kept", "posted", "pending", "old", "edited"].map(String::from);
    assert_eq!(known_fixed(&said, &priors), ["kept", "posted", "edited"]);
}

#[test]
fn a_re_review_turn_keeps_the_stable_prefix_and_adds_its_files_last() {
    let full = json!({"number": 42, "title": "Parse input", "changedFiles": 2, "omittedFiles": []});
    let mut incremental = full.clone();
    incremental["incremental"] = json!({"from": "aaaaaaaaaaaaaaaa", "to": "bbbbbbbbbbbbbbbb", "fellBack": false});
    let turn = prompt(&incremental, ".bridge-review-0a1b2c3d4e5f");
    assert!(turn.starts_with(&prompt(&full, ".bridge-review-0a1b2c3d4e5f")), "what a full review says comes first, unchanged");
    assert!(turn.contains(".bridge-review-0a1b2c3d4e5f/incremental.patch") && turn.contains("`aaaaaaaaaaaa..bbbbbbbbbbbb`"));
    assert!(turn.contains(".bridge-review-0a1b2c3d4e5f/prior_findings.json") && turn.contains("`fixed`"));
    let mut fell_back = full.clone();
    fell_back["incremental"] = json!({"from": "aaaaaaaaaaaaaaaa", "to": "bbbbbbbbbbbbbbbb", "fellBack": true});
    let turn = prompt(&fell_back, "ctx");
    assert!(turn.contains("no longer in the pull request's history") && !turn.contains("incremental.patch"));
    assert!(turn.contains("ctx/prior_findings.json"), "the memory stays after a force-push");
    // The cached prefix never mentions a re-review: it is the same for both.
    assert!(OUTPUT_CONTRACT.contains("\"fixed\""));
}

#[test]
fn a_re_review_pass_drops_repeats_and_reports_what_the_new_commits_fixed() {
    let scratch = tempfile::tempdir().unwrap();
    let core = BridgeCore::for_tests(scratch.path());
    let checkout = scratch.path().join("checkout");
    let context = scratch.path().join("context");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(&context).unwrap();
    let pr = json!({"number": 42, "title": "Parse input", "changedFiles": 1, "omittedFiles": [],
        "incremental": {"from": "aaaaaaaaaaaaaaaa", "to": "bbbbbbbbbbbbbbbb", "fellBack": false}});
    std::fs::write(context.join("pr.json"), pr.to_string()).unwrap();
    std::fs::write(context.join("files.json"), json!([{"path": "src/a.rs", "status": "modified"}]).to_string()).unwrap();
    std::fs::write(context.join("diff.patch"), PATCH).unwrap();
    std::fs::write(context.join("incremental.patch"), PATCH).unwrap();
    std::fs::write(context.join("threads.json"), "[]").unwrap();
    std::fs::write(
        context.join("prior_findings.json"),
        json!([
            {"id": "f-kept", "path": "src/a.rs", "title": "Missing check", "severity": "major", "fingerprint": "x1", "decision": "kept", "posted": true, "thread": null},
            {"id": "f-dropped", "path": "src/a.rs", "title": "Style nit", "severity": "nit", "fingerprint": "x2", "decision": "dropped", "posted": false, "thread": null},
        ])
        .to_string(),
    )
    .unwrap();
    let job = job(&core);
    let prepared = Prepared { worktree: Some(checkout.clone()), context: Some(context), head_sha: "h".into(), ..Prepared::default() };
    let reply = format!(
        "```json\n{}\n```",
        json!({"summary": "The new commit fixes the missing check.", "fixed": ["f-kept", "f-dropped", "made-up"], "findings": [
            {"path": "src/a.rs", "line": 2, "side": "RIGHT", "severity": "nit", "title": "Style nit", "body": "Again.", "confidence": 0.4},
            {"path": "src/a.rs", "line": 2, "side": "RIGHT", "severity": "major", "title": "Unwrap on user input", "body": "New.", "confidence": 0.9},
        ]})
    );
    let script = script(vec![vec![Step::Events(vec![event("turn.started"), said("m", &reply), event("turn.completed")])]]);
    let meter = crate::reviews::steps::UsageMeter::default();
    let request = PassRequest { job: &job, prepared: &prepared, pass_id: "p", harness: "claude", usage: &meter };
    let outcome = review_pass_with(&core, &ScriptedTransport(script.clone()), request, &CancelToken::new(), Limits::default()).unwrap();
    let titles: Vec<&str> = outcome.findings.iter().map(|finding| finding.title.as_str()).collect();
    assert_eq!(titles, ["Unwrap on user input"], "the dropped nit is not raised again");
    assert_eq!(outcome.fixed, ["f-kept"], "only a known finding that could be fixed");
    let placed: Vec<_> = std::fs::read_dir(&checkout).unwrap().filter_map(Result::ok).map(|entry| entry.path()).collect();
    assert!(placed[0].join("incremental.patch").is_file() && placed[0].join("prior_findings.json").is_file(), "the re-review files reach the reviewer");
    assert!(script.sent.lock().unwrap()[0].contains("This is a re-review."));
}
