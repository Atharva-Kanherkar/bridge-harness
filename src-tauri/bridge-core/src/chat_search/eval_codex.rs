//! Evaluation-only Codex CLI transport for the shared search funnel.
//!
//! This is not production briefing support. The CLI runs in an empty scratch
//! directory with read-only sandboxing, no user config, and native integrations
//! disabled. Each turn replays this search's text history to `codex exec`, so
//! elapsed time includes a CLI launch per turn and provider tokens include its
//! remaining harness overhead. A native tool event rejects the run.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use chrono::Utc;
use rusqlite::params;
use serde_json::{json, Value};
use uuid::Uuid;

use super::agent::{ModelTurn, SearchModel, INSTRUCTIONS};
use super::{tools::redact, CHAT_SEARCH_SESSION_KIND};
use crate::runtime::BridgeCore;

pub const DEFAULT_MODEL: &str = "gpt-6-luna";

pub struct CodexEvalModel {
    core: Arc<BridgeCore>,
    session_id: String,
    binary: PathBuf,
    model: String,
    scratch: tempfile::TempDir,
    history: Vec<String>,
    failed: bool,
}

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        crate::adapters::terminate_process_group(self.0.id());
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl CodexEvalModel {
    pub fn start(core: &Arc<BridgeCore>, model: &str) -> Result<Self, String> {
        let binary = crate::codex_adapter::resolve_runtime().ok_or("Codex CLI is not installed")?;
        Self::start_with_binary(core, model, binary)
    }

    fn start_with_binary(core: &Arc<BridgeCore>, model: &str, binary: PathBuf) -> Result<Self, String> {
        let scratch = tempfile::tempdir().map_err(|error| error.to_string())?;
        std::fs::write(scratch.path().join("instructions.md"), INSTRUCTIONS).map_err(|error| error.to_string())?;
        let session_id = Uuid::new_v4().to_string();
        core.db
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,metric_source,model,kind,title,cwd,depth)
             VALUES(?1,NULL,'codex','Search eval','working','reported',?2,?3,'Chat search eval',?4,0)",
                params![
                    session_id,
                    model,
                    CHAT_SEARCH_SESSION_KIND,
                    scratch.path().to_string_lossy()
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(Self {
            core: Arc::clone(core),
            session_id,
            binary,
            model: model.into(),
            scratch,
            history: Vec::new(),
            failed: false,
        })
    }

    fn turn_inner(&mut self, text: &str, deadline: Instant) -> Result<ModelTurn, String> {
        self.history.push(format!("Bridge: {text}"));
        let prompt = redact(&self.history.join("\n\n"));
        let mut command = Command::new(&self.binary);
        crate::binary::hydrate_command_path(&mut command);
        command.args([
            "exec",
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "--skip-git-repo-check",
            "--sandbox",
            "read-only",
            "--json",
            "--color",
            "never",
            "--model",
            &self.model,
            "-c",
            "approval_policy=\"never\"",
            "-c",
            "model_reasoning_effort=\"low\"",
            "-c",
            "web_search=\"disabled\"",
        ]);
        command.arg("-c").arg(format!(
            "model_instructions_file={}",
            serde_json::to_string(&self.scratch.path().join("instructions.md").to_string_lossy()).unwrap()
        ));
        for feature in [
            "shell_tool",
            "multi_agent",
            "apps",
            "plugins",
            "browser_use",
            "computer_use",
            "in_app_browser",
            "code_mode_host",
            "code_mode",
            "sleep_tool",
            "skill_search",
            "tool_suggest",
        ] {
            command.args(["--disable", feature]);
        }
        command
            .arg("-")
            .current_dir(self.scratch.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::adapters::configure_process_group(&mut command);
        let mut running = Running(command.spawn().map_err(|error| error.to_string())?);
        let stderr = crate::adapters::StderrTail::capture(&mut running.0);
        let stdout = running.0.stdout.take().ok_or("Codex stdout missing")?;
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut input = running.0.stdin.take().ok_or("Codex stdin missing")?;
        input.write_all(prompt.as_bytes()).map_err(|error| error.to_string())?;
        drop(input);
        let mut reply = String::new();
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .ok_or("deadline exceeded")?;
            let line = match lines.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(redact(
                        &stderr
                            .snapshot()
                            .unwrap_or_else(|| "Codex ended before answering".into()),
                    ));
                }
            };
            let Ok(event) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(error) = event_error(&event) {
                return Err(error);
            }
            match event.get("type").and_then(Value::as_str) {
                Some("item.completed")
                    if event.pointer("/item/type").and_then(Value::as_str) == Some("agent_message") =>
                {
                    if let Some(text) = event.pointer("/item/text").and_then(Value::as_str) {
                        reply.push_str(text);
                    }
                }
                Some("turn.completed") => {
                    let tokens = total_tokens(&event);
                    let db = self.core.db.lock().unwrap();
                    crate::policy::record_provider_usage(
                        &db,
                        &self.session_id,
                        &self.session_id,
                        None,
                        "provider.codex",
                        &ledger_usage(&event),
                    )
                    .map_err(|error| error.to_string())?;
                    db.execute(
                        "UPDATE usage_ledger SET task_family='chat_search' WHERE session_id=?1",
                        params![self.session_id],
                    )
                    .map_err(|error| error.to_string())?;
                    self.history.push(format!("Model: {reply}"));
                    return Ok(ModelTurn { text: reply, tokens });
                }
                _ => {}
            }
        }
    }
}

