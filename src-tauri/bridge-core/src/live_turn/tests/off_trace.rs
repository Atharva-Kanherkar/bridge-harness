//! The golden trace of the send, frame and manual compaction paths. Early
//! compaction must keep each part of it equal while its switch is off.

use super::*;
use crate::adapters::{AdapterRuntime, InterruptRequest, NativeCompaction, ShutdownReason, TurnContext};
use bridge_protocol::messages::TurnImage;
use regex::Regex;
use rusqlite::types::ValueRef;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) const CLAUDE_CHAT: &str = "claude-chat";
pub(super) const CODEX_CHAT: &str = "codex-chat";
pub(super) const CLAUDE_THREAD: &str = "claude-thread-1";
pub(super) const CODEX_THREAD: &str = "codex-thread-1";
pub(super) const CLAUDE_MODEL: &str = "claude-haiku-5-5";
pub(super) const CODEX_MODEL: &str = "gpt-6.2-sol";
const STARTED_AT: &str = "2026-11-20T10:00:00+00:00";
/// Set to record the fixture again from the run with the switch absent.
const RECORD: &str = "BRIDGE_RECORD_OFF_TRACE";
const SETTINGS_KIND: &str = "early_compaction_settings";
const OPT_OUT_KIND: &str = "early_compaction_opt_out";
const CODEX_TURN_START_ID: i64 = crate::codex_adapter::TURN_START_REQUEST_ID_BASE;
const UUID: &str = r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const TIME: &str = r"\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?";

pub(super) type CallLog = Arc<Mutex<Vec<Value>>>;

/// How the early compaction switch is stored for one run.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Switch {
    Absent,
    SavedOff,
}

impl Switch {
    fn label(self) -> &'static str {
        match self {
            Switch::Absent => "switch absent",
            Switch::SavedOff => "switch saved as false",
        }
    }
}

/// Run the scenario and compare its trace with the fixture, line for line.
pub(super) fn check(switch: Switch) {
    let trace = run(switch);
    let actual = format!("{}\n", serde_json::to_string_pretty(&trace).unwrap());
    let path = fixture_path();
    if switch == Switch::Absent && std::env::var_os(RECORD).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}. Record it with {RECORD}=1.", path.display()));
    if expected == actual {
        return;
    }
    let (expected_lines, actual_lines): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    let line = expected_lines.iter().zip(&actual_lines).position(|(left, right)| left != right)
        .unwrap_or(expected_lines.len().min(actual_lines.len()));
    panic!(
        "the off trace changed with the {}: first difference at line {}\nexpected: {}\nactual:   {}",
        switch.label(),
        line + 1,
        expected_lines.get(line).unwrap_or(&"<end of file>"),
        actual_lines.get(line).unwrap_or(&"<end of file>"),
    );
}

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testing/fixtures/early-compaction/off-trace.json")
}

fn run(switch: Switch) -> Value {
    let mut run = Run::new(switch);
    claude_scenario(&mut run);
    codex_scenario(&mut run);
    run.finish()
}

fn claude_scenario(run: &mut Run) {
    run.send(CLAUDE_CHAT, "Read notes.txt and summarise it in five lines.");
    run.feed(CLAUDE_CHAT, "a turn with one tool call", claude_first_turn());
    run.tick();
    run.send(CLAUDE_CHAT, "/compact");
    run.feed(CLAUDE_CHAT, "the manual compaction", claude_compaction("manual"));
    run.tick();
    run.send(CLAUDE_CHAT, "Reply with the single word ok.");
    run.feed(CLAUDE_CHAT, "a turn with its own compaction", claude_turn_with_auto_compaction());
    run.send(CLAUDE_CHAT, "/compact keep the parser notes");
    run.feed(CLAUDE_CHAT, "the focused compaction", claude_compaction("manual"));
    run.tick();
}

