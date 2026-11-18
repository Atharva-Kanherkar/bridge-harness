//! One approved personal-app write on a Claude connection.
//!
//! A one-shot launch of the connection's own account and folder whose whole
//! authority is one exact call ([`BriefingRuntimePolicy::compile_app_write`]).
//! Rust sends its own copy of the approved arguments; the sidecar refuses a
//! frame that is not deep-equal to its grant, and the SDK gate checks again
//! and consumes it. The launch stops as soon as the call answers.
//!
//! Codex needs none of this: its writes are a direct `mcpServer/tool/call`
//! on the connection's runtime, sent by the router after the approval.

use super::runtimes::{self, Key, Launched};
use crate::briefing_policy::BriefingRuntimePolicy;
use crate::adapters::ShutdownReason;
use crate::{runtime::BridgeCore, BridgeError};
use serde_json::Value;
use std::io::BufRead;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const WALL: Duration = Duration::from_secs(60);

fn failed() -> BridgeError {
    BridgeError::Invalid("The write did not complete; nothing was confirmed as sent".into())
}

pub fn claude(core: &Arc<BridgeCore>, key: &Key, tool: &str, arguments: &Value) -> Result<Value, BridgeError> {
    claude_with(key, tool, arguments, |key, policy| runtimes::launch_with(core, key, Some(policy)))
}

