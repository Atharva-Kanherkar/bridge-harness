use crate::{adapters::AdapterRuntime, binary, BridgeError};
use serde_json::{json, Value};
use std::{
    io::{BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use uuid::Uuid;

pub struct ClaudeRuntime {
    pub writer: Arc<Mutex<ChildStdin>>,
    pub child: Child,
    pub session_id: String,
    pub current_turn: Arc<Mutex<Option<String>>>,
    request_id: AtomicU64,
}

pub struct StartedClaude {
    pub runtime: ClaudeRuntime,
    pub reader: BufReader<ChildStdout>,
    pub startup_messages: Vec<Value>,
}

pub fn start(
    cwd: &str,
    model: Option<&str>,
    effort: Option<&str>,
    instructions: Option<&str>,
) -> Result<StartedClaude, BridgeError> {
    let binary = binary::resolve("claude").ok_or_else(|| {
        BridgeError::Invalid(
            "Claude Code binary is not installed (expected `claude` on PATH or in ~/.local/bin)"
                .into(),
        )
    })?;
    let session_id = Uuid::new_v4().to_string();
    let chosen_model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("sonnet");
    let mut command = Command::new(binary);
    command.args([
        "-p",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-mode",
        "acceptEdits",
        "--model",
        chosen_model,
        "--session-id",
        &session_id,
    ]);
    // Bridge injects the delegation protocol + worker brief as an appended
    // system prompt so the child agent can itself delegate and knows its task.
    if let Some(instructions) = instructions.map(str::trim).filter(|value| !value.is_empty()) {
        command.args(["--append-system-prompt", instructions]);
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("CLAUDE_CODE_ENTRYPOINT", "bridge-deck");
    // Claude Code has no per-run effort flag; the closest real knob is the
    // extended-thinking budget, which we scale by the routed effort tier.
    if let Some(budget) = thinking_budget(effort) {
        command.env("MAX_THINKING_TOKENS", budget.to_string());
    }
    let mut child = command.spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| BridgeError::Invalid("Claude stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Invalid("Claude stdout unavailable".into()))?;
    let writer = Arc::new(Mutex::new(stdin));
    let reader = BufReader::new(stdout);
    let startup_messages = vec![json!({
        "type": "system",
        "subtype": "session_ready",
        "session_id": session_id,
        "cwd": cwd,
        "model": chosen_model,
    })];
    Ok(StartedClaude {
        runtime: ClaudeRuntime {
            writer,
            child,
            session_id,
            current_turn: Arc::new(Mutex::new(None)),
            request_id: AtomicU64::new(1),
        },
        reader,
        startup_messages,
    })
}

impl ClaudeRuntime {
    pub fn start_turn(&self, text: &str) -> Result<(), BridgeError> {
        let turn_id = format!("turn-{}", self.request_id.fetch_add(1, Ordering::Relaxed));
        *self.current_turn.lock().unwrap() = Some(turn_id.clone());
        write_value(
            &self.writer,
            &json!({
                "type": "user",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": text}]
                }
            }),
        )?;
        Ok(())
    }

    pub fn interrupt(&self) -> Result<(), BridgeError> {
        let request_id = format!("irq-{}", self.request_id.fetch_add(1, Ordering::Relaxed));
        write_value(
            &self.writer,
            &json!({
                "type": "control_request",
                "request_id": request_id,
                "request": {"subtype": "interrupt"}
            }),
        )
    }

    pub fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        let behavior = match decision {
            "accept" | "acceptForSession" => "allow",
            _ => "deny",
        };
        write_value(
            &self.writer,
            &json!({
                "type": "control_response",
                "response": {
                    "subtype": "success",
                    "request_id": request_id,
                    "response": {"behavior": behavior}
                }
            }),
        )
    }
}

impl AdapterRuntime for ClaudeRuntime {
    fn provider_session_id(&self) -> &str {
        &self.session_id
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.start_turn(text)
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        ClaudeRuntime::interrupt(self)
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        ClaudeRuntime::respond(self, request_id, decision)
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn binary_version() -> Option<String> {
    binary::version("claude")
}

/// Map a routed effort tier to an extended-thinking token budget. `None` leaves
/// Claude Code on its default (used for low/medium).
fn thinking_budget(effort: Option<&str>) -> Option<u32> {
    match effort.map(str::trim).unwrap_or("") {
        "high" => Some(16_000),
        "xhigh" => Some(32_000),
        _ => None,
    }
}

fn write_value(writer: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<(), BridgeError> {
    let mut writer = writer.lock().unwrap();
    serde_json::to_writer(&mut *writer, value)
        .map_err(|e| BridgeError::Invalid(format!("Cannot encode Claude request: {e}")))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_turn_is_structured_json_not_terminal_text() {
        let value = json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": "hello"}]}
        });
        assert_eq!(value["type"], "user");
        assert!(!value.to_string().contains("\\u001b"));
    }

    #[test]
    fn interrupt_control_request_is_structured() {
        let value = json!({
            "type": "control_request",
            "request_id": "irq-1",
            "request": {"subtype": "interrupt"}
        });
        assert_eq!(value["request"]["subtype"], "interrupt");
    }

    #[test]
    #[ignore = "requires an installed, authenticated Claude Code binary"]
    fn live_stream_json_emits_a_structured_turn() {
        use std::{io::BufRead, sync::mpsc, thread, time::Duration};
        let cwd = std::env::temp_dir();
        let started = start(cwd.to_str().unwrap(), None, None, None).unwrap();
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        runtime
            .start_turn("Reply exactly BRIDGE_CLAUDE_OK. Do not use tools.")
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                let done = value.get("type").and_then(Value::as_str) == Some("result");
                let _ = sender.send(value);
                if done {
                    break;
                }
            }
        });
        let mut types = Vec::new();
        loop {
            let value = receiver
                .recv_timeout(Duration::from_secs(90))
                .expect("Claude turn timed out");
            if let Some(kind) = value.get("type").and_then(Value::as_str) {
                types.push(kind.to_owned());
            }
            if value.get("type").and_then(Value::as_str) == Some("result") {
                break;
            }
        }
        runtime.stop();
        assert!(types.iter().any(|kind| kind == "assistant" || kind == "stream_event"));
        assert!(types.iter().any(|kind| kind == "result"));
    }
}
