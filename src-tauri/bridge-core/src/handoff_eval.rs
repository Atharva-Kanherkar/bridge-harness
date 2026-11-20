//! Offline eval for the model-switch handoff: what a model taking over a chat
//! is handed, and what it can read back.
//!
//! Seeds the labeled chats in `testing/evals/model-switch-handoff/cases.json`
//! through the same writers the adapters use (forest messages, normalized tool
//! events in each adapter's own shape), then builds the handoff for three
//! incoming windows and scores it, with no model:
//!
//! - every labeled file and link is in the facts block;
//! - the goal line holds the latest ask;
//! - the newest tool run is named in a `tools:` line;
//! - the fact said early is reachable: in the handoff, or in a `chats_history`
//!   page read back from the `before` the handoff names;
//! - no handoff and no page holds twelve characters of a credential planted in
//!   a tool command or output;
//! - the handoff is within the incoming model's budget.
//!
//! ```text
//! cargo test -p bridge-core --lib handoff_eval -- --nocapture
//! ```
//!
//! The live half runs against the real daemon: see
//! `testing/evals/model-switch-handoff/README.md`.

use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::history_tools;
use crate::restoration;
use crate::session_forest::{EntryKind, SessionForest};

const CASES: &str = include_str!("../../../testing/evals/model-switch-handoff/cases.json");
const ENVELOPE: &str = "Bridge checkpoint-restoration context (stored history, not native provider resume):\n";

#[derive(Deserialize)]
struct Corpus {
    windows: Vec<i64>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    steps: Vec<Step>,
    expect: Expect,
    secrets: Vec<String>,
}