/// [`claude`] with the launch supplied, so a live test can use real adapters.
pub fn claude_with(
    key: &Key,
    tool: &str,
    arguments: &Value,
    launch: impl FnOnce(&Key, BriefingRuntimePolicy) -> Result<Launched, BridgeError>,
) -> Result<Value, BridgeError> {
    let policy = BriefingRuntimePolicy::compile_app_write(
        key.server.clone(),
        tool,
        arguments.clone(),
        crate::connector_runs::run_limits(crate::connector_runs::RunKind::Action),
    )
    .map_err(|error| BridgeError::Invalid(error.reason()))?;
    let grant = policy.exact_action_config().ok_or_else(failed)?;
    let Launched { mut runtime, reader } = launch(key, policy)?;
    let lines = forward(reader);
    let id = uuid::Uuid::new_v4().to_string();
    let sent = grant["tool"]
        .as_str()
        .ok_or_else(failed)
        .and_then(|wire_tool| runtime.call_tool(&id, wire_tool, &grant["arguments"]));
    let outcome = sent.and_then(|()| await_result(&lines, &id, Instant::now() + WALL));
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

/// The tool result for `id`, kept to the MCP result shape. A refusal or a
/// failure never relays the connector's own text: it can carry credentials.
fn await_result(lines: &mpsc::Receiver<String>, id: &str, deadline: Instant) -> Result<Value, BridgeError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BridgeError::Invalid("The write timed out; check the connector before trying again".into()));
        }
        let line = match lines.recv_timeout(remaining.min(Duration::from_secs(1))) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(failed()),
        };
        let Ok(frame) = serde_json::from_str::<Value>(&line) else { continue };
        if frame["type"] != "mcp_call_result" || frame["id"].as_str() != Some(id) {
            continue;
        }
        if frame["ok"] != true {
            return Err(failed());
        }
        let result = &frame["result"];
        if !result["content"].is_array() {
            return Err(failed());
        }
        let mut kept = serde_json::Map::new();
        for key in ["content", "structuredContent", "isError"] {
            if let Some(value) = result.get(key) {
                kept.insert(key.to_owned(), value.clone());
            }
        }
        let kept = Value::Object(kept);
        return if kept.to_string().len() > 256 * 1024 { Err(failed()) } else { Ok(kept) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture_server() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/fake-mcp-server.mjs")
    }

    fn logged(log: &std::path::Path) -> Vec<Value> {
        std::fs::read_to_string(log).unwrap_or_default().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    /// A real Claude sidecar, launched with the exact-write policy, sends the
    /// one approved call and the fake server receives exactly its arguments.
    #[test]
    #[ignore = "needs claude and the sidecar; run with --ignored"]
    fn live_claude_exact_write_sends_exactly_the_approved_arguments_once() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("calls.log");
        std::fs::write(
            home.path().join(".claude.json"),
            json!({"mcpServers": {"fake": {"type": "stdio", "command": "node", "args": [fixture_server().to_string_lossy()], "env": {"FAKE_MCP_LOG": log.to_string_lossy()}}}}).to_string(),
        )
        .unwrap();
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", home.path()) };
        let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let ready = Instant::now();
        while !registry.descriptors().iter().any(|d| d.id == "claude" && d.available) {
            assert!(ready.elapsed() < Duration::from_secs(60), "claude was not discovered");
            std::thread::sleep(Duration::from_millis(200));
        }
        let cwd = home.path().to_string_lossy().to_string();
        let key = Key { harness: "claude".into(), account: None, cwd: None, server: "fake".into() };
        let arguments = json!({"to": "Ada", "text": "hi \u{1F44B}", "meta": {"b": [2, 1], "a": null}});
        let result = claude_with(&key, "send_note", &arguments, |key, policy| {
            let started = registry.start(&key.harness, crate::adapters::StartRequest {
                session_id: None, cwd: &cwd, model: None, effort: None, speed: None, instructions: None, write_mode: None,
                read_only_sandbox: None, briefing: Some(&policy), interactive: false, account: None, on_progress: None,
            })?;
            Ok(Launched { runtime: started.runtime, reader: started.reader })
        })
        .unwrap();
        assert!(result.to_string().contains("Sent"), "{result}");
        let calls = logged(&log);
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(calls[0]["name"], "send_note");
        assert_eq!(crate::mcp_apps::canonical_json(&calls[0]["arguments"]), crate::mcp_apps::canonical_json(&arguments));
    }

    /// A real codex app-server: inventory and reads reach the server, but it
    /// logs no call until the approved write, which arrives byte-for-byte.
    #[test]
    #[ignore = "needs codex on PATH; run with --ignored"]
    fn live_codex_write_reaches_the_server_only_as_the_approved_call() {
        let home = tempfile::tempdir().unwrap();
        let log = home.path().join("calls.log");
        std::fs::write(
            home.path().join("config.toml"),
            format!("[mcp_servers.fake]\ncommand = \"node\"\nargs = [\"{}\"]\nenv = {{ FAKE_MCP_LOG = \"{}\" }}\n", fixture_server().display(), log.display()),
        )
        .unwrap();
        // SAFETY: an ignored test run on its own.
        unsafe { std::env::set_var("CODEX_HOME", home.path()) };
        let registry = Arc::new(crate::adapters::AdapterRegistry::built_in().unwrap());
        let ready = Instant::now();
        while !registry.descriptors().iter().any(|d| d.id == "codex" && d.available) {
            assert!(ready.elapsed() < Duration::from_secs(30), "codex was not discovered");
            std::thread::sleep(Duration::from_millis(200));
        }
        let cwd = home.path().to_string_lossy().to_string();
        let launcher = move |key: &Key| -> Result<Launched, BridgeError> {
            let started = registry.start(&key.harness, crate::adapters::StartRequest {
                session_id: None, cwd: &cwd, model: None, effort: None, speed: None, instructions: None,
                write_mode: Some(crate::delegation::WriteMode::Shared),
                read_only_sandbox: None, briefing: None, interactive: false, account: None, on_progress: None,
            })?;
            Ok(Launched { runtime: started.runtime, reader: started.reader })
        };
        let key = Key { harness: "codex".into(), account: None, cwd: None, server: "fake".into() };
        runtimes::request_with(&key, &json!({"kind":"inventory","server":"fake"}), &launcher).unwrap();
        assert!(logged(&log).is_empty(), "inventory calls no tool");
        let arguments = json!({"text": "hi", "to": "Ada"});
        let sent = runtimes::request_with(&key, &json!({"kind":"write","server":"fake","tool":"send_note","arguments":arguments}), &launcher).unwrap();
        assert!(sent.to_string().contains("Sent"), "{sent}");
        let calls = logged(&log);
        assert_eq!(calls.len(), 1, "{calls:?}");
        assert_eq!(crate::mcp_apps::canonical_json(&calls[0]["arguments"]), crate::mcp_apps::canonical_json(&arguments));
        runtimes::retire(&key);
    }

    #[test]
    fn only_this_calls_result_counts_and_failures_are_not_relayed() {
        let (sender, receiver) = mpsc::channel();
        for line in [
            json!({"type":"system","subtype":"init"}),
            json!({"type":"mcp_call_result","id":"other","ok":true,"result":{"content":[{"type":"text","text":"not mine"}]}}),
            json!({"type":"mcp_call_result","id":"mine","ok":true,"result":{"content":[{"type":"text","text":"sent"}],"_meta":{"x":1}}}),
        ] {
            sender.send(line.to_string()).unwrap();
        }
        let result = await_result(&receiver, "mine", Instant::now() + Duration::from_secs(5)).unwrap();
        assert_eq!(result, json!({"content":[{"type":"text","text":"sent"}]}));
        let (sender, receiver) = mpsc::channel();
        sender.send(json!({"type":"mcp_call_result","id":"mine","ok":false,"error":"token xoxb-secret refused"}).to_string()).unwrap();
        let error = await_result(&receiver, "mine", Instant::now() + Duration::from_secs(5)).unwrap_err().to_string();
        assert!(!error.contains("xoxb"), "{error}");
        drop(sender);
        let (sender, receiver) = mpsc::channel::<String>();
        drop(sender);
        assert!(await_result(&receiver, "mine", Instant::now() + Duration::from_secs(5)).is_err());
    }
}
