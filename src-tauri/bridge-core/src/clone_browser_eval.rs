//! Opt-in browser-selection evals through Bridge's production Codex and Claude
//! transports. Each case gets a fresh provider session and an empty cwd, so
//! repository guidance cannot decide the answer. No browser or cookie import
//! runs: the clone helper has no orchestrator behind it.
use crate::{
    adapters::{AdapterRuntime, ShutdownReason, StartRequest, TurnContext},
    browser_clone::CloneSupervisor,
    claude_adapter,
    clone_browser_tool::CloneBrowserTool,
    codex_adapter,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::ChildStdout,
    sync::mpsc,
    time::{Duration, Instant},
};

/// A local page with a signup form, standing in for the app under test.
const SIGNUP_PAGE: &str = r#"<!doctype html><title>Signup</title><h1>Create account</h1><form><label>Email <input type="email" name="email"></label><label>Password <input type="password" name="password"></label><button>Sign up</button></form>"#;

/// What one provider frame means for an eval.
enum Frame {
    Continue,
    Text(String),
    /// A tool call or provider request, serialized for inspection.
    Tool(String),
    Done,
    Fail(String),
}

enum End {
    Completed,
    /// The eval stopped the turn once it had seen enough.
    Stopped,
    Deadline,
    Error(String),
}

struct Turn {
    end: End,
    reply: String,
    tools: Vec<String>,
}

struct Harness {
    tool: std::sync::Arc<CloneBrowserTool>,
    tools_dir: std::path::PathBuf,
    /// A previous context to compare against, with `{tool}` for the helper.
    baseline: Option<String>,
    _scratch: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let scratch = tempfile::tempdir_in("/tmp").unwrap();
        let tools_dir = scratch.path().join("tools");
        let tool = CloneBrowserTool::new(
            CloneSupervisor::guarded(scratch.path().join("ledger.json")),
            tools_dir.clone(),
        )
        .unwrap();
        let baseline = std::env::var_os("BRIDGE_BROWSER_EVAL_CONTEXT")
            .map(|path| std::fs::read_to_string(path).unwrap());
        Self { tool, tools_dir, baseline, _scratch: scratch }
    }

    /// Runs one turn on a fresh session, delivering the clone context through
    /// the same trusted-context path live turns use for each provider.
    fn run(
        &self,
        provider: &str,
        session: &str,
        cwd: &Path,
        prompt: &str,
        deadline: Duration,
        stop: impl Fn(&[String]) -> bool,
    ) -> Turn {
        let (mut runtime, mut reader) = start(provider, cwd.to_str().unwrap());
        let context = self
            .tool
            .request_capability_context(session, runtime.process_id())
            .unwrap();
        let context = self
            .baseline
            .as_ref()
            .map(|text| {
                let helper = self.tools_dir.join(format!("clone-request-{session}"));
                text.replace("{tool}", &helper.display().to_string())
            })
            .unwrap_or(context);
        runtime
            .send_turn_with_context(
                prompt,
                TurnContext { session: None, credentials: Some(&context) },
            )
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        let reader_thread = std::thread::spawn(move || {
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) != 0 {
                if sender.send(line.clone()).is_err() {
                    break;
                }
                line.clear();
            }
        });
        let deadline = Instant::now() + deadline;
        let mut reply = String::new();
        let mut tools = Vec::new();
        let end = loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break End::Deadline;
            };
            let line = match receiver.recv_timeout(remaining) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => break End::Deadline,
                Err(error) => break End::Error(format!("provider did not complete: {error}")),
            };
            let Ok(frame) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let step = match provider {
                "codex" => codex_frame(&frame),
                _ => claude_frame(&frame),
            };
            match step {
                Frame::Continue => {}
                Frame::Text(text) => reply.push_str(&text),
                Frame::Tool(call) => {
                    tools.push(call);
                    if stop(&tools) {
                        break End::Stopped;
                    }
                }
                Frame::Done => break End::Completed,
                Frame::Fail(error) => break End::Error(error),
            }
        };
        runtime.stop(ShutdownReason::Completed);
        drop(receiver);
        reader_thread.join().unwrap();
        self.tool.revoke_session(session);
        Turn { end, reply, tools }
    }
}