fn codex_scenario(run: &mut Run) {
    run.send(CODEX_CHAT, "Read notes.txt and summarise it in five lines.");
    let first = codex_turn_frames("codex-turn-1", "Read notes.txt and summarise it in five lines.", "The file has 400 numbered lines.");
    run.feed(CODEX_CHAT, "a turn with one reply", first);
    run.tick();
    run.send(CODEX_CHAT, "/compact");
    run.feed(CODEX_CHAT, "the compaction starts", codex_compaction_start(3, "codex-turn-2", "codex-compaction-1"));
    run.send(CODEX_CHAT, "And list the line numbers.");
    run.tick();
    run.feed(CODEX_CHAT, "the interrupted compaction ends", vec![codex_turn_completed("codex-turn-2", "interrupted")]);
    run.tick();
    let queued = codex_turn_frames("codex-turn-3", "And list the line numbers.", "The lines are numbered 001 to 400.");
    run.feed(CODEX_CHAT, "the queued message runs", queued);
    run.tick();
    run.send(CODEX_CHAT, "/compact keep the parser notes");
    let mut compaction = codex_compaction_start(4, "codex-turn-4", "codex-compaction-2");
    compaction.extend(codex_compaction_end("codex-turn-4", "codex-compaction-2"));
    run.feed(CODEX_CHAT, "the whole compaction", compaction);
    run.tick();
}

/// One scenario run: a core with two warm direct chats and a log of what
/// reached each adapter.
struct Run {
    scratch: tempfile::TempDir,
    core: Arc<BridgeCore>,
    events: crate::events::EventReceiver,
    calls: CallLog,
    turns: HashMap<&'static str, Arc<Mutex<Option<String>>>>,
    steps: Vec<Value>,
    attention_seen: i64,
}

impl Run {
    fn new(switch: Switch) -> Self {
        let scratch = tempfile::tempdir().unwrap();
        let mut core = BridgeCore::for_tests(scratch.path());
        let mut registry = AdapterRegistry::empty();
        registry.register(Box::new(TraceHarness::new("claude"))).unwrap();
        registry.register(Box::new(TraceHarness::new("codex"))).unwrap();
        core.adapter_registry = Arc::new(registry);
        let core = Arc::new(core);
        let calls = CallLog::default();
        let mut turns = HashMap::new();
        for (session, provider, claude) in [(CLAUDE_CHAT, CLAUDE_THREAD, true), (CODEX_CHAT, CODEX_THREAD, false)] {
            seed_chat(&core, session, provider, claude);
            let runtime = TraceRuntime { session, provider, claude, turn: Arc::default(), requests: AtomicUsize::new(1), calls: calls.clone() };
            turns.insert(session, runtime.current_turn());
            core.adapters.lock().unwrap().insert(session.to_owned(), Box::new(runtime));
        }
        if switch == Switch::SavedOff {
            save_switch_off(&core);
        }
        let attention_seen = crate::attention::revision(&core.db.lock().unwrap()).unwrap_or(0);
        let events = core.events.subscribe();
        Self { scratch, core, events, calls, turns, steps: Vec::new(), attention_seen }
    }

    fn send(&mut self, session: &str, text: &str) {
        let result = match submit_input(&self.core, session.to_owned(), text.to_owned()) {
            Ok(result) => serde_json::to_value(result).unwrap(),
            Err(error) => json!({"error": error.to_string()}),
        };
        self.record(json!({"step": "send", "session": session, "input": text, "result": result}));
    }

    fn feed(&mut self, session: &str, label: &str, frames: Vec<Value>) {
        let turn = self.turns[session].clone();
        for frame in &frames {
            handle_agent_value(&self.core, session, &turn, frame);
        }
        self.record(json!({"step": "frames", "session": session, "label": label, "frames": frames.len()}));
    }

    fn tick(&mut self) {
        queued_input_maintenance_tick(&self.core, &mut self.attention_seen);
        self.record(json!({"step": "tick"}));
    }

    /// Close one step with the adapter calls and the published events since
    /// the step before.
    fn record(&mut self, mut step: Value) {
        step["adapterCalls"] = Value::Array(std::mem::take(&mut *self.calls.lock().unwrap()));
        let mut events = Vec::new();
        loop {
            match self.events.try_next() {
                Ok(event) => events.push(json!({"kind": event.kind().as_str(), "payload": event.payload()})),
                Err(true) => events.push(json!("lagged")),
                Err(false) => break,
            }
        }
        step["events"] = Value::Array(events);
        self.steps.push(step);
    }