impl SearchModel for CodexEvalModel {
    fn turn(&mut self, text: &str, deadline: Instant) -> Result<ModelTurn, String> {
        let result = self.turn_inner(text, deadline);
        self.failed |= result.is_err();
        result
    }
}

impl Drop for CodexEvalModel {
    fn drop(&mut self) {
        let _ = self.core.db.lock().unwrap().execute(
            "UPDATE sessions SET status=?2,ended_at=?3 WHERE id=?1",
            params![
                self.session_id,
                if self.failed { "failed" } else { "ended" },
                Utc::now().to_rfc3339()
            ],
        );
    }
}

fn total_tokens(event: &Value) -> u64 {
    // Codex input already includes cache reads; do not add cached_input_tokens again.
    ["input_tokens", "output_tokens"]
        .iter()
        .filter_map(|key| {
            event
                .get("usage")
                .and_then(|usage| usage.get(key))
                .and_then(Value::as_u64)
        })
        .sum()
}

fn ledger_usage(event: &Value) -> Value {
    let usage = &event["usage"];
    json!({"usage": {
        "input_tokens": usage["input_tokens"],
        "output_tokens": usage["output_tokens"],
        "cache_read_tokens": usage["cached_input_tokens"],
        "cache_write_tokens": usage["cache_write_input_tokens"],
        "reasoning_tokens": usage["reasoning_output_tokens"],
    }})
}

fn event_error(event: &Value) -> Option<String> {
    match event.get("type").and_then(Value::as_str) {
        Some("turn.failed" | "error") => Some(redact(
            &event
                .pointer("/error/message")
                .or_else(|| event.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("Codex provider error"),
        )),
        Some("item.started" | "item.completed") => {
            let kind = event.pointer("/item/type").and_then(Value::as_str)?;
            // A startup diagnostic can be informational; failed turns use turn.failed.
            (!matches!(kind, "agent_message" | "reasoning" | "error"))
                .then(|| format!("Codex attempted a native tool ({kind}); this evaluator accepts text only"))
        }
        _ => None,
    }
}

#[test]
fn codex_cache_reads_are_not_counted_twice() {
    let event = json!({"usage":{"input_tokens":100,"cached_input_tokens":80,"output_tokens":9}});
    assert_eq!(total_tokens(&event), 109);
    let records = crate::usage::normalize("provider.codex", &ledger_usage(&event));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].tokens.cache_read_tokens, Some(80));
    assert_eq!(records[0].tokens.uncached_input_tokens, Some(20));
}

#[test]
fn native_tools_and_provider_failures_are_rejected() {
    assert!(event_error(&json!({"type":"item.started","item":{"type":"command_execution"}})).is_some());
    assert_eq!(
        event_error(&json!({"type":"turn.failed","error":{"message":"quota reached"}})).as_deref(),
        Some("quota reached")
    );
    assert_eq!(
        event_error(&json!({"type":"item.completed","item":{"type":"agent_message","text":"{}"}})),
        None
    );
}

#[cfg(unix)]
#[test]
fn a_deadline_reaps_the_cli_and_settles_the_hidden_session() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-codex");
    let pid_file = root.path().join("pid");
    std::fs::write(
        &binary,
        format!("#!/bin/sh\nprintf '%s' $$ > '{}'\nexec sleep 30\n", pid_file.display()),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let core = Arc::new(BridgeCore::for_tests(root.path()));
    let mut model = CodexEvalModel::start_with_binary(&core, "fixture", binary).unwrap();
    let session_id = model.session_id.clone();
    let error = model
        .turn("Return an empty answer", Instant::now() + Duration::from_secs(2))
        .unwrap_err();
    assert!(error.contains("deadline"), "{error}");
    let pid: i32 = std::fs::read_to_string(pid_file).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "CLI must be reaped on timeout");
    drop(model);
    let db = core.db.lock().unwrap();
    let (status, ended): (String, bool) = db
        .query_row(
            "SELECT status, ended_at IS NOT NULL FROM sessions WHERE id=?1",
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "failed");
    assert!(ended);
}
