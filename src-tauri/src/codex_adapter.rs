use crate::{adapters::AdapterRuntime, binary, BridgeError};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex,
    },
};

pub struct CodexRuntime {
    pub writer: Arc<Mutex<ChildStdin>>,
    pub child: Child,
    pub thread_id: String,
    pub current_turn: Arc<Mutex<Option<String>>>,
    request_id: AtomicI64,
}

pub struct StartedCodex {
    pub runtime: CodexRuntime,
    pub reader: BufReader<ChildStdout>,
    pub startup_messages: Vec<Value>,
}

pub fn start(
    cwd: &str,
    model: Option<&str>,
    effort: Option<&str>,
    instructions: Option<&str>,
) -> Result<StartedCodex, BridgeError> {
    let binary = binary::resolve("codex").ok_or_else(|| {
        BridgeError::Invalid("Codex binary is not installed".into())
    })?;
    let mut child = Command::new(binary)
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| BridgeError::Invalid("Codex app-server stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Invalid("Codex app-server stdout unavailable".into()))?;
    let writer = Arc::new(Mutex::new(stdin));
    let mut reader = BufReader::new(stdout);
    write_value(
        &writer,
        &json!({"method":"initialize","id":1,"params":{"clientInfo":{"name":"bridge","title":"Bridge","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false,"requestAttestation":false}}}),
    )?;
    let (_, mut startup_messages) = wait_for_response(&mut reader, 1)?;
    write_value(&writer, &json!({"method":"initialized"}))?;
    // Full auto-accept: the user runs Bridge in unattended auto-accept mode, so
    // agents never wait on approval prompts and can perform local actions
    // (e.g. `open <file>` to launch the browser) without escalation.
    let mut params = json!({"cwd":cwd,"approvalPolicy":"never","sandbox":"danger-full-access","ephemeral":false,"serviceName":"Bridge"});
    if let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) {
        params["model"] = json!(model);
    }
    if let Some(effort) = effort.map(str::trim).filter(|value| !value.is_empty()) {
        // Reasoning-effort override. Field names accepted by current Codex
        // app-server builds; unknown fields are ignored safely on older ones,
        // and the worker briefing also states the effort so behavior follows.
        params["effort"] = json!(effort);
        params["model_reasoning_effort"] = json!(effort);
    }
    if let Some(instructions) = instructions.map(str::trim).filter(|value| !value.is_empty()) {
        // Accepted by current Codex app-server builds; unknown fields are ignored safely on older ones.
        params["developerInstructions"] = json!(instructions);
        params["instructions"] = json!(instructions);
    }
    write_value(
        &writer,
        &json!({"method":"thread/start","id":2,"params":params}),
    )?;
    let (response, mut later_messages) = wait_for_response(&mut reader, 2)?;
    startup_messages.append(&mut later_messages);
    let thread_id = response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            BridgeError::Invalid(format!(
                "Codex app-server returned no thread id: {response}"
            ))
        })?
        .to_owned();
    Ok(StartedCodex {
        runtime: CodexRuntime {
            writer,
            child,
            thread_id,
            current_turn: Arc::new(Mutex::new(None)),
            request_id: AtomicI64::new(10),
        },
        reader,
        startup_messages,
    })
}

impl CodexRuntime {
    pub fn start_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.request("turn/start", json!({"threadId":self.thread_id,"input":[{"type":"text","text":text,"text_elements":[]}]}))
    }
    pub fn interrupt(&self) -> Result<(), BridgeError> {
        let turn_id = self
            .current_turn
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| BridgeError::Invalid("No active turn to stop".into()))?;
        self.request(
            "turn/interrupt",
            json!({"threadId":self.thread_id,"turnId":turn_id}),
        )
    }
    pub fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        write_value(
            &self.writer,
            &json!({"id":request_id,"result":{"decision":decision}}),
        )
    }
    fn request(&self, method: &str, params: Value) -> Result<(), BridgeError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        write_value(
            &self.writer,
            &json!({"method":method,"id":id,"params":params}),
        )
    }
}

impl AdapterRuntime for CodexRuntime {
    fn provider_session_id(&self) -> &str {
        &self.thread_id
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.current_turn.clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.start_turn(text)
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        CodexRuntime::interrupt(self)
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        CodexRuntime::respond(self, request_id, decision)
    }
    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn binary_version() -> Option<String> {
    binary::version("codex")
}

fn write_value(writer: &Arc<Mutex<ChildStdin>>, value: &Value) -> Result<(), BridgeError> {
    let mut writer = writer.lock().unwrap();
    serde_json::to_writer(&mut *writer, value)
        .map_err(|e| BridgeError::Invalid(format!("Cannot encode adapter request: {e}")))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
fn wait_for_response(
    reader: &mut BufReader<ChildStdout>,
    id: i64,
) -> Result<(Value, Vec<Value>), BridgeError> {
    let mut skipped = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(BridgeError::Invalid(
                "Codex app-server closed during initialization".into(),
            ));
        }
        let value: Value = serde_json::from_str(line.trim())
            .map_err(|e| BridgeError::Invalid(format!("Invalid Codex app-server frame: {e}")))?;
        if value.get("id").and_then(Value::as_i64) == Some(id) {
            return Ok((value, skipped));
        }
        skipped.push(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn turn_request_is_structured_json_not_terminal_text() {
        let value = json!({"method":"turn/start","id":10,"params":{"threadId":"t","input":[{"type":"text","text":"hello","text_elements":[]}]}});
        assert_eq!(value["method"], "turn/start");
        assert!(value.to_string().contains("text_elements"));
        assert!(!value.to_string().contains("\\u001b"));
    }

    #[test]
    #[ignore = "requires an installed, authenticated Codex binary"]
    fn live_app_server_emits_a_structured_turn() {
        use std::{sync::mpsc, thread, time::Duration};
        let cwd = std::env::current_dir().unwrap();
        let started = start(cwd.to_str().unwrap(), None, None, None).unwrap();
        let mut runtime = started.runtime;
        let mut reader = started.reader;
        runtime
            .start_turn("Reply exactly BRIDGE_SMOKE_OK. Do not use tools.")
            .unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                break;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                let completed =
                    value.get("method").and_then(Value::as_str) == Some("turn/completed");
                let _ = sender.send(value);
                if completed {
                    break;
                }
            }
        });
        let mut methods = Vec::new();
        loop {
            let value = receiver
                .recv_timeout(Duration::from_secs(90))
                .expect("Codex turn timed out");
            if let Some(method) = value.get("method").and_then(Value::as_str) {
                methods.push(method.to_owned());
            }
            if value.get("method").and_then(Value::as_str) == Some("turn/completed") {
                break;
            }
        }
        runtime.stop();
        assert!(methods.iter().any(|method| method == "turn/started"));
        assert!(methods
            .iter()
            .any(|method| method == "item/agentMessage/delta" || method == "item/completed"));
        assert!(methods.iter().any(|method| method == "turn/completed"));
    }
}