fn providers() -> Vec<String> {
    std::env::var("BRIDGE_BROWSER_EVAL_PROVIDERS")
        .unwrap_or_else(|_| "codex,claude".into())
        .split(',')
        .map(|provider| provider.trim().to_owned())
        .filter(|provider| !provider.is_empty())
        .collect()
}

/// `BRIDGE_BROWSER_EVAL_CASES` (comma-separated ids) reruns a subset, e.g.
/// cases a provider rate limit cut short.
fn selected(id: &str) -> bool {
    std::env::var("BRIDGE_BROWSER_EVAL_CASES")
        .map(|ids| ids.split(',').any(|wanted| wanted.trim() == id))
        .unwrap_or(true)
}

fn start(provider: &str, cwd: &str) -> (Box<dyn AdapterRuntime>, BufReader<ChildStdout>) {
    let request = StartRequest {
        cwd,
        model: None,
        effort: None,
        instructions: None,
        write_mode: None,
        read_only_sandbox: None,
        briefing: None,
        interactive: false,
        account: None,
        on_progress: None,
    };
    match provider {
        "codex" => {
            let started = codex_adapter::start(request).expect("Bridge starts authenticated Codex");
            (Box::new(started.runtime), started.reader)
        }
        "claude" => {
            let started = claude_adapter::start(request).expect("Bridge starts authenticated Claude");
            (Box::new(started.runtime), started.reader)
        }
        other => panic!("unknown eval provider {other}"),
    }
}

fn codex_frame(frame: &Value) -> Frame {
    let method = frame["method"].as_str().unwrap_or("");
    if frame.get("error").is_some() || method == "error" {
        return Frame::Fail(format!("provider error: {frame}"));
    }
    // A server request is an approval prompt for something the model ran.
    if frame.get("id").is_some() && frame.get("method").is_some() {
        return Frame::Tool(frame.to_string());
    }
    if method == "item/started" || method == "item/completed" {
        let item = &frame["params"]["item"];
        return match item["type"].as_str().unwrap_or("") {
            "agentMessage" if method == "item/completed" => {
                Frame::Text(item["text"].as_str().unwrap_or("").to_owned())
            }
            "agentMessage" | "reasoning" | "userMessage" => Frame::Continue,
            _ if method == "item/started" => Frame::Tool(item.to_string()),
            _ => Frame::Continue,
        };
    }
    if method == "turn/completed" {
        if frame["params"]["turn"]["status"] != "completed" {
            return Frame::Fail(format!("turn did not succeed: {frame}"));
        }
        return Frame::Done;
    }
    Frame::Continue
}

fn claude_frame(frame: &Value) -> Frame {
    match frame["type"].as_str().unwrap_or("") {
        // A permission prompt only exists for a tool the model tried to use.
        "control_request" => Frame::Tool(frame.to_string()),
        "assistant" => {
            let mut text = String::new();
            let mut calls = Vec::new();
            for block in frame["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or("") {
                    "text" => text.push_str(block["text"].as_str().unwrap_or("")),
                    "thinking" | "redacted_thinking" => {}
                    _ => calls.push(format!("{} {}", block["name"], block["input"])),
                }
            }
            if calls.is_empty() { Frame::Text(text) } else { Frame::Tool(calls.join("\n")) }
        }
        "result" => {
            if frame["is_error"] == true || frame["subtype"] != "success" {
                return Frame::Fail(format!("turn did not succeed: {frame}"));
            }
            Frame::Done
        }
        _ => Frame::Continue,
    }
}

/// Serves the signup page on an ephemeral loopback port for the whole run.
fn serve_signup_page() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{SIGNUP_PAGE}",
                SIGNUP_PAGE.len()
            );
        }
    });
    port
}

#[derive(Deserialize)]
struct SelectionCase {
    id: String,
    task: String,
    expected: String,
}