    fn finish(self) -> Value {
        let Run { scratch, core, steps, .. } = self;
        let rows = table_rows(&core.db.lock().unwrap());
        let transcript: serde_json::Map<String, Value> = [CLAUDE_CHAT, CODEX_CHAT]
            .into_iter()
            .map(|session| (session.to_owned(), serde_json::to_value(core.replay_session_events(session, 0, None, None).unwrap()).unwrap()))
            .collect();
        let mut trace = json!({"steps": steps, "rows": rows, "transcript": transcript});
        settle_clocks(&mut trace);
        Normalizer::new(scratch.path()).value(trace)
    }
}

/// A direct chat with a user title, so no title job reads a harness store
/// under the home directory.
fn seed_chat(core: &BridgeCore, session: &str, provider: &str, claude: bool) {
    let (harness, model) = if claude { ("claude", CLAUDE_MODEL) } else { ("codex", CODEX_MODEL) };
    core.db.lock().unwrap().execute(
        "INSERT INTO sessions(id,workspace_id,harness,model,label,title,status,metric_source,kind,depth,provider_session_id,started_at)
         VALUES(?1,NULL,?2,?3,'Chat','Notes chat','ready','reported','direct',0,?4,?5)",
        params![session, harness, model, provider, STARTED_AT],
    ).unwrap();
}

/// The real save path writes the row, so this run holds the same row as a
/// person who turned the switch off.
fn save_switch_off(core: &BridgeCore) {
    let params = bridge_protocol::messages::SaveEarlyCompactionSettingsParams { session_id: None, enabled: Some(false) };
    let view = crate::early_compaction_settings::save(core, &params).unwrap();
    assert!(!view.enabled && !core.early_compaction.enabled());
    let payload: String = core.db.lock().unwrap()
        .query_row("SELECT payload FROM configuration_entries WHERE kind=?1 AND id='global'", params![SETTINGS_KIND], |row| row.get(0))
        .unwrap();
    assert_eq!(payload, r#"{"enabled":false}"#);
}

/// A harness that starts nothing and normalizes frames as the real Claude and
/// Codex adapters do, with the same stream keys.
pub(super) struct TraceHarness {
    id: &'static str,
    claude: Mutex<HashMap<String, agent::ClaudeStreamState>>,
    codex: Mutex<HashMap<String, agent::CodexStreamState>>,
}

impl TraceHarness {
    pub(super) fn new(id: &'static str) -> Self {
        Self { id, claude: Mutex::default(), codex: Mutex::default() }
    }
}

impl HarnessAdapter for TraceHarness {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn descriptor(&self) -> AdapterDescriptor {
        let (label, model) = if self.id == "claude" { ("Claude Code", CLAUDE_MODEL) } else { ("Codex", CODEX_MODEL) };
        serde_json::from_value(json!({
            "id": self.id, "label": label, "available": true, "authState": "unknown",
            "capabilities": ["messages", "streaming", "reasoning", "tools", "commands", "file_changes", "approvals", "usage", "interrupt", "briefings", "steering"],
            "sandboxModes": ["read_only", "workspace_write", "danger_full_access"],
            "defaultModel": model,
            "models": [{"id": model, "label": model, "tier": "standard", "defaultForTier": true, "supportedEffortLevels": []}]
        }))
        .unwrap()
    }
    fn start(&self, _: StartRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid("the trace starts no process".into()))
    }
    fn resume(&self, _: ResumeRequest<'_>) -> Result<StartedAdapter, BridgeError> {
        Err(BridgeError::Invalid("the trace resumes no process".into()))
    }
    fn supports_native_resume(&self) -> bool {
        true
    }
    fn supports_native_compaction(&self) -> bool {
        true
    }
    fn normalize(&self, value: &Value) -> Vec<agent::NormalizedEvent> {
        if self.id == "claude" {
            let key = value.get("session_id").and_then(Value::as_str).unwrap_or("default").to_owned();
            let mut streams = self.claude.lock().unwrap();
            return agent::normalize_claude_message_with_state(value, streams.entry(key).or_default());
        }
        let key = value.get("_bridgeRootThreadId").or_else(|| value.pointer("/params/threadId"))
            .and_then(Value::as_str).unwrap_or("default").to_owned();
        let mut streams = self.codex.lock().unwrap();
        agent::normalize_codex_message_with_state(value, streams.entry(key).or_default())
    }
    fn forget_session(&self, provider_session_id: &str) {
        if provider_session_id != "default" {
            self.claude.lock().unwrap().remove(provider_session_id);
            self.codex.lock().unwrap().remove(provider_session_id);
        }
    }
}

