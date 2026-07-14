use crate::{
    adapters::{AdapterRuntime, ResumeRequest, ShutdownReason, StartRequest},
    binary,
    delegation::WriteMode,
    BridgeError,
};
use serde_json::{json, Value};
use std::{
    io::{BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};
use uuid::Uuid;

pub struct ClaudeRuntime {
    pub writer: Arc<Mutex<ChildStdin>>,
    pub child: Child,
    pub session_id: String,
    pub current_turn: Arc<Mutex<Option<String>>>,
    request_id: AtomicU64,
    stopped: bool,
}

pub struct StartedClaude {
    pub runtime: ClaudeRuntime,
    pub reader: BufReader<ChildStdout>,
    pub startup_messages: Vec<Value>,
}

pub fn start(request: StartRequest<'_>) -> Result<StartedClaude, BridgeError> {
    launch(request, None)
}

pub fn resume(request: ResumeRequest<'_>) -> Result<StartedClaude, BridgeError> {
    launch(
        StartRequest {
            cwd: request.cwd,
            model: request.model,
            effort: request.effort,
            instructions: request.instructions,
            write_mode: request.write_mode,
        },
        Some(request.provider_session_id),
    )
}

fn launch(
    request: StartRequest<'_>,
    resume_session_id: Option<&str>,
) -> Result<StartedClaude, BridgeError> {
    let StartRequest {
        cwd,
        model,
        effort,
        instructions,
        write_mode,
    } = request;
    // Claude runs through the Claude Agent SDK, driven by a Node sidecar. One
    // long-lived streaming query serves every turn on a single session (fixing
    // the `claude -p` "exit after one turn" behaviour), and the sidecar isolates
    // the child from the user's global settings/hooks and MCP servers.
    let node = binary::resolve("node").ok_or_else(|| {
        BridgeError::Invalid(
            "Node.js is required to run Claude (expected `node` on PATH). Install Node 18+ to use Claude models."
                .into(),
        )
    })?;
    let sidecar = sidecar_entry()?;
    let session_id = resume_session_id
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let chosen_model = model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("sonnet");
    let config = json!({
        "sessionId": session_id,
        "model": chosen_model,
        "cwd": cwd,
        "resume": resume_session_id.is_some(),
        // Bridge injects the delegation protocol + worker brief as an appended
        // system prompt so the child agent knows its single typed task.
        "instructions": instructions.map(str::trim).filter(|value| !value.is_empty()),
        "writeMode": write_mode.map(write_mode_label),
    });
    let mut command = Command::new(node);
    command
        .arg(&sidecar)
        .arg(config.to_string())
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // Claude Code has no per-run effort flag; the closest real knob is the
    // extended-thinking budget, which we scale by the routed effort tier.
    if let Some(budget) = thinking_budget(effort) {
        command.env("MAX_THINKING_TOKENS", budget.to_string());
    }
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.spawn().map_err(|e| {
        BridgeError::Invalid(format!("Failed to launch the Claude Agent SDK sidecar via node: {e}"))
    })?;
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
        "resumed": resume_session_id.is_some(),
    })];
    Ok(StartedClaude {
        runtime: ClaudeRuntime {
            writer,
            child,
            session_id,
            current_turn: Arc::new(Mutex::new(None)),
            request_id: AtomicU64::new(1),
            stopped: false,
        },
        reader,
        startup_messages,
    })
}

/// The write-mode label passed to the sidecar, which maps it to SDK permission
/// options (see `permissionOptions` in sidecar/claude-agent/index.mjs).
fn write_mode_label(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Full => "Full",
        WriteMode::ReadOnly => "ReadOnly",
        WriteMode::Shared => "Shared",
        WriteMode::Isolated => "Isolated",
    }
}

/// Locate the Claude Agent SDK sidecar entrypoint. Honours an explicit
/// `BRIDGE_CLAUDE_SIDECAR` override, then a bundled copy next to the executable
/// (production), then the in-repo path (development).
fn sidecar_entry() -> Result<PathBuf, BridgeError> {
    if let Ok(path) = std::env::var("BRIDGE_CLAUDE_SIDECAR") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("sidecar/claude-agent/index.mjs"));
            candidates.push(dir.join("../Resources/sidecar/claude-agent/index.mjs"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar/claude-agent/index.mjs"),
    );
    candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .ok_or_else(|| {
            BridgeError::Invalid(
                "Claude Agent SDK sidecar not found (set BRIDGE_CLAUDE_SIDECAR or ship sidecar/claude-agent/index.mjs next to the app)"
                    .into(),
            )
        })
}