#[derive(Deserialize)]
struct Expect {
    files: Vec<String>,
    links: Vec<String>,
    fact: String,
    goal: String,
    tools: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Step {
    User(String),
    Assistant(String),
    Filler { turns: usize, bytes: usize, text: String },
    ClaudeBash { command: String, output: String, error: bool },
    ClaudeEdit { path: String },
    CodexCommand { command: String, output: String, exit: i64 },
    CodexChange { paths: Vec<String> },
    OpencodeShell { command: String, output: String, exit: i64 },
    OpencodeEdit { path: String },
    Worker {
        summary: String,
        #[serde(rename = "filesChanged")]
        files_changed: Vec<String>,
    },
}

/// The credentials `{{secret:NAME}}` stands for, generated so no token-shaped
/// literal is committed.
fn secrets() -> Vec<(&'static str, String)> {
    vec![
        ("anthropic", format!("sk-ant-api03-{}", "Hq7wP2xLr9".repeat(4))),
        ("openai", format!("sk-proj-{}", "Vb4Nm6Kj8H".repeat(3))),
        ("github", format!("ghp_{}", "Xy9Zw8".repeat(7))),
    ]
}

fn substitute(text: &str) -> String {
    let mut out = text.to_owned();
    for (name, value) in secrets() {
        out = out.replace(&format!("{{{{secret:{name}}}}}"), &value);
    }
    assert!(!out.contains("{{secret:"), "an unknown secret name in: {text}");
    out
}

/// Whether any twelve characters of `secret` appear in `output`.
fn leaked(output: &str, secret: &str) -> bool {
    let chars: Vec<char> = secret.chars().collect();
    chars.windows(12.min(chars.len())).any(|piece| output.contains(&piece.iter().collect::<String>()))
}

fn tool(db: &Connection, session: &str, kind: &str, item: &str, title: Option<&str>, text: Option<&str>, data: Value) {
    let mut event = crate::agent::NormalizedEvent::new(kind);
    event.item_id = Some(item.to_owned());
    event.title = title.map(str::to_owned);
    event.text = text.map(str::to_owned);
    event.status = Some(if kind.ends_with(".started") { "inProgress" } else { "completed" }.into());
    event.data = data;
    crate::store::session_event(db, session, &event, &json!({})).unwrap();
}

/// Write `case` into `session` the way each adapter would have.
fn seed(db: &Connection, session: &str, case: &Case) {
    db.execute(
        "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,kind) VALUES(?1,NULL,'claude','eval','idle','reported','direct')",
        params![session],
    )
    .unwrap();
    let say = |kind: EntryKind, text: &str| {
        SessionForest::new(db).append(session, kind, json!({"text": text})).unwrap();
    };
    for (index, step) in case.steps.iter().enumerate() {
        let item = format!("item-{index}");
        match step {
            Step::User(text) => say(EntryKind::UserMessage, text),
            Step::Assistant(text) => say(EntryKind::AssistantMessage, text),
            Step::Filler { turns, bytes, text } => {
                for turn in 0..*turns {
                    let kind = if turn % 2 == 0 { EntryKind::AssistantMessage } else { EntryKind::UserMessage };
                    let mut body = format!("filler {turn}: {text} ");
                    while body.len() < *bytes {
                        body.push_str(text);
                        body.push(' ');
                    }
                    say(kind, &body);
                }
            }
            Step::ClaudeBash { command, output, error } => {
                let (command, output) = (substitute(command), substitute(output));
                tool(db, session, "command.started", &item, Some("Bash"), None, json!({"type": "tool_use", "name": "Bash", "input": {}, "phase": "preparing"}));
                tool(db, session, "command.started", &item, Some(&command), None, json!({"type": "tool_use", "name": "Bash", "input": {"command": command}, "phase": "running"}));
                tool(db, session, "command.completed", &item, None, Some(&output), json!({"type": "tool_result", "tool_use_id": item, "content": output, "is_error": error}));
            }
            Step::ClaudeEdit { path } => {
                tool(db, session, "file_change.started", &item, Some("Edit"), None, json!({"type": "tool_use", "name": "Edit", "input": {"file_path": path, "old_string": "a", "new_string": "b"}}));
                tool(db, session, "file_change.completed", &item, None, Some(&format!("The file {path} has been updated.")), json!({"type": "tool_result", "is_error": false}));
            }
            Step::CodexCommand { command, output, exit } => {
                let (command, output) = (substitute(command), substitute(output));
                tool(db, session, "command.started", &item, Some(&command), None, json!({"type": "commandExecution", "command": command, "status": "inProgress"}));
                tool(db, session, "command.completed", &item, Some(&command), None, json!({"type": "commandExecution", "command": command, "aggregatedOutput": output, "exitCode": exit}));
            }
            Step::CodexChange { paths } => {
                let changes = paths.iter().map(|path| json!({"path": path, "kind": {"type": "update"}, "diff": "@@"})).collect::<Vec<_>>();
                tool(db, session, "file_change.completed", &item, None, None, json!({"type": "fileChange", "changes": changes, "status": "completed"}));
            }
            Step::OpencodeShell { command, output, exit } => {
                let (command, output) = (substitute(command), substitute(output));
                tool(db, session, "command.started", &item, Some("shell"), None, json!({"type": "tool", "tool": "shell", "state": {"status": "pending", "input": {}}}));
                tool(db, session, "command.completed", &item, Some("shell"), Some(&output), json!({"type": "tool", "tool": "shell", "state": {"status": "completed", "input": {"command": command}, "output": output, "metadata": {"exit": exit}}}));
            }
            Step::OpencodeEdit { path } => {
                tool(db, session, "file_change.completed", &item, Some("edit"), None, json!({"type": "tool", "tool": "edit", "state": {"status": "completed", "input": {"filePath": path, "oldString": "a", "newString": "b"}}}));
            }
            Step::Worker { summary, files_changed } => {
                SessionForest::new(db)
                    .append(session, EntryKind::WorkerResult, json!({"status": "completed", "summary": summary, "filesChanged": files_changed}))
                    .unwrap();
            }
        }
    }
}

/// Every page of `chats_history` from `before` back to the start.
fn read_back(db: &Connection, session: &str, before: i64) -> Vec<String> {
    let mut pages = Vec::new();
    let mut next = Some(before);
    while let Some(before) = next {
        let page = history_tools::run(db, session, history_tools::HISTORY, &json!({"before": before, "limit": 50}), chrono::Utc::now())
            .expect("a top-level chat reads its own history");
        next = page["nextBefore"].as_i64();
        pages.push(page.to_string());
        assert!(pages.len() < 200, "paging must end");
    }
    pages
}

/// The `before` an `earlier:` line names.
fn named_before(handoff: &str) -> Option<i64> {
    let line = handoff.lines().find(|line| line.starts_with("earlier: "))?;
    let rest = line.split("before=").nth(1)?;
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

#[derive(Default)]
struct Score {
    runs: usize,
    passed: usize,
    files: (usize, usize),
    links: (usize, usize),
    reached_by_tool: usize,
    leaks: Vec<String>,
}

#[test]
fn offline() {
    let corpus: Corpus = serde_json::from_str(CASES).expect("the corpus parses");
    let dir = tempfile::tempdir().unwrap();
    let db = crate::store::open(&dir.path().join("bridge.db")).unwrap();
    let mut score = Score::default();
    for (index, case) in corpus.cases.iter().enumerate() {
        let session = format!("handoff-eval-{index}");
        seed(&db, &session, case);
        let planted = secrets()
            .into_iter()
            .filter(|(name, _)| case.secrets.iter().any(|wanted| wanted == name))
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        for &window in &corpus.windows {
            score.runs += 1;
            let handoff = restoration::restoration_handoff(&db, &session, window).unwrap().expect("a handoff");
            let text = &handoff.text;
            let body = text.strip_prefix(ENVELOPE).unwrap();
            let mut failures = Vec::new();
            if body.len() > restoration::restoration_budget_bytes(window) {
                failures.push(format!("over budget: {}", body.len()));
            }
            let facts_line = |prefix: &str| body.lines().find(|line| line.starts_with(prefix)).unwrap_or("").to_owned();
            let (files, links) = (facts_line("files touched: "), facts_line("links: "));
            for file in &case.expect.files {
                score.files.1 += 1;
                if files.contains(file.as_str()) {
                    score.files.0 += 1;
                } else {
                    failures.push(format!("file {file}"));
                }
            }
            for link in &case.expect.links {
                score.links.1 += 1;
                if links.split(", ").any(|found| found.trim_start_matches("links: ") == link) {
                    score.links.0 += 1;
                } else {
                    failures.push(format!("link {link}"));
                }
            }
            if !facts_line("goal: ").contains(&case.expect.goal) {
                failures.push("goal".into());
            }
            for expected in &case.expect.tools {
                if !body.lines().any(|line| line.starts_with("tools: ") && line.contains(expected.as_str())) {
                    failures.push(format!("tools {expected}"));
                }
            }
            let mut outputs = vec![text.clone()];
            let reachable = if body.contains(&case.expect.fact) {
                true
            } else if let Some(before) = named_before(body)
                .or_else(|| body.lines().any(|line| line.starts_with("tool output: ")).then_some(i64::MAX))
            {
                let pages = read_back(&db, &session, before);
                let found = pages.iter().any(|page| page.contains(&case.expect.fact));
                score.reached_by_tool += usize::from(found);
                outputs.extend(pages);
                found
            } else {
                false
            };
            if !reachable {
                failures.push("early fact unreachable".into());
            }
            // The newest page too: what a model reads first when it pages back.
            outputs.push(history_tools::run(&db, &session, history_tools::HISTORY, &json!({}), chrono::Utc::now()).unwrap().to_string());
            for secret in &planted {
                for output in &outputs {
                    if leaked(output, secret) {
                        score.leaks.push(format!("{} @ {window}", case.name));
                        failures.push("leak".into());
                    }
                }
            }
            println!(
                "{} {:<56} window {:>8}  {:>3}/{:<3} messages  facts {:>4} B  {}",
                if failures.is_empty() { "PASS" } else { "FAIL" },
                case.name,
                window,
                handoff.carried_entries,
                handoff.eligible_entries,
                body.lines()
                    .filter(|line| ["goal: ", "started with: ", "files touched: ", "links: ", "earlier: ", "tool output: "].iter().any(|p| line.starts_with(p)))
                    .map(|line| line.len() + 1)
                    .sum::<usize>(),
                failures.join(", "),
            );
            score.passed += usize::from(failures.is_empty());
        }
    }
    println!(
        "model-switch handoff: {}/{} runs, files {}/{}, links {}/{}, early facts read back through chats_history {}, leaks {}",
        score.passed, score.runs, score.files.0, score.files.1, score.links.0, score.links.1, score.reached_by_tool, score.leaks.len()
    );
    assert!(score.leaks.is_empty(), "{:?}", score.leaks);
    assert_eq!(score.passed, score.runs);
    assert!(score.reached_by_tool > 0, "at least one case must need chats_history, or the corpus is too short");
}