/// A live process that logs each call in the shape that fixes the bytes the
/// real adapter writes. Turn names and refusals follow the real runtimes.
pub(super) struct TraceRuntime {
    pub(super) session: &'static str,
    pub(super) provider: &'static str,
    pub(super) claude: bool,
    pub(super) turn: Arc<Mutex<Option<String>>>,
    pub(super) requests: AtomicUsize,
    pub(super) calls: CallLog,
}

impl TraceRuntime {
    fn log(&self, call: &str, args: Value) {
        self.calls.lock().unwrap().push(json!({"session": self.session, "call": call, "args": args}));
    }

    /// The Claude runtime names the turn when it writes the message. Codex
    /// names it in `turn/started`, which the frame path reads.
    fn begin_turn(&self) {
        if self.claude {
            let number = self.requests.fetch_add(1, Ordering::SeqCst);
            *self.turn.lock().unwrap() = Some(format!("turn-{number}"));
        }
    }
}

fn context_entries(context: TurnContext<'_>) -> Value {
    context.entries().map(|entry| json!({"name": entry.name, "value": entry.value})).collect()
}

impl AdapterRuntime for TraceRuntime {
    fn process_id(&self) -> u32 {
        0
    }
    fn provider_session_id(&self) -> &str {
        self.provider
    }
    fn current_turn(&self) -> Arc<Mutex<Option<String>>> {
        self.turn.clone()
    }
    fn send_turn(&self, text: &str) -> Result<(), BridgeError> {
        self.log("send_turn", json!([text]));
        self.begin_turn();
        Ok(())
    }
    fn send_turn_with_context(&self, text: &str, context: TurnContext<'_>) -> Result<(), BridgeError> {
        self.log("send_turn_with_context", json!([text, context_entries(context)]));
        self.begin_turn();
        Ok(())
    }
    fn supports_images(&self) -> bool {
        true
    }
    fn send_turn_with_images(&self, text: &str, context: TurnContext<'_>, images: &[TurnImage]) -> Result<(), BridgeError> {
        self.log("send_turn_with_images", json!([text, context_entries(context), images.len()]));
        self.begin_turn();
        Ok(())
    }
    fn supports_active_turn_steering(&self) -> bool {
        true
    }
    fn steer_requires_acknowledged_turn(&self) -> bool {
        !self.claude
    }
    fn steer_turn(&self, text: &str, context: TurnContext<'_>, images: &[TurnImage]) -> Result<(), BridgeError> {
        if self.claude {
            return if images.is_empty() { self.send_turn_with_context(text, context) } else { self.send_turn_with_images(text, context, images) };
        }
        let turn = self.turn.lock().unwrap().clone()
            .ok_or_else(|| BridgeError::Invalid("The turn has finished; send the guidance again to start the next turn".into()))?;
        self.log("steer_turn", json!([turn, text, context_entries(context), images.len()]));
        Ok(())
    }
    fn interrupt(&self) -> Result<(), BridgeError> {
        self.log("interrupt", json!([]));
        Ok(())
    }
    /// The request is built at once and written later, as in the real runtimes.
    fn interrupt_request(&self) -> Option<InterruptRequest> {
        let target = if self.claude {
            format!("irq-{}", self.requests.fetch_add(1, Ordering::SeqCst))
        } else {
            self.turn.lock().unwrap().clone()?
        };
        let (calls, session) = (self.calls.clone(), self.session);
        Some(Box::new(move || {
            calls.lock().unwrap().push(json!({"session": session, "call": "interrupt_request", "args": [target]}));
            Ok(())
        }))
    }
    fn respond(&self, request_id: Value, decision: &str) -> Result<(), BridgeError> {
        self.log("respond", json!([request_id, decision]));
        Ok(())
    }
    fn answer_question(&self, request_id: Value, answers: Value) -> Result<(), BridgeError> {
        self.log("answer_question", json!([request_id, answers]));
        Ok(())
    }
    fn reject_question(&self, request_id: Value) -> Result<(), BridgeError> {
        self.log("reject_question", json!([request_id]));
        Ok(())
    }
    fn read_usage(&self) -> Result<(), BridgeError> {
        self.log("read_usage", json!([]));
        Ok(())
    }
    fn native_compaction(&self) -> NativeCompaction {
        if self.claude { NativeCompaction::WithFocus } else { NativeCompaction::WholeConversation }
    }
    fn compact_native(&self, focus: Option<&str>) -> Result<(), BridgeError> {
        self.log("compact_native", json!([focus]));
        self.begin_turn();
        Ok(())
    }
    fn stop(&mut self, reason: ShutdownReason) {
        self.log("stop", json!([format!("{reason:?}")]));
    }
}