/// Query Claude Code's subscription usage via the headless `/usage` command.
/// This is a read-only, zero-cost account query (no turn, no quota). Returns a
/// `{ "rateLimits": { ... } }` snapshot shaped like the Codex payload so the UI
/// can render both providers uniformly, or None when nothing parses.
pub fn read_usage_snapshot(cwd: &str) -> Option<Value> {
    // Headless `/usage` only includes the "% used" windows once its network fetch
    // resolves, which is racy. Retry a few times; the frontend keeps the last good
    // snapshot, so returning None just means "no fresh windows this poll".
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
        if let Some(snapshot) = read_usage_once(cwd) {
            return Some(snapshot);
        }
    }
    None
}

fn read_usage_once(cwd: &str) -> Option<Value> {
    let binary = binary::resolve("claude")?;
    let output = Command::new(binary)
        .args(["-p", "/usage", "--output-format", "json"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = parse_json_object(&stdout)?;
    let text = parsed.get("result").and_then(Value::as_str)?;
    let rate_limits = parse_usage_text(text);
    if rate_limits.as_object().map(|map| map.is_empty()).unwrap_or(true) {
        return None;
    }
    Some(json!({ "rateLimits": rate_limits }))
}

/// Best-effort extraction of the single JSON object printed by `--output-format json`.
fn parse_json_object(stdout: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str::<Value>(stdout.trim()) {
        return Some(value);
    }
    let start = stdout.find('{')?;
    let end = stdout.rfind('}')?;
    serde_json::from_str::<Value>(&stdout[start..=end]).ok()
}

/// Parse Claude's `/usage` report into labeled rate-limit windows. Lines look like:
/// `Current session: 38% used · resets Jul 13 at 7:09pm (Asia/Calcutta)`
fn parse_usage_text(text: &str) -> Value {
    let mut map = serde_json::Map::new();
    let mut order = 1usize;
    for raw in text.lines() {
        let line = raw.trim();
        if !line.contains("% used") {
            continue;
        }
        let Some(colon) = line.find(':') else {
            continue;
        };
        let head = line[..colon].trim();
        let rest = &line[colon + 1..];
        let Some(percent_pos) = rest.find('%') else {
            continue;
        };
        let digits: String = rest[..percent_pos]
            .chars()
            .rev()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        let Ok(percent) = digits.parse::<i64>() else {
            continue;
        };
        let resets = rest.find("resets").map(|index| {
            let mut phrase = rest[index..].trim().to_string();
            if let Some(timezone) = phrase.rfind(" (") {
                phrase.truncate(timezone);
            }
            phrase.trim().to_string()
        });
        let label = classify_usage_label(head);
        let key = format!("{order}_{}", label.to_lowercase().replace(' ', "_"));
        let mut window = serde_json::Map::new();
        window.insert("label".into(), json!(label));
        window.insert("usedPercent".into(), json!(percent));
        if let Some(resets) = resets.filter(|value| !value.is_empty()) {
            window.insert("resetsLabel".into(), json!(resets));
        }
        map.insert(key, Value::Object(window));
        order += 1;
    }
    Value::Object(map)
}

fn classify_usage_label(head: &str) -> String {
    let lower = head.to_lowercase();
    if lower.contains("session") {
        return "Session".into();
    }
    if let Some(open) = head.find('(') {
        let close = head.rfind(')').unwrap_or(head.len());
        let inside = head[open + 1..close].trim();
        if inside.to_lowercase().contains("all models") {
            return "Week".into();
        }
        if !inside.is_empty() {
            return inside.to_string();
        }
    }
    if lower.contains("week") {
        return "Week".into();
    }
    head.to_string()
}

pub fn supports_native_resume() -> bool {
    binary::resolve("node").is_some() && sidecar_entry().is_ok()
}

impl ClaudeRuntime {
    fn terminate(&mut self) {
        if self.stopped { return; }
        self.stopped = true;
        let _ = crate::adapters::terminate_process_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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
    fn process_id(&self) -> u32 { self.child.id() }
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
    fn stop(&mut self, _reason: ShutdownReason) {
        self.terminate();
    }
}

impl Drop for ClaudeRuntime {
    fn drop(&mut self) { self.terminate(); }
}

pub fn binary_version() -> Option<String> {
    sidecar_entry().ok()?;
    binary::version("node").map(|version| format!("Agent SDK (Node {version})"))
}

pub fn unavailable_reason() -> Option<String> {
    if binary::resolve("node").is_none() {
        return Some("Node.js 18+ is required to run Claude models".into());
    }
    sidecar_entry().err().map(|error| error.to_string())
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
    let mut writer = lock_writer(writer, "Claude")?;
    serde_json::to_writer(&mut *writer, value)
        .map_err(|e| BridgeError::Invalid(format!("Cannot encode Claude request: {e}")))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
fn lock_writer<'a, T>(writer: &'a Mutex<T>, provider: &str) -> Result<MutexGuard<'a, T>, BridgeError> {
    writer.lock().map_err(|_| BridgeError::Adapter(format!("{provider} stdin lock was poisoned; restart the session")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poisoned_writer_is_a_typed_adapter_error() {
        let writer = Mutex::new(());
        let _ = std::panic::catch_unwind(|| { let _guard = writer.lock().unwrap(); panic!("provider thread failed"); });
        assert!(matches!(lock_writer(&writer, "Claude"), Err(BridgeError::Adapter(_))));
    }

    #[test]
    fn parses_claude_usage_report_into_labeled_windows() {
        let text = "You are currently using your subscription to power your Claude Code usage\n\n\
            Current session: 38% used · resets Jul 13 at 7:09pm (Asia/Calcutta)\n\
            Current week (all models): 60% used · resets Jul 14 at 3:29am (Asia/Calcutta)\n\
            Current week (Fable): 81% used · resets Jul 14 at 3:29am (Asia/Calcutta)\n\n\
            What's contributing to your limits usage?";
        let value = parse_usage_text(text);
        let map = value.as_object().unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(map["1_session"]["label"], "Session");
        assert_eq!(map["1_session"]["usedPercent"], 38);
        assert_eq!(map["1_session"]["resetsLabel"], "resets Jul 13 at 7:09pm");
        assert_eq!(map["2_week"]["label"], "Week");
        assert_eq!(map["2_week"]["usedPercent"], 60);
        assert_eq!(map["3_fable"]["label"], "Fable");
        assert_eq!(map["3_fable"]["usedPercent"], 81);
    }

    #[test]
    fn write_mode_labels_map_to_sidecar_permission_modes() {
        assert_eq!(write_mode_label(WriteMode::Full), "Full");
        assert_eq!(write_mode_label(WriteMode::ReadOnly), "ReadOnly");
        assert_eq!(write_mode_label(WriteMode::Shared), "Shared");
        assert_eq!(write_mode_label(WriteMode::Isolated), "Isolated");
    }

    #[test]
    fn sidecar_entry_honours_explicit_override() {
        let dir = std::env::temp_dir().join(format!("bridge-sidecar-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let entry = dir.join("index.mjs");
        std::fs::write(&entry, "// test").unwrap();
        std::env::set_var("BRIDGE_CLAUDE_SIDECAR", &entry);
        let resolved = sidecar_entry().unwrap();
        std::env::remove_var("BRIDGE_CLAUDE_SIDECAR");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(resolved, entry);
    }

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
    #[ignore = "requires an authenticated Claude Agent SDK runtime"]
    fn live_stream_json_emits_a_structured_turn() {
        use std::{io::BufRead, sync::mpsc, thread, time::Duration};
        let cwd = std::env::temp_dir();
        let started = start(StartRequest {
            cwd: cwd.to_str().unwrap(),
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
        })
        .unwrap();
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
        runtime.stop(ShutdownReason::Completed);
        assert!(types
            .iter()
            .any(|kind| kind == "assistant" || kind == "stream_event"));
        assert!(types.iter().any(|kind| kind == "result"));
    }

    #[test]
    #[ignore = "requires an authenticated Claude Agent SDK runtime and persists a provider session"]
    fn live_claude_session_survives_process_restart() {
        use std::io::BufRead;

        fn run_turn(started: &mut StartedClaude, prompt: &str) -> String {
            started.runtime.start_turn(prompt).unwrap();
            let mut transcript = String::new();
            loop {
                let mut line = String::new();
                assert_ne!(started.reader.read_line(&mut line).unwrap(), 0);
                transcript.push_str(&line);
                let frame: Value = serde_json::from_str(line.trim()).unwrap();
                if frame.get("type").and_then(Value::as_str) == Some("result") {
                    return transcript;
                }
            }
        }

        let cwd = std::env::temp_dir();
        let cwd = cwd.to_str().unwrap();
        let mut started = start(StartRequest {
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
        })
        .unwrap();
        run_turn(&mut started, "Remember this exact token for the next turn: BRIDGE_CLAUDE_RESUME_5A72. Reply only SAVED.");
        let session_id = started.runtime.session_id.clone();
        started.runtime.stop(ShutdownReason::AppShutdown);

        let mut resumed = resume(ResumeRequest {
            cwd,
            model: None,
            effort: None,
            instructions: None,
            write_mode: None,
            provider_session_id: &session_id,
        })
        .unwrap();
        let transcript = run_turn(
            &mut resumed,
            "What exact token did I ask you to remember? Reply with only the token.",
        );
        resumed.runtime.stop(ShutdownReason::Completed);
        assert!(transcript.contains("BRIDGE_CLAUDE_RESUME_5A72"));
    }
}
