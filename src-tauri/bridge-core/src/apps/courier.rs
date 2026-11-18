//! The gated courier: one personal-app call on an agent with no direct MCP
//! call (OpenCode).
//!
//! Bridge starts a hidden session whose rules deny everything, allow OpenCode
//! 2's code-mode `execute` tool (where its MCP tools live), and *ask* for the
//! one target tool. The model is prompted to make the call. OpenCode raises a
//! permission prompt carrying the exact arguments before anything runs, and
//! Bridge replies `once` only when they equal the approved arguments, byte for
//! byte in canonical JSON, and only for the first such prompt. Every other
//! prompt is rejected and the session ends. The model is a courier; Bridge
//! decides.
//!
//! The caller has already authorized the call: an allowlisted read with the
//! chat's access mode checked, or a write whose approval token was claimed.

use super::runtimes::{self, Key};
use crate::adapters::{ShutdownReason, StartRequest, StartedAdapter};
use crate::{runtime::BridgeCore, BridgeError};
use serde_json::{json, Value};
use std::io::BufRead;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(150);

fn invalid(text: impl Into<String>) -> BridgeError {
    BridgeError::Invalid(text.into())
}

/// OpenCode's permission name for an MCP tool: `<server>_<tool>`, each with
/// characters outside `[A-Za-z0-9_-]` replaced by `_`.
pub fn opencode_permission(server: &str, tool: &str) -> String {
    let clean = |text: &str| text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect::<String>();
    format!("{}_{}", clean(server), clean(tool))
}

fn launch(core: &Arc<BridgeCore>, key: &Key, permission: Option<&str>) -> Result<StartedAdapter, BridgeError> {
    let (cwd, account) = runtimes::environment(core, key)?;
    core.adapter_registry.start_app_courier(
        &key.harness,
        StartRequest {
            session_id: None,
            cwd: &cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: Some(crate::delegation::WriteMode::ReadOnly),
            read_only_sandbox: None,
            briefing: None,
            interactive: false,
            account: account.as_ref(),
            on_progress: None,
        },
        // Status only needs a running server: a permission nothing will match.
        permission.unwrap_or("bridge_status_only__"),
        &key.server,
    )
}

/// `{name, status, tools: null, verified: false}`: OpenCode reports whether
/// the server is connected and nothing about its tools.
pub fn opencode_inventory(core: &Arc<BridgeCore>, key: &Key) -> Result<Value, BridgeError> {
    let mut started = launch(core, key, None)?;
    let status = started.runtime.mcp_status(&key.server);
    started.runtime.stop(ShutdownReason::Completed);
    let status = status?;
    Ok(json!({"name": key.server, "status": status["status"], "tools": Value::Null, "verified": false}))
}

/// One call through an OpenCode courier session.
pub fn opencode_call(core: &Arc<BridgeCore>, key: &Key, tool: &str, arguments: &Value) -> Result<Value, BridgeError> {
    opencode_call_with(key, tool, arguments, |permission| launch(core, key, Some(permission)))
}

/// [`opencode_call`] with the launch supplied, so a live test can use real adapters.
pub fn opencode_call_with(
    key: &Key,
    tool: &str,
    arguments: &Value,
    launch: impl Fn(&str) -> Result<StartedAdapter, BridgeError>,
) -> Result<Value, BridgeError> {
    if !arguments.is_object() || arguments.to_string().len() > 64 * 1024 {
        return Err(invalid("Tool arguments must be an object within 64 KiB"));
    }
    let permission = opencode_permission(&key.server, tool);
    let canonical = crate::mcp_apps::canonical_json(arguments);
    // A model sometimes ends without asking at all. Nothing was approved or
    // sent then, so one fresh session may try again; any other failure ends
    // the call.
    match attempt(key, tool, &permission, &canonical, &launch) {
        Err(Outcome::NoCall) => attempt(key, tool, &permission, &canonical, &launch).map_err(Outcome::into_error),
        other => other.map_err(Outcome::into_error),
    }
}

/// Why a courier attempt ended without a result.
#[derive(Debug)]
pub(crate) enum Outcome {
    /// The agent never asked to make any call: nothing ran.
    NoCall,
    Failed(BridgeError),
}

impl Outcome {
    fn into_error(self) -> BridgeError {
        match self {
            Outcome::NoCall => failed("the agent ended without making the call"),
            Outcome::Failed(error) => error,
        }
    }
}

impl From<BridgeError> for Outcome {
    fn from(error: BridgeError) -> Self {
        Outcome::Failed(error)
    }
}