pub(super) fn claude_frame(mut frame: Value) -> Value {
    frame["session_id"] = json!(CLAUDE_THREAD);
    frame
}

pub(super) fn claude_init() -> Value {
    json!({"type": "system", "subtype": "init", "model": CLAUDE_MODEL, "tools": ["Read"], "mcp_servers": [], "slash_commands": ["compact"], "uuid": "claude-init"})
}

pub(super) fn claude_status(status: Value, uuid: &str) -> Value {
    json!({"type": "system", "subtype": "status", "status": status, "uuid": uuid})
}

pub(super) fn claude_usage(input: u64, cache_write: u64, cache_read: u64, output: u64) -> Value {
    json!({"input_tokens": input, "cache_creation_input_tokens": cache_write, "cache_read_input_tokens": cache_read, "output_tokens": output})
}

pub(super) fn claude_assistant(id: &str, content: Value, usage: Value) -> Value {
    json!({"type": "assistant", "message": {"id": id, "role": "assistant", "model": CLAUDE_MODEL, "content": content, "usage": usage}, "parent_tool_use_id": null, "uuid": format!("{id}-uuid")})
}

/// A `result` frame. `usage` is the share of the turn and `modelUsage` holds
/// the session totals, as the SDK sends them.
pub(super) fn claude_result(text: &str, turns: u64, usage: Value, totals: (u64, u64, u64, u64, f64)) -> Value {
    let (input, output, cache_read, cache_write, cost) = totals;
    json!({
        "type": "result", "subtype": "success", "is_error": false, "duration_ms": 7071, "num_turns": turns, "result": text, "usage": usage,
        "modelUsage": {CLAUDE_MODEL: {"inputTokens": input, "outputTokens": output, "cacheReadInputTokens": cache_read, "cacheCreationInputTokens": cache_write, "costUSD": cost, "contextWindow": 1_000_000, "maxOutputTokens": 128_000}},
        "total_cost_usd": cost
    })
}

pub(super) fn claude_context_usage(used: u64) -> Value {
    json!({"type": "context_usage", "usedTokens": used, "windowTokens": 256_000, "categories": []})
}