/// Asks for the route as one word, so the decision is scored without any tool
/// running. This checks the stated boundary; the behavior eval checks actions.
#[test]
#[ignore = "requires authenticated Codex and Claude; makes paid live model calls"]
fn live_browser_selection_eval() {
    let cases: Vec<SelectionCase> = serde_json::from_str(include_str!(
        "../../../testing/evals/browser-routing/cases.json"
    ))
    .unwrap();
    let harness = Harness::new();
    let mut failures = Vec::new();
    for provider in providers() {
        for case in cases.iter().filter(|case| selected(&case.id)) {
            let cwd = tempfile::tempdir_in("/tmp").unwrap();
            let session = format!("{provider}-{}", case.id);
            let prompt = format!(
                "Choose your next step for this task, without executing it or calling any tools. \
                 Reply with exactly one route word: automation (ordinary/fresh browser automation), \
                 direct (an existing non-browser tool), clone (request a Bridge browser clone), \
                 or blocked (explain missing automation to the user).\nTask: {}",
                case.task,
            );
            let turn = harness.run(&provider, &session, cwd.path(), &prompt, Duration::from_secs(120), |_| true);
            let error = match &turn.end {
                End::Completed => None,
                End::Stopped => Some(format!("unexpected tool attempt: {}", turn.tools[0])),
                End::Deadline => Some("provider deadline exceeded".into()),
                End::Error(error) => Some(error.clone()),
            };
            // The route is the first word; formatting and any explanation after
            // it are not part of the decision.
            let observed = turn
                .reply
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase();
            let correct = error.is_none() && observed == case.expected;
            println!(
                "{} {provider} {} expected={} observed={observed:?} error={error:?}",
                if correct { "PASS" } else { "FAIL" },
                case.id,
                case.expected,
            );
            if !correct {
                failures.push(session);
            }
        }
    }
    assert!(failures.is_empty(), "browser selection failures: {failures:?}");
}

#[derive(Deserialize)]
struct BehaviorCase {
    id: String,
    task: String,
    clone: bool,
}

/// Gives the agent the task as a person would type it, lets it act, and
/// records whether it invokes the clone-request helper. A local signup page is
/// served for the run. The turn stops at the first clone request, after a few
/// tool calls, or at the deadline: the decision is made early, and nothing in
/// these tasks needs the agent to finish.
#[test]
#[ignore = "requires authenticated Codex and Claude; makes paid live model calls and runs agent tools"]
fn live_browser_behavior_eval() {
    let cases: Vec<BehaviorCase> = serde_json::from_str(include_str!(
        "../../../testing/evals/browser-routing/behavior-cases.json"
    ))
    .unwrap();
    let origin = format!("http://127.0.0.1:{}", serve_signup_page());
    let localhost = origin.replace("127.0.0.1", "localhost");
    let harness = Harness::new();
    let mut failures = Vec::new();
    for provider in providers() {
        for case in cases.iter().filter(|case| selected(&case.id)) {
            let cwd = tempfile::tempdir_in("/tmp").unwrap();
            std::fs::write(cwd.path().join("index.html"), SIGNUP_PAGE).unwrap();
            let session = format!("{provider}-{}", case.id);
            let task = case.task.replace("{origin}", &origin).replace("{localhost}", &localhost);
            let requested = |tools: &[String]| {
                tools.iter().any(|call| call.contains("clone-request-"))
            };
            let turn = harness.run(&provider, &session, cwd.path(), &task, Duration::from_secs(240), |tools| {
                requested(tools) || tools.len() >= 8
            });
            let cloned = requested(&turn.tools);
            let error = match &turn.end {
                End::Error(error) => Some(error.clone()),
                _ => None,
            };
            let correct = error.is_none() && cloned == case.clone;
            println!(
                "{} {provider} {} expected_clone={} observed_clone={cloned} tools={} error={error:?} first_tool={:?}",
                if correct { "PASS" } else { "FAIL" },
                case.id,
                case.clone,
                turn.tools.len(),
                turn.tools.first().map(|call| call.chars().take(160).collect::<String>()),
            );
            if !correct {
                failures.push(session);
            }
        }
    }
    assert!(failures.is_empty(), "browser behavior failures: {failures:?}");
}
