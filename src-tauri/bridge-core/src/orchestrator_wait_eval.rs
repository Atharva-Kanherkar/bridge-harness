//! Opt-in live eval for `bridge-wait`: does an orchestrator, given Bridge's
//! production briefing and delegation protocol, follow a parallel fan-out
//! with one wait, and hold off claiming the work is done until the results
//! arrive? Scored from what it emits, through the production parsers, not
//! from what it says it would do.
//!
//! Run: `cargo test -p bridge-core orchestrator_wait_eval -- --ignored --nocapture`
//! with an authenticated Claude, in a dedicated `CARGO_TARGET_DIR`.
//! `BRIDGE_WAIT_EVAL_CASES` reruns a subset by id.
use crate::{
    adapters::{AdapterRuntime, ShutdownReason, StartRequest},
    claude_adapter, delegation, orchestrator,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::ChildStdout,
    sync::mpsc,
    time::{Duration, Instant},
};

const TURN_DEADLINE: Duration = Duration::from_secs(240);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cases {
    fan_out: Vec<FanOutCase>,
    digest: DigestCase,
}

#[derive(Deserialize)]
struct FanOutCase {
    id: String,
    task: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DigestCase {
    id: String,
    task: String,
    still_running_label: String,
    envelope: Value,
}

struct Turn {
    reply: String,
    /// The first tool the model tried to run itself, if any. An orchestrator
    /// that does the work instead of delegating it never reaches the wait.
    tool: Option<String>,
    error: Option<String>,
}

/// One Claude session with the given orchestrator instructions, kept open
/// across turns so a later turn sees the earlier ones.
struct Session {
    runtime: Box<dyn AdapterRuntime>,
    lines: mpsc::Receiver<String>,
    reader: Option<std::thread::JoinHandle<()>>,
    _cwd: tempfile::TempDir,
}

impl Session {
    fn start(instructions: &str) -> Self {
        let cwd = tempfile::tempdir_in("/tmp").unwrap();
        let request = StartRequest {
            session_id: None,
            cwd: cwd.path().to_str().unwrap(),
            model: None,
            effort: None,
            instructions: Some(instructions),
            write_mode: None,
            read_only_sandbox: None,
            briefing: None,
            interactive: false,
            account: None,
            on_progress: None,
        };
        let started = claude_adapter::start(request).expect("Bridge starts authenticated Claude");
        let reader: BufReader<ChildStdout> = started.reader;
        let (sender, lines) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut reader = reader;
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) != 0 {
                if sender.send(line.clone()).is_err() {
                    break;
                }
                line.clear();
            }
        });
        Self { runtime: Box::new(started.runtime), lines, reader: Some(reader), _cwd: cwd }
    }

    fn turn(&mut self, prompt: &str) -> Turn {
        if let Err(error) = self.runtime.send_turn(prompt) {
            return Turn { reply: String::new(), tool: None, error: Some(error.to_string()) };
        }
        let deadline = Instant::now() + TURN_DEADLINE;
        let mut reply = String::new();
        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Turn { reply, tool: None, error: Some("deadline".into()) };
            };
            let line = match self.lines.recv_timeout(remaining) {
                Ok(line) => line,
                Err(error) => return Turn { reply, tool: None, error: Some(format!("provider ended: {error}")) },
            };
            let Ok(frame) = serde_json::from_str::<Value>(&line) else { continue };
            match frame["type"].as_str().unwrap_or("") {
                "control_request" => return Turn { reply, tool: Some(frame.to_string()), error: None },
                "assistant" => {
                    for block in frame["message"]["content"].as_array().into_iter().flatten() {
                        match block["type"].as_str().unwrap_or("") {
                            "text" => reply.push_str(block["text"].as_str().unwrap_or("")),
                            "thinking" | "redacted_thinking" => {}
                            _ => return Turn { reply, tool: Some(format!("{} {}", block["name"], block["input"])), error: None },
                        }
                    }
                }
                "result" => {
                    let failed = frame["is_error"] == true || frame["subtype"] != "success";
                    return Turn { reply, tool: None, error: failed.then(|| format!("turn did not succeed: {frame}")) };
                }
                _ => {}
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.runtime.stop(ShutdownReason::Completed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn instructions(with_wait_guidance: bool) -> String {
    let protocol = delegation::protocol(0);
    let protocol = if with_wait_guidance { protocol } else { protocol.replace(delegation::WAIT_GUIDANCE, "") };
    format!("{}\n\n{}", orchestrator::briefing(), protocol)
}

fn selected(id: &str) -> bool {
    std::env::var("BRIDGE_WAIT_EVAL_CASES")
        .map(|ids| ids.split(',').any(|wanted| wanted.trim() == id))
        .unwrap_or(true)
}

/// A claim that the whole task is finished, in the prose around the blocks.
fn claims_done(prose: &str) -> bool {
    let prose = prose.to_lowercase();
    [
        "all three are done",
        "all three are complete",
        "all three have finished",
        "all three reports are",
        "everything is done",
        "everything is complete",
        "all done",
        "the task is complete",
        "the work is complete",
        "all of them are done",
        "all the write-ups are",
        "here are all three",
    ]
    .iter()
    .any(|claim| prose.contains(claim))
}

fn delegations(reply: &str) -> usize {
    match delegation::parse_delegation_requests(reply) {
        delegation::ParseOutcome::Parsed(parsed) => parsed.requests.len(),
        _ => 0,
    }
}

fn waits(reply: &str) -> bool {
    matches!(delegation::parse_wait_request(reply), delegation::ParseOutcome::Parsed(_))
}

fn prose(reply: &str) -> String {
    delegation::strip_wait(&delegation::strip_directives(reply))
}

#[test]
#[ignore = "requires authenticated Claude; makes paid live model calls"]
fn orchestrator_wait_eval() {
    let cases: Cases = serde_json::from_str(include_str!("../../../testing/evals/orchestrator-wait/cases.json")).unwrap();
    let mut failures = Vec::new();

    for case in cases.fan_out.iter().filter(|case| selected(&case.id)) {
        // With the guidance: two or more delegations, then one valid wait, and
        // no claim that the work is done.
        let turn = Session::start(&instructions(true)).turn(&case.task);
        let (count, wait, done) = (delegations(&turn.reply), waits(&turn.reply), claims_done(&prose(&turn.reply)));
        let pass = turn.error.is_none() && turn.tool.is_none() && count >= 2 && wait && !done;
        println!(
            "{} {} delegations={count} wait={wait} claimsDone={done} tool={:?} error={:?}\n--- reply ---\n{}\n",
            if pass { "PASS" } else { "FAIL" },
            case.id,
            turn.tool,
            turn.error,
            turn.reply.trim(),
        );
        if !pass {
            failures.push(case.id.clone());
        }

        // Control: the same task without the guidance must not produce a
        // wait, or the guidance is not what the first run measured.
        let control = Session::start(&instructions(false)).turn(&case.task);
        let control_wait = waits(&control.reply);
        println!(
            "{} {}-control wait={control_wait} delegations={} error={:?}\n",
            if control_wait { "FAIL" } else { "PASS" },
            case.id,
            delegations(&control.reply),
            control.error,
        );
        if control_wait {
            failures.push(format!("{}-control", case.id));
        }
    }

    let digest = &cases.digest;
    if selected(&digest.id) {
        // Two turns in one session: the fan-out, then the timed-out delivery.
        let mut session = Session::start(&instructions(true));
        let first = session.turn(&digest.task);
        let second = session.turn(&digest.envelope.to_string());
        let reply = second.reply.to_lowercase();
        let names_running = reply.contains(&digest.still_running_label.to_lowercase())
            && ["still", "running", "pending", "in progress", "not finished", "hasn't", "has not", "not yet"].iter().any(|word| reply.contains(word));
        let done = claims_done(&second.reply);
        let pass = first.error.is_none() && second.error.is_none() && names_running && !done;
        println!(
            "{} {} firstWait={} namesStillRunning={names_running} claimsDone={done} error={:?}\n--- reply ---\n{}\n",
            if pass { "PASS" } else { "FAIL" },
            digest.id,
            waits(&first.reply),
            second.error,
            second.reply.trim(),
        );
        if !pass {
            failures.push(digest.id.clone());
        }
    }

    assert!(failures.is_empty(), "bridge-wait eval failures: {failures:?}");
}

#[test]
fn the_cases_parse_and_the_scorers_read_blocks_the_production_parsers_read() {
    let cases: Cases = serde_json::from_str(include_str!("../../../testing/evals/orchestrator-wait/cases.json")).unwrap();
    assert_eq!(cases.fan_out.len(), 3);
    assert_eq!(cases.digest.envelope["type"], "bridge-worker-results");
    let delegate = |objective: &str| {
        format!("```bridge-delegate\n{}\n```", serde_json::json!({"schemaVersion":1,"role":"research","objective":objective,"acceptanceCriteria":["a summary"],"knownFacts":[],"decisions":[],"evidenceIds":[],"relevantFiles":[],"ownedPaths":[],"writeMode":"readOnly","capabilityTier":"fast","effort":"low","verification":[],"outputContract":"research-result"}))
    };
    let reply = format!("Starting three researchers.\n{}\n{}\n```bridge-wait\n{{\"workers\":\"all\"}}\n```\n", delegate("WAL"), delegate("VACUUM"));
    assert_eq!(delegations(&reply), 2);
    assert!(waits(&reply));
    assert_eq!(prose(&reply).trim(), "Starting three researchers.");
    assert!(claims_done("Great news: all three are done."));
    assert!(!claims_done("Two of three are in; RocksDB is still running."));
    assert!(!instructions(false).contains("bridge-wait"), "the control really lacks the guidance");
    assert!(instructions(true).contains("## Waiting for several workers (bridge-wait)"));
}