fn claude_first_turn() -> Vec<Value> {
    let read = json!([{"type": "tool_use", "id": "toolu_01", "name": "Read", "input": {"file_path": "notes.txt"}}]);
    let answer = "The file has 400 numbered lines. Each line holds the same sentence.";
    vec![
        claude_init(),
        claude_status(json!("requesting"), "claude-request-1"),
        claude_assistant("msg_01", read, claude_usage(2, 12_604, 32_429, 486)),
        json!({"type": "user", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_01", "content": "001 the quick brown fox number 1 jumps over the lazy dog", "is_error": false}]}, "parent_tool_use_id": null, "uuid": "claude-tool-result-1"}),
        claude_status(json!("requesting"), "claude-request-2"),
        claude_assistant("msg_02", json!([{"type": "text", "text": answer}]), claude_usage(2, 21_039, 11_390, 173)),
        claude_result(answer, 2, claude_usage(4, 33_643, 43_819, 659), (1_193, 676, 43_819, 33_643, 0.0049)),
        claude_context_usage(45_121),
    ]
    .into_iter()
    .map(claude_frame)
    .collect()
}

/// A `/compact` as Claude Code answers it: status frames, the boundary, and a
/// `result` with a usage of zero.
pub(super) fn claude_compaction(trigger: &str) -> Vec<Value> {
    vec![
        claude_status(json!("compacting"), "claude-compacting"),
        json!({"type": "system", "subtype": "status", "status": null, "compact_result": "success", "uuid": "claude-compacted"}),
        claude_init(),
        json!({"type": "system", "subtype": "compact_boundary", "uuid": "claude-boundary", "compact_metadata": {"trigger": trigger, "pre_tokens": 45_586, "post_tokens": 13_654, "duration_ms": 5_705}}),
        claude_result("", 0, claude_usage(0, 0, 0, 0), (15_619, 1_765, 76_248, 33_818, 0.009)),
        claude_context_usage(35_752),
    ]
    .into_iter()
    .map(claude_frame)
    .collect()
}

pub(super) fn claude_turn_with_auto_compaction() -> Vec<Value> {
    vec![
        claude_init(),
        claude_status(json!("requesting"), "claude-request-3"),
        claude_status(json!("compacting"), "claude-auto-compacting"),
        json!({"type": "system", "subtype": "status", "status": null, "compact_result": "success", "uuid": "claude-auto-compacted"}),
        json!({"type": "system", "subtype": "compact_boundary", "uuid": "claude-auto-boundary", "compact_metadata": {"trigger": "auto", "pre_tokens": 35_900, "post_tokens": 12_100, "duration_ms": 4_200}}),
        claude_status(json!("requesting"), "claude-request-4"),
        claude_assistant("msg_03", json!([{"type": "text", "text": "ok"}]), claude_usage(2, 22_457, 11_390, 80)),
        claude_result("ok", 1, claude_usage(2, 22_457, 11_390, 80), (15_621, 1_845, 87_638, 56_275, 0.0137)),
        claude_context_usage(33_850),
    ]
    .into_iter()
    .map(claude_frame)
    .collect()
}

pub(super) fn codex(method: &str, params: Value) -> Value {
    json!({"method": method, "params": params})
}

pub(super) fn codex_turn(id: &str, status: &str) -> Value {
    json!({"id": id, "rootTurnId": id, "items": [], "itemsView": "notLoaded", "status": status, "error": null})
}

pub(super) fn codex_item(method: &str, turn: &str, item: Value) -> Value {
    codex(method, json!({"item": item, "threadId": CODEX_THREAD, "turnId": turn}))
}

pub(super) fn codex_tokens(total: u64, input: u64, cached: u64, output: u64) -> Value {
    json!({"totalTokens": total, "inputTokens": input, "cachedInputTokens": cached, "cacheWriteInputTokens": 0, "outputTokens": output, "reasoningOutputTokens": 0})
}

pub(super) fn codex_usage(turn: &str, total: Value, last: Value) -> Value {
    codex("thread/tokenUsage/updated", json!({"threadId": CODEX_THREAD, "turnId": turn, "tokenUsage": {"total": total, "last": last, "modelContextWindow": 475_000}}))
}

pub(super) fn codex_turn_completed(turn: &str, status: &str) -> Value {
    codex("turn/completed", json!({"threadId": CODEX_THREAD, "turn": codex_turn(turn, status)}))
}

fn codex_turn_frames(turn: &str, text: &str, reply: &str) -> Vec<Value> {
    let user = json!({"type": "userMessage", "id": format!("{turn}-user"), "content": [{"type": "text", "text": text, "text_elements": []}]});
    let message = |text: &str| json!({"type": "agentMessage", "id": format!("{turn}-reply"), "text": text, "phase": "final_answer"});
    vec![
        json!({"id": CODEX_TURN_START_ID, "result": {"turn": codex_turn(turn, "inProgress")}}),
        codex("thread/status/changed", json!({"threadId": CODEX_THREAD, "status": {"type": "active", "activeFlags": []}})),
        codex("turn/started", json!({"threadId": CODEX_THREAD, "turn": codex_turn(turn, "inProgress")})),
        codex_item("item/started", turn, user.clone()),
        codex_item("item/completed", turn, user),
        codex_item("item/started", turn, message("")),
        codex("item/agentMessage/delta", json!({"threadId": CODEX_THREAD, "turnId": turn, "itemId": format!("{turn}-reply"), "delta": reply})),
        codex_item("item/completed", turn, message(reply)),
        codex_usage(turn, codex_tokens(36_721, 36_600, 28_288, 121), codex_tokens(36_721, 36_600, 28_288, 121)),
        codex("thread/status/changed", json!({"threadId": CODEX_THREAD, "status": {"type": "idle"}})),
        codex_turn_completed(turn, "completed"),
    ]
}

/// The frames of `thread/compact/start` up to the open compaction item.
pub(super) fn codex_compaction_start(request: i64, turn: &str, item: &str) -> Vec<Value> {
    vec![
        json!({"id": request, "result": {}}),
        codex("turn/started", json!({"threadId": CODEX_THREAD, "turn": codex_turn(turn, "inProgress")})),
        codex_item("item/started", turn, json!({"type": "contextCompaction", "id": item})),
    ]
}

pub(super) fn codex_compaction_end(turn: &str, item: &str) -> Vec<Value> {
    vec![
        codex_usage(turn, codex_tokens(65_315, 65_195, 28_288, 120), codex_tokens(32_942, 0, 0, 0)),
        codex_item("item/completed", turn, json!({"type": "contextCompaction", "id": item})),
        codex_turn_completed(turn, "completed"),
    ]
}

/// Each base table with rows, in name order. Full text indexes and their
/// shadow tables are left out: they derive from the base rows.
fn table_rows(db: &Connection) -> serde_json::Map<String, Value> {
    let tables: Vec<(String, String)> = db
        .prepare("SELECT name, COALESCE(sql,'') FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let is_virtual = |sql: &str| sql.to_ascii_uppercase().starts_with("CREATE VIRTUAL TABLE");
    let shadow_prefixes: Vec<String> = tables.iter().filter(|(_, sql)| is_virtual(sql)).map(|(name, _)| format!("{name}_")).collect();
    let mut rows = serde_json::Map::new();
    for (name, sql) in &tables {
        if is_virtual(sql) || shadow_prefixes.iter().any(|prefix| name.starts_with(prefix.as_str())) {
            continue;
        }
        let table: Vec<Value> = read_table(db, name).into_iter().filter(|row| !is_switch_row(name, row)).collect();
        if !table.is_empty() {
            rows.insert(name.clone(), Value::Array(table));
        }
    }
    rows
}

/// The switch and the opt-out rows are reads the contract allows. A write of
/// any other row still shows.
fn is_switch_row(table: &str, row: &Value) -> bool {
    table == "configuration_entries" && matches!(row.get("kind").and_then(Value::as_str), Some(SETTINGS_KIND | OPT_OUT_KIND))
}

fn read_table(db: &Connection, name: &str) -> Vec<Value> {
    let mut statement = db
        .prepare(&format!("SELECT * FROM \"{name}\" ORDER BY rowid"))
        .or_else(|_| db.prepare(&format!("SELECT * FROM \"{name}\" ORDER BY 1")))
        .unwrap();
    let columns: Vec<String> = statement.column_names().into_iter().map(str::to_owned).collect();
    let mut rows = statement.query([]).unwrap();
    let mut table = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        let object: serde_json::Map<String, Value> = columns.iter().enumerate().map(|(index, column)| (column.clone(), cell(row.get_ref(index).unwrap()))).collect();
        table.push(Value::Object(object));
    }
    table
}

fn cell(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => json!(number),
        ValueRef::Real(number) => json!(number),
        ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            let structured = text.starts_with('{') || text.starts_with('[');
            structured.then(|| serde_json::from_str(&text).ok()).flatten().unwrap_or_else(|| Value::String(text.into_owned()))
        }
        ValueRef::Blob(bytes) => Value::String(bytes.iter().map(|byte| format!("{byte:02x}")).collect()),
    }
}