fn attempt(
    key: &Key,
    tool: &str,
    permission: &str,
    canonical: &str,
    launch: &dyn Fn(&str) -> Result<StartedAdapter, BridgeError>,
) -> Result<Value, Outcome> {
    let started = launch(permission)?;
    let mut runtime = started.runtime;
    let lines = forward(started.reader);
    let prompt = format!(
        "Make exactly one tool call and nothing else. Use the execute tool to call the MCP tool `{tool}` on the server `{server}` \
         with exactly these JSON arguments, unchanged: {canonical}\nDo not call any other tool, do not call it twice, and do not \
         change the arguments. When it returns, reply with one word: done.",
        server = key.server,
    );
    let outcome = match runtime.send_turn(&prompt) {
        Ok(()) => drive(&lines, permission, canonical, Instant::now() + DEADLINE, &mut |id, decision| runtime.respond(id, decision)),
        Err(error) => Err(Outcome::Failed(error)),
    };
    runtime.stop(ShutdownReason::Completed);
    outcome
}

fn forward(mut reader: Box<dyn BufRead + Send>) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender.send(line.trim_end().to_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

fn failed(reason: &str) -> BridgeError {
    invalid(format!("The call through OpenCode did not complete: {reason}. Nothing was confirmed as sent"))
}

/// The courier's decision loop over the session's frames.
///
/// `respond(requestId, "accept" | "decline")` answers a permission prompt.
/// Exactly one prompt can be accepted: the first whose permission is
/// `permission` and whose input is `canonical`. Any other prompt is declined
/// and ends the call. The result is the output of the tool call that carried
/// the accepted prompt.
pub(crate) fn drive(
    lines: &mpsc::Receiver<String>,
    permission: &str,
    canonical: &str,
    deadline: Instant,
    respond: &mut dyn FnMut(Value, &str) -> Result<(), BridgeError>,
) -> Result<Value, Outcome> {
    let mut approved_call: Option<Value> = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Outcome::Failed(failed("it timed out")));
        }
        let line = match lines.recv_timeout(remaining.min(Duration::from_secs(1))) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(Outcome::Failed(failed("OpenCode stopped"))),
        };
        let Ok(frame) = serde_json::from_str::<Value>(&line) else { continue };
        let properties = frame.get("properties").unwrap_or(&Value::Null);
        match frame["type"].as_str().unwrap_or_default() {
            "permission.asked" => {
                let id = properties["id"].clone();
                let asked = properties["permission"].as_str().or_else(|| properties["action"].as_str()).unwrap_or_default();
                let input = properties.pointer("/metadata/input").map(crate::mcp_apps::canonical_json);
                let exact = approved_call.is_none() && asked == permission && input.as_deref() == Some(canonical);
                if exact {
                    respond(id, "accept")?;
                    approved_call = Some(properties.pointer("/source/callID").cloned().unwrap_or(Value::Null));
                } else {
                    respond(id, "decline")?;
                    return Err(Outcome::Failed(failed(if approved_call.is_some() {
                        "the agent tried a second call, which was refused"
                    } else {
                        "the agent tried a different call than the one approved, which was refused"
                    })));
                }
            }
            "message.part.updated" => {
                let part = &properties["part"];
                let children = part.pointer("/state/metadata/toolCalls").and_then(Value::as_array);
                let settled = |status: &str| children.into_iter().flatten().any(|child| child["status"] == status);
                let Some(call) = &approved_call else {
                    // A child MCP call that finished before Bridge approved
                    // anything ran without asking (a saved approval, say):
                    // it may have written, so it is never retried.
                    if settled("completed") || settled("error") {
                        return Err(Outcome::Failed(failed("OpenCode ran a call without asking Bridge first")));
                    }
                    continue;
                };
                if part["type"] != "tool" || (!call.is_null() && part["callID"] != *call) {
                    continue;
                }
                match part.pointer("/state/status").and_then(Value::as_str) {
                    Some("completed") => {
                        // The outer `execute` completes whatever its script
                        // did; the approved MCP call's own status decides.
                        let errored = settled("error") || part.pointer("/state/metadata/error") == Some(&Value::Bool(true));
                        if errored || !settled("completed") {
                            return Err(Outcome::Failed(failed("the connector reported an error")));
                        }
                        let output = part.pointer("/state/output").and_then(Value::as_str).unwrap_or_default();
                        return Ok(json!({"content": [{"type": "text", "text": output}]}));
                    }
                    Some("error") => return Err(Outcome::Failed(failed("the connector reported an error"))),
                    _ => {}
                }
            }
            "session.error" => return Err(Outcome::Failed(failed("OpenCode reported an error"))),
            "session.idle" if approved_call.is_none() => return Err(Outcome::NoCall),
            "session.idle" => return Err(Outcome::Failed(failed("the call ended without a result"))),
            "session.status" if properties.pointer("/status/type").and_then(Value::as_str) == Some("idle") => {
                return Err(if approved_call.is_none() { Outcome::NoCall } else { Outcome::Failed(failed("the call ended without a result")) });
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERMISSION: &str = "fake_send_note";
    const CANONICAL: &str = r#"{"text":"hi","to":"Ada"}"#;

    fn asked(id: &str, permission: &str, input: Value, call: &str) -> String {
        json!({"type":"permission.asked","properties":{"id":id,"permission":permission,"metadata":{"input":input},"source":{"callID":call}}}).to_string()
    }
    fn tool(call: &str, status: &str, output: &str) -> String {
        let child = if status == "running" { "running" } else { status };
        tool_with(call, status, output, json!({"toolCalls": [{"tool": "fake.send_note", "status": child}]}))
    }
    fn tool_with(call: &str, status: &str, output: &str, metadata: Value) -> String {
        json!({"type":"message.part.updated","properties":{"part":{"type":"tool","callID":call,"tool":"execute","state":{"status":status,"output":output,"metadata":metadata}}}}).to_string()
    }
    fn run(frames: Vec<String>) -> (Result<Value, Outcome>, Vec<(Value, String)>) {
        let (sender, receiver) = mpsc::channel();
        for frame in frames {
            sender.send(frame).unwrap();
        }
        drop(sender);
        let mut replies = Vec::new();
        let result = drive(&receiver, PERMISSION, CANONICAL, Instant::now() + Duration::from_secs(5), &mut |id, decision| {
            replies.push((id, decision.to_owned()));
            Ok(())
        });
        (result, replies)
    }

    fn live_opencode(home: &std::path::Path, log: &std::path::Path) -> (Arc<crate::adapters::AdapterRegistry>, String) {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/fake-mcp-server.mjs");
        // `fake_send` + `note` flattens to `fake_send_note`, the target's name.
        let colliding = home.join("colliding.log");
        let config = json!({"mcp": {
            "fake": {"type": "local", "command": ["node", fixture.to_string_lossy()], "enabled": true, "environment": {"FAKE_MCP_LOG": log.to_string_lossy()}},
            "fake_send": {"type": "local", "command": ["node", fixture.to_string_lossy()], "enabled": true, "environment": {"FAKE_MCP_LOG": colliding.to_string_lossy(), "FAKE_MCP_TOOLS": "note"}}
        }});
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("OPENCODE_CONFIG_CONTENT", config.to_string()) };
        let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let ready = Instant::now();
        while !registry.descriptors().iter().any(|d| d.id == "opencode" && d.available) {
            assert!(ready.elapsed() < Duration::from_secs(60), "opencode was not discovered");
            std::thread::sleep(Duration::from_millis(200));
        }
        (registry, std::fs::canonicalize(home).unwrap().to_string_lossy().into_owned())
    }

    fn start(registry: &crate::adapters::AdapterRegistry, cwd: &str, permission: &str) -> Result<StartedAdapter, BridgeError> {
        let server = "fake";
        let model = std::env::var("BRIDGE_COURIER_MODEL").ok();
        registry.start_app_courier("opencode", StartRequest {
            session_id: None, cwd, model: model.as_deref(), effort: None, instructions: None,
            write_mode: Some(crate::delegation::WriteMode::ReadOnly), read_only_sandbox: None, briefing: None,
            interactive: false, account: None, on_progress: None,
        }, permission, server)
    }

    /// A real `opencode serve` with the fake MCP server: the courier's write
    /// lands byte-equal in the server's call log, exactly once, and a read
    /// returns the server's output. Uses `OPENCODE_CONFIG_CONTENT`, never the
    /// person's own OpenCode config, and makes real model turns.
    #[test]
    #[ignore = "needs opencode signed in with a model; run with --ignored"]
    fn live_opencode_courier_sends_exactly_the_approved_call() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("calls.log");
        let (registry, cwd) = live_opencode(home.path(), &log);
        let key = Key { harness: "opencode".into(), account: None, cwd: Some(cwd.clone()), server: "fake".into() };
        let mut status = start(&registry, &cwd, "bridge_status_only__").unwrap();
        let connected = (0..40).any(|_| {
            std::thread::sleep(Duration::from_millis(250));
            status.runtime.mcp_status("fake").is_ok_and(|s| s["status"] == "connected")
        });
        status.runtime.stop(ShutdownReason::Completed);
        assert!(connected, "OpenCode reports the fake server connected");
        let arguments = json!({"to": "Ada", "text": "hi from the courier"});
        let sent = opencode_call_with(&key, "send_note", &arguments, |permission| start(&registry, &cwd, permission)).unwrap();
        assert!(sent.to_string().contains("Sent"), "{sent}");
        let calls: Vec<Value> = std::fs::read_to_string(&log).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0]["name"], "send_note");
        assert_eq!(crate::mcp_apps::canonical_json(&calls[0]["arguments"]), crate::mcp_apps::canonical_json(&arguments));
        let read = opencode_call_with(&key, "search_people", &json!({"query": "Ada"}), |permission| start(&registry, &cwd, permission)).unwrap();
        assert!(read.to_string().contains("Found Ada Lovelace"), "{read}");
        assert!(!home.path().join("colliding.log").exists(), "the colliding server was set aside and never called");
    }

    #[test]
    fn names_follow_opencode() {
        assert_eq!(opencode_permission("fake", "send_note"), "fake_send_note");
        assert_eq!(opencode_permission("claude.ai Slack", "slack_send_message"), "claude_ai_Slack_slack_send_message");
    }

    #[test]
    fn an_exact_prompt_is_approved_once_and_its_output_returned() {
        // Key order differs; canonical JSON is the same bytes.
        let (result, replies) = run(vec![
            tool_with("other", "completed", "unrelated", json!({})),
            asked("per_1", PERMISSION, json!({"to":"Ada","text":"hi"}), "call_1"),
            tool("call_1", "running", ""),
            tool("call_1", "completed", "Sent {\"to\":\"Ada\",\"text\":\"hi\"}"),
        ]);
        assert_eq!(result.unwrap()["content"][0]["text"], "Sent {\"to\":\"Ada\",\"text\":\"hi\"}");
        assert_eq!(replies, vec![(json!("per_1"), "accept".to_owned())]);
    }

    #[test]
    fn anything_but_the_exact_first_call_is_refused() {
        for (frames, why) in [
            (vec![asked("per_1", PERMISSION, json!({"to":"Ada","text":"hi!"}), "c")], "one byte differs"),
            (vec![asked("per_1", "fake_delete_all", json!({"to":"Ada","text":"hi"}), "c")], "another tool"),
            (vec![asked("per_1", PERMISSION, json!({"to":"Ada","text":"hi"}), "c"), asked("per_2", PERMISSION, json!({"to":"Ada","text":"hi"}), "c")], "a second call"),
        ] {
            let (result, replies) = run(frames);
            assert!(result.is_err(), "{why}");
            assert_eq!(replies.last().unwrap().1, "decline", "{why}");
            assert!(replies.iter().filter(|(_, d)| d == "accept").count() <= 1, "{why}");
        }
    }

    #[test]
    fn the_agent_not_calling_or_the_call_failing_is_an_error() {
        let idle = json!({"type":"session.status","properties":{"status":{"type":"idle"}}}).to_string();
        assert!(matches!(run(vec![idle.clone()]).0, Err(Outcome::NoCall)), "nothing asked: may retry");
        assert!(matches!(run(vec![asked("per_1", PERMISSION, json!({"to":"Ada","text":"hi"}), "call_1"), idle]).0, Err(Outcome::Failed(_))), "an approved call that ends without a result is never retried");
        let (result, _) = run(vec![asked("per_1", PERMISSION, json!({"to":"Ada","text":"hi"}), "call_1"), tool("call_1", "error", "boom")]);
        let error = result.unwrap_err().into_error().to_string();
        assert!(!error.contains("boom"), "the connector's text is not relayed: {error}");
        assert!(run(vec![]).0.is_err(), "OpenCode stopping ends the call");
    }

    #[test]
    fn the_approved_calls_own_status_decides_not_the_script() {
        let approve = asked("per_1", PERMISSION, json!({"to":"Ada","text":"hi"}), "call_1");
        // The script caught the connector's error and returned success.
        let caught = tool_with("call_1", "completed", "SENT_SUCCESSFULLY", json!({"toolCalls": [{"tool": "fake.send_note", "status": "error"}]}));
        assert!(matches!(run(vec![approve.clone(), caught]).0, Err(Outcome::Failed(_))));
        // An uncaught error still completes the outer tool, flagged.
        let flagged = tool_with("call_1", "completed", "Error: denied", json!({"error": true, "toolCalls": [{"tool": "fake.send_note", "status": "completed"}]}));
        assert!(matches!(run(vec![approve.clone(), flagged]).0, Err(Outcome::Failed(_))));
        // No child call at all: nothing proves the approved call ran.
        let empty = tool_with("call_1", "completed", "done", json!({"toolCalls": []}));
        assert!(matches!(run(vec![approve, empty]).0, Err(Outcome::Failed(_))));
    }

    #[test]
    fn a_call_that_ran_without_asking_is_never_retried() {
        // A saved approval let the call run with no prompt at all.
        let ran = tool_with("call_1", "completed", "Sent", json!({"toolCalls": [{"tool": "fake.send_note", "status": "completed"}]}));
        let idle = json!({"type":"session.status","properties":{"status":{"type":"idle"}}}).to_string();
        let (result, replies) = run(vec![ran, idle]);
        assert!(matches!(result, Err(Outcome::Failed(_))), "it may have written: no second attempt");
        assert!(replies.is_empty());
    }
}