/// Zero the values that a host clock writes and that no text pattern finds:
/// the latency marks of a send, the wall time of a tool result, and the Unix
/// minute that Screen Time records for each active chat.
fn settle_clocks(trace: &mut Value) {
    if let Some(rows) = trace.pointer_mut("/rows/events").and_then(Value::as_array_mut) {
        for row in rows.iter_mut().filter(|row| row["kind"].as_str().is_some_and(|kind| kind.starts_with("latency."))) {
            zero_numbers(&mut row["body"]);
        }
    }
    if let Some(rows) = trace.pointer_mut("/rows/agent_activity_minutes").and_then(Value::as_array_mut) {
        for row in rows.iter_mut() {
            zero_numbers(&mut row["minute"]);
        }
        // A run that crosses a minute boundary writes a second minute for a
        // chat. The rows come in key order, so the copies are next to each other.
        rows.dedup();
    }
    zero_tool_result_durations(trace);
}

fn zero_numbers(value: &mut Value) {
    match value {
        Value::Number(_) => *value = json!(0),
        Value::Array(items) => items.iter_mut().for_each(zero_numbers),
        Value::Object(object) => object.values_mut().for_each(zero_numbers),
        _ => {}
    }
}

fn zero_tool_result_durations(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(zero_tool_result_durations),
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("tool_result") && object.contains_key("durationMs") {
                object.insert("durationMs".into(), json!(0));
            }
            object.values_mut().for_each(zero_tool_result_durations);
        }
        _ => {}
    }
}

/// Replace host paths, times, clock numbers and generated ids with stable
/// marks. Ids get a number in order of first appearance, so a changed id
/// order still shows.
struct Normalizer {
    paths: Vec<(String, &'static str)>,
    ids: HashMap<String, String>,
    uuid: Regex,
    time: Regex,
}

impl Normalizer {
    fn new(scratch: &Path) -> Self {
        let mut paths = Vec::new();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let home = std::env::var_os("HOME").map(PathBuf::from);
        for (path, mark) in [(Some(scratch.to_path_buf()), "<scratch>"), (Some(repo), "<repo>"), (home, "<home>")] {
            let Some(path) = path else { continue };
            if let Ok(real) = path.canonicalize() {
                paths.push((real.display().to_string(), mark));
            }
            paths.push((path.display().to_string(), mark));
        }
        paths.retain(|(path, _)| path.len() > 1);
        paths.sort_by_key(|(path, _)| std::cmp::Reverse(path.len()));
        Self { paths, ids: HashMap::new(), uuid: Regex::new(UUID).unwrap(), time: Regex::new(TIME).unwrap() }
    }

    fn value(&mut self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(&text)),
            Value::Number(number) => clock_mark(number),
            Value::Array(items) => Value::Array(items.into_iter().map(|item| self.value(item)).collect()),
            Value::Object(object) => Value::Object(object.into_iter().map(|(key, item)| (self.text(&key), self.value(item))).collect()),
            other => other,
        }
    }

    fn text(&mut self, text: &str) -> String {
        let mut text = text.to_owned();
        for (path, mark) in &self.paths {
            text = text.replace(path.as_str(), mark);
        }
        let text = self.time.replace_all(&text, "<time>");
        let ids = &mut self.ids;
        self.uuid
            .replace_all(&text, |found: &regex::Captures<'_>| {
                let next = ids.len() + 1;
                ids.entry(found[0].to_ascii_lowercase()).or_insert_with(|| format!("<id-{next}>")).clone()
            })
            .into_owned()
    }
}

/// A Unix time in seconds or in milliseconds is a clock reading.
fn clock_mark(number: serde_json::Number) -> Value {
    let clock = number.as_i64().is_some_and(|value| (1_600_000_000..2_100_000_000).contains(&value) || (1_600_000_000_000..2_100_000_000_000).contains(&value));
    if clock { json!("<clock>") } else { Value::Number(number) }
}
