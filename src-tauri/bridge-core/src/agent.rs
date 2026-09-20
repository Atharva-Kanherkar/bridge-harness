use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

/// The `requestMethod` marker for an OpenCode `question.asked` request.
pub const OPENCODE_QUESTION_REQUEST_METHOD: &str = "opencode.question";

fn permission_actions(allow_session: bool) -> Value {
    let mut actions = vec![
        json!({"id":"decline","decision":"decline","label":"Decline"}),
        json!({"id":"accept","decision":"accept","label":"Allow once"}),
    ];
    if allow_session {
        actions.insert(
            1,
            json!({"id":"acceptForSession","decision":"acceptForSession","label":"Allow for session"}),
        );
    }
    Value::Array(actions)
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedEvent {
    pub kind: String,
    pub item_id: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub data: Value,
}

#[derive(Debug, Default)]
pub struct OpenCodeStreamState {
    message_roles: HashMap<String, String>,
    /// Deltas name a field, not a part type: reasoning also streams as `text`.
    parts: HashMap<String, OpenCodePartState>,
    /// Busy/retry are status notifications and can repeat within one turn.
    turn_active: Option<bool>,
    /// `(modelID, providerID)` per assistant message, so a `step-finish` part
    /// can name the model that produced its tokens.
    message_models: HashMap<String, (Option<String>, Option<String>)>,
}

#[derive(Debug)]
struct OpenCodePartState {
    message_id: String,
    kind: String,
    completed: bool,
}

fn opencode_part_finished(part: &Value) -> bool {
    part.pointer("/time/end").is_some_and(|end| !end.is_null())
        || part.pointer("/state/status").and_then(Value::as_str) == Some("completed")
        || part.get("completed").and_then(Value::as_bool) == Some(true)
}

fn complete_opencode_turn(properties: &Value, state: &mut OpenCodeStreamState) -> Vec<NormalizedEvent> {
    if state.turn_active == Some(false) {
        return vec![];
    }
    state.turn_active = Some(false);
    let mut event = with_data("turn.completed", properties, properties.clone());
    event.status = Some("completed".into());
    vec![event]
}

pub fn normalize_opencode_message_with_state(
    message: &Value,
    state: &mut OpenCodeStreamState,
) -> Vec<NormalizedEvent> {
    let Some(event_type) = message.get("type").and_then(Value::as_str) else {
        return vec![];
    };
    let properties = message
        .get("properties")
        .cloned()
        .unwrap_or_else(|| json!({}));
    match event_type {
        "session.created" => vec![with_data(
            "session.started",
            &properties,
            properties.clone(),
        )],
        "session.status" => {
            let status = properties
                .pointer("/status/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            match status {
                "busy" | "retry" => {
                    if state.turn_active == Some(true) {
                        return vec![];
                    }
                    if state.turn_active == Some(false) {
                        state.message_roles.clear();
                        state.message_models.clear();
                        state.parts.clear();
                    }
                    state.turn_active = Some(true);
                    let mut event = with_data("turn.started", &properties, properties.clone());
                    event.status = Some(
                        if status == "retry" {
                            "retrying"
                        } else {
                            "working"
                        }
                        .into(),
                    );
                    vec![event]
                }
                "idle" => complete_opencode_turn(&properties, state),
                _ => vec![],
            }
        }
        "session.idle" => complete_opencode_turn(&properties, state),
        "message.updated" => {
            let info = properties.get("info").cloned().unwrap_or_else(|| json!({}));
            let message_id = info.get("id").and_then(Value::as_str).unwrap_or_default();
            let role = info.get("role").and_then(Value::as_str).unwrap_or_default();
            if !message_id.is_empty() && !role.is_empty() {
                state.message_roles.insert(message_id.into(), role.into());
            }
            if role == "assistant" && !message_id.is_empty() {
                let model = info.get("modelID").and_then(Value::as_str).map(str::to_owned);
                let provider = info.get("providerID").and_then(Value::as_str).map(str::to_owned);
                if model.is_some() || provider.is_some() {
                    state.message_models.insert(message_id.into(), (model, provider));
                }
            }
            // The message's own `tokens` are the last step's figures and its
            // `cost` the running sum over steps, so a row here would double
            // count the `step-finish` parts that carry each request exactly
            // once. Usage is read from those parts; this frame only names the
            // model they belong to.
            vec![]
        }
        "message.part.delta" => {
            let message_id = properties
                .get("messageID")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let field = properties
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or("text");
            let part = properties.get("partID").and_then(Value::as_str)
                .and_then(|id| state.parts.get(id))
                .filter(|part| part.message_id == message_id);
            if part.is_some_and(|part| part.completed) {
                return vec![];
            }
            let is_reasoning_field = match part {
                Some(part) => part.kind == "reasoning",
                None => field.contains("reasoning"),
            };
            // Tool input/output fields are not assistant prose.
            if part.is_some_and(|part| !matches!(part.kind.as_str(), "reasoning" | "text"))
                || (field != "text" && !field.contains("reasoning"))
            {
                return vec![];
            }
            let role = state.message_roles.get(message_id).map(String::as_str);
            if let Some(role) = role {
                if role != "assistant" {
                    return vec![];
                }
            } else if !is_reasoning_field {
                return vec![];
            }
            let mut event = with_data(
                if is_reasoning_field {
                    "reasoning.delta"
                } else {
                    "message.delta"
                },
                &properties,
                properties.clone(),
            );
            event.item_id = properties
                .get("partID")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.role = Some("assistant".into());
            event.status = Some("streaming".into());
            event.text = properties
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "message.part.updated" => {
            if let Some(part) = properties.get("part") {
                if let (Some(id), Some(message_id), Some(kind)) = (
                    part.get("id").and_then(Value::as_str),
                    part.get("messageID").and_then(Value::as_str),
                    part.get("type").and_then(Value::as_str),
                ) {
                    let held = state.parts.get(id);
                    // Snapshots can be repeated, but an older start must not
                    // reopen a thought whose completion already arrived.
                    if held.is_some_and(|held| held.completed) && !opencode_part_finished(part) {
                        return vec![];
                    }
                    state.parts.insert(id.into(), OpenCodePartState {
                        message_id: message_id.into(), kind: kind.into(),
                        completed: opencode_part_finished(part),
                    });
                }
            }
            normalize_opencode_part(&properties, state)
        }
        "session.diff" => vec![with_data("diff.updated", &properties, properties.clone())],
        // OpenCode summarized its own session. The event carries only the
        // session id, so the record names the harness and nothing else.
        "session.compacted" => vec![native_compaction("opencode", json!({}))],
        "todo.updated" => {
            let mut event = with_data("plan.updated", &properties, properties.clone());
            event.title = Some("OpenCode plan".into());
            vec![event]
        }
        "permission.v2.asked" | "permission.asked" => {
            let mut event = with_data("permission.requested", &properties, properties.clone());
            event.item_id = properties
                .pointer("/source/callID")
                .or_else(|| properties.pointer("/tool/callID"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let action = properties
                .get("action")
                .or_else(|| properties.get("permission"))
                .and_then(Value::as_str)
                .unwrap_or("tool action");
            event.title = Some(approval_title(ApprovalSubject::Tool(action)));
            event.text = properties
                .get("resources")
                .or_else(|| properties.get("patterns"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("\n")
                });
            event.status = Some("pending".into());
            event.data["requestId"] = properties.get("id").cloned().unwrap_or(Value::Null);
            event.data["interactionKind"] = Value::String("permission".into());
            event.data["actions"] = permission_actions(true);
            vec![event]
        }
        // OpenCode's `question` tool is a distinct channel from `permission`:
        // it is answered with a text/option payload over
        // `POST /question/{requestID}/reply`, never with a permission decision.
        "question.asked" => {
            let questions = properties
                .get("questions")
                .cloned()
                .unwrap_or_else(|| json!([]));
            let first_question = questions.get(0).cloned().unwrap_or_else(|| json!({}));
            let mut event = with_data("question.requested", &properties, properties.clone());
            event.item_id = properties
                .pointer("/tool/callID")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.title = first_question
                .get("header")
                .and_then(Value::as_str)
                .filter(|header| !header.is_empty())
                .map(str::to_owned)
                .or_else(|| Some("Question".into()));
            event.text = questions
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("question").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .filter(|text| !text.is_empty());
            event.status = Some("pending".into());
            event.data["requestId"] = properties.get("id").cloned().unwrap_or(Value::Null);
            event.data["requestMethod"] = Value::String(OPENCODE_QUESTION_REQUEST_METHOD.into());
            event.data["interactionKind"] = Value::String("question".into());
            event.data["questions"] = questions;
            vec![event]
        }
        // Whoever settled the question — Bridge's own reply, a decline, or a
        // completely different client on the same OpenCode session — this is
        // OpenCode's own record that the request is gone. `live_turn.rs`
        // resolves the matching `approval.requested` by `requestId` so a row
        // this process never itself answered still stops blocking the
        // session (#282: an unresolved row survives to target a stale id
        // forever otherwise).
        "question.replied" | "question.rejected" => {
            let mut event = with_data("question.settled", &properties, properties.clone());
            event.status = Some(
                if event_type == "question.replied" {
                    "answered"
                } else {
                    "rejected"
                }
                .into(),
            );
            event.data["requestId"] = properties.get("requestID").cloned().unwrap_or(Value::Null);
            vec![event]
        }
        "session.error" => {
            state.turn_active = Some(false);
            let mut event = with_data("error", &properties, properties.clone());
            event.status = Some("failed".into());
            event.title = Some("OpenCode error".into());
            event.text = properties
                .pointer("/error/data/message")
                .or_else(|| properties.pointer("/error/message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| properties.get("error").map(Value::to_string));
            let mut ended = NormalizedEvent::new("turn.completed");
            ended.status = Some("failed".into());
            vec![event, ended]
        }
        _ => {
            let mut event = with_data("provider.unknown", &properties, properties.clone());
            event.title = Some(event_type.into());
            vec![event]
        }
    }
}

fn normalize_opencode_part(
    properties: &Value,
    state: &mut OpenCodeStreamState,
) -> Vec<NormalizedEvent> {
    let part = properties.get("part").cloned().unwrap_or_else(|| json!({}));
    let part_type = part
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message_id = part
        .get("messageID")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_reasoning_part = part_type == "reasoning";
    let is_assistant = match state.message_roles.get(message_id).map(String::as_str) {
        Some(role) => role == "assistant",
        None => is_reasoning_part,
    };
    let item_id = part.get("id").and_then(Value::as_str).map(str::to_owned);
    match part_type {
        "text" if is_assistant && opencode_part_finished(&part) => {
            let mut event = with_data("message.completed", &part, part.clone());
            event.item_id = item_id;
            event.role = Some("assistant".into());
            event.status = Some("completed".into());
            event.text = part.get("text").and_then(Value::as_str).map(str::to_owned);
            vec![event]
        }
        "reasoning" if is_assistant => {
            let finished = opencode_part_finished(&part);
            let kind = if finished {
                "reasoning.completed"
            } else {
                // A part update is cumulative. The shared reducer replaces
                // started snapshots and appends only actual delta events.
                "reasoning.started"
            };
            let mut event = with_data(kind, &part, part.clone());
            event.item_id = item_id;
            event.status = Some(if finished { "completed" } else { "inProgress" }.into());
            event.text = part
                .get("reasoning")
                .or_else(|| part.get("text"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "tool" if is_assistant => {
            let status = part
                .pointer("/state/status")
                .and_then(Value::as_str)
                .unwrap_or("pending");
            let suffix = if matches!(status, "completed" | "error") {
                "completed"
            } else {
                "started"
            };
            let tool = part.get("tool").and_then(Value::as_str).unwrap_or("tool");
            let kind = if tool == "bash" {
                format!("command.{suffix}")
            } else if is_opencode_file_tool(tool) {
                format!("file_change.{suffix}")
            } else {
                format!("tool.{suffix}")
            };
            let mut event = with_data(&kind, &part, part.clone());
            event.item_id = item_id;
            event.title = part
                .pointer("/state/title")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some(tool.into()));
            event.status = Some(
                if status == "running" {
                    "inProgress"
                } else {
                    status
                }
                .into(),
            );
            event.text = part
                .pointer("/state/output")
                .or_else(|| part.pointer("/state/error"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if is_opencode_file_tool(tool) {
                stamp_file_change_fields(&mut event.data);
                if file_change_title_is_generic(event.title.as_deref()) {
                    if let Some(title) = file_change_title(&event.data) {
                        event.title = Some(title);
                    }
                }
            }
            vec![event]
        }
        "patch" => {
            let mut data = part.clone();
            stamp_file_change_fields(&mut data);
            vec![with_data("diff.updated", &part, data)]
        }
        "step-start" => {
            // A tool loop has several model steps within ONE submitted turn.
            // Retain this fallback for streams that omit the initial busy,
            // but never replace Bridge's turn id with a model-step part id.
            if state.turn_active == Some(true) {
                return vec![];
            }
            state.turn_active = Some(true);
            let mut event = with_data("turn.started", &part, part.clone());
            event.status = Some("working".into());
            vec![event]
        }
        "step-finish" => {
            let tokens = part.get("tokens").cloned().unwrap_or(Value::Null);
            let (model, provider) = state
                .message_models
                .get(message_id)
                .cloned()
                .unwrap_or((None, None));
            let mut event = with_data(
                "usage.updated",
                &part,
                json!({
                    "usage": {
                        "input_tokens": tokens.get("input").cloned().unwrap_or(Value::Null),
                        "output_tokens": tokens.get("output").cloned().unwrap_or(Value::Null),
                        "cache_read_tokens": tokens.pointer("/cache/read").cloned().unwrap_or(Value::Null),
                        "cache_write_tokens": tokens.pointer("/cache/write").cloned().unwrap_or(Value::Null),
                        "reasoning_tokens": tokens.get("reasoning").cloned().unwrap_or(Value::Null),
                    },
                    "cost": part.get("cost").cloned().unwrap_or(Value::Null),
                    "model": model,
                    "provider": provider,
                }),
            );
            event.item_id = item_id;
            vec![event]
        }
        _ => vec![],
    }
}

impl NormalizedEvent {
    /// An event of this kind with every optional field empty.
    ///
    /// Public so an integration outside this module can build one — the
    /// built-in normalizers live here, but #166's integrations do not.
    pub fn new(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            item_id: None,
            role: None,
            status: None,
            title: None,
            text: None,
            data: json!({}),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.kind.trim().is_empty() {
            return Err("normalized event kind cannot be empty".into());
        }
        if let Some(role) = &self.role {
            if !matches!(role.as_str(), "user" | "assistant" | "system" | "tool") {
                return Err(format!("unsupported normalized role: {role}"));
            }
        }
        if !self.data.is_object() && !self.data.is_array() {
            return Err("normalized event data must be structured JSON".into());
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct CodexStreamState {
    pub active_reasoning_id: Option<String>,
    pub reasoning_counter: usize,
    /// The turn whose compaction boundary has already been recorded.
    ///
    /// Codex describes one boundary two ways: a `contextCompaction` thread
    /// item, and the `thread/compacted` notification its own schema marks
    /// deprecated in favour of that item. A build that sends both would put
    /// two boundaries in history for one compaction, so the first arrival for
    /// a turn wins and the echo is dropped. The accepted cost is that a turn
    /// which compacts twice is recorded once; that is a smaller error than
    /// telling a reader the context shrank twice when it shrank once.
    pub compacted_turn: Option<String>,
    /// The model Codex switched to mid-turn (`model/rerouted`). Stamped on
    /// every usage frame until the turn ends, so the ledger attributes those
    /// requests to the model that served them rather than the one asked for.
    pub serving_model: Option<String>,
}

pub fn normalize_codex_message(message: &Value) -> Vec<NormalizedEvent> {
    normalize_codex_message_with_state(message, &mut CodexStreamState::default())
}

pub fn normalize_codex_message_with_state(
    message: &Value,
    state: &mut CodexStreamState,
) -> Vec<NormalizedEvent> {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return vec![];
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    match method {
        "thread/started" => vec![with_data("session.started", &params, params.clone())],
        "thread/status/changed" => {
            let status = params
                .pointer("/status/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let mut event = with_data("session.status", &params, params.clone());
            event.status = Some(status.into());
            vec![event]
        }
        "turn/started" => {
            state.active_reasoning_id = None;
            state.compacted_turn = None;
            state.serving_model = None;
            let mut event = with_data("turn.started", &params, params.clone());
            event.status = Some("working".into());
            vec![event]
        }
        "turn/completed" => {
            state.active_reasoning_id = None;
            state.serving_model = None;
            let status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let mut event = with_data("turn.completed", &params, params.clone());
            event.status = Some(status.into());
            if matches!(status, "failed" | "error") {
                let mut err_event = with_data("error", &params, params.clone());
                err_event.status = Some("failed".into());
                err_event.title = Some("Codex turn failed".into());
                err_event.text = params
                    .pointer("/turn/error/message")
                    .or_else(|| params.pointer("/error/message"))
                    .or_else(|| params.pointer("/turn/statusDetails"))
                    .or_else(|| params.pointer("/turn/reason"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| Some("Codex turn encountered a failure.".into()));
                vec![event, err_event]
            } else {
                vec![event]
            }
        }
        "item/agentMessage/delta" => {
            let mut event = with_data("message.delta", &params, json!({}));
            event.role = Some("assistant".into());
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            let mut event = with_data("reasoning.delta", &params, json!({}));
            let item_id = params
                .get("itemId")
                .or_else(|| params.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| state.active_reasoning_id.clone())
                .unwrap_or_else(|| {
                    state.reasoning_counter += 1;
                    let id = format!("reasoning-{}", state.reasoning_counter);
                    state.active_reasoning_id = Some(id.clone());
                    id
                });
            event.item_id = Some(item_id);
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "item/reasoning/summaryPartAdded" => {
            let mut event = with_data("reasoning.delta", &params, json!({}));
            let item_id = params
                .get("itemId")
                .or_else(|| params.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| state.active_reasoning_id.clone())
                .unwrap_or_else(|| {
                    state.reasoning_counter += 1;
                    let id = format!("reasoning-{}", state.reasoning_counter);
                    state.active_reasoning_id = Some(id.clone());
                    id
                });
            event.item_id = Some(item_id);
            event.text = params
                .get("summary")
                .or_else(|| params.get("text"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if event.text.is_some() {
                vec![event]
            } else {
                vec![]
            }
        }
        "item/commandExecution/outputDelta" => {
            // `delta` is already carried in `text`; retaining the complete
            // params duplicates the largest field in every transient frame.
            let mut event = with_data("command.output_delta", &params, json!({}));
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.status = Some("inProgress".into());
            vec![event]
        }
        "item/fileChange/outputDelta" => {
            let mut event = with_data("diff.delta", &params, json!({}));
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "item/mcpToolCall/progress" => {
            // App-server progress is a latest-state snapshot, not an output
            // delta. Keep only its display text and semantic item id; the
            // started/completed lifecycle items carry the durable tool shape.
            let mut event = with_data("tool.progress", &params, json!({}));
            event.text = params
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.status = Some("inProgress".into());
            vec![event]
        }
        "turn/plan/updated" => {
            let mut event = with_data("plan.updated", &params, params.clone());
            event.title = params
                .get("explanation")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "thread/tokenUsage/updated" => {
            let mut data = params.clone();
            // Always shadow the raw frame with an explicit `usage` key, even an
            // empty one. `UsageReport::from_normalized` falls back to the whole
            // event when `usage` is absent, and its recursive search would then
            // reach `tokenUsage.total` — the cumulative counter this normalizer
            // exists to keep out of the ledger. An empty object resolves to no
            // figures at all, so no row is written.
            data["usage"] = codex_request_usage(&params).unwrap_or_else(|| json!({}));
            if let Some(serving_model) = &state.serving_model {
                data["servingModel"] = Value::String(serving_model.clone());
            }
            vec![with_data("usage.updated", &params, data)]
        }
        // Codex compacted its own context. Its schema marks this notification
        // deprecated in favour of the `contextCompaction` thread item, so
        // whichever of the pair arrives first for a turn is the record and the
        // other is dropped. Neither carries token figures.
        "thread/compacted" => {
            if compaction_already_recorded(state, &params) {
                return vec![];
            }
            vec![native_compaction("codex", json!({}))]
        }
        "turn/diff/updated" | "item/fileChange/patchUpdated" => {
            let mut data = params.clone();
            stamp_file_change_fields(&mut data);
            vec![with_data("diff.updated", &params, data)]
        }
        "error" => {
            let mut event = with_data("error", &params, params.clone());
            event.text = params
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.status = Some(
                if params
                    .get("willRetry")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    "retrying"
                } else {
                    "failed"
                }
                .into(),
            );
            vec![event]
        }
        "model/rerouted" => {
            let from_model = params
                .get("fromModel")
                .and_then(Value::as_str)
                .unwrap_or("the requested model");
            let to_model = params
                .get("toModel")
                .and_then(Value::as_str)
                .unwrap_or("a fallback model");
            let reason = params.get("reason").and_then(Value::as_str);
            state.serving_model = params
                .get("toModel")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned);
            let mut event = with_data("model.rerouted", &params, params.clone());
            event.title = Some("Model rerouted".into());
            event.text = Some(match reason {
                Some(reason) => format!("Codex switched from {from_model} to {to_model}: {reason}"),
                None => format!("Codex switched from {from_model} to {to_model}."),
            });
            event.status = Some("completed".into());
            vec![event]
        }
        "thread/realtime/error" => {
            let mut event = with_data("error", &params, params.clone());
            event.title = Some("Realtime connection error".into());
            event.text = params
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some("Codex realtime encountered an error.".into()));
            event.status = Some("failed".into());
            vec![event]
        }
        "item/started" | "item/completed" => {
            let item_type = params
                .pointer("/item/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if item_type == "reasoning" {
                if method == "item/started" {
                    let id = params
                        .pointer("/item/id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            state.reasoning_counter += 1;
                            format!("reasoning-{}", state.reasoning_counter)
                        });
                    state.active_reasoning_id = Some(id);
                } else {
                    state.active_reasoning_id = None;
                }
            }
            normalize_item(method, &params, state)
        }
        _ if is_codex_internal_notification(method) => vec![],
        _ => {
            let mut event = with_data("provider.unknown", &params, params.clone());
            event.title = Some(method.into());
            vec![event]
        }
    }
}

/// The per-request slice of a Codex `thread/tokenUsage/updated` frame.
///
/// Codex reports two breakdowns: `total` is the thread's running counter and
/// `last` is the request that just completed. A ledger row is a per-request
/// delta, so only `last` belongs in the normalized `usage` object — and it has
/// to be named here, because `serde_json` runs with `preserve_order` and a
/// recursive alias search over the raw frame reaches `total` first.
///
/// The raw `tokenUsage` object stays on the event beside this, so the running
/// total and `modelContextWindow` remain readable. That is also why the caller
/// must still write an explicit `usage` key when this returns `None`: the raw
/// object it preserves is precisely what a recursive alias search would find.
fn codex_request_usage(params: &Value) -> Option<Value> {
    let last = params.pointer("/tokenUsage/last")?.as_object()?;
    let mut usage = serde_json::Map::new();
    for (wire, normalized) in [
        ("inputTokens", "input_tokens"),
        ("outputTokens", "output_tokens"),
        ("cachedInputTokens", "cache_read_tokens"),
        ("cacheWriteInputTokens", "cache_write_tokens"),
        ("reasoningOutputTokens", "reasoning_tokens"),
        ("totalTokens", "total_tokens"),
    ] {
        if let Some(count) = last.get(wire).and_then(Value::as_i64) {
            usage.insert(normalized.into(), count.into());
        }
    }
    (!usage.is_empty()).then(|| Value::Object(usage))
}

/// Documented app-server notifications that are transport/control-plane
/// bookkeeping rather than conversation. Letting these fall through to
/// `provider.unknown` made every progress tick durable, moved the forest
/// digest, and forced full-history reconciliation even though the UI hid the
/// row. This list is intentionally exact: a genuinely new method still falls
/// through as an inspectable unknown event.
fn is_codex_internal_notification(method: &str) -> bool {
    matches!(
        method,
        "thread/archived"
            | "thread/deleted"
            | "thread/unarchived"
            | "thread/closed"
            | "thread/reverted"
            | "skills/changed"
            | "thread/name/updated"
            | "thread/goal/updated"
            | "thread/goal/cleared"
            | "thread/queue/changed"
            | "project/changed"
            | "thread/project/updated"
            | "thread/environment/connected"
            | "thread/environment/disconnected"
            | "thread/settings/updated"
            | "hook/started"
            | "hook/completed"
            | "item/autoApprovalReview/started"
            | "item/autoApprovalReview/completed"
            | "autoApprovalReview/strictReviewRequired"
            | "item/plan/delta"
            | "command/exec/outputDelta"
            | "process/outputDelta"
            | "process/exited"
            | "item/commandExecution/terminalInteraction"
            | "serverRequest/resolved"
            | "mcpServer/oauthLogin/completed"
            | "mcpServer/startupStatus/updated"
            | "mcpServer/event/stream/notification"
            | "account/updated"
            | "app/list/updated"
            | "remoteControl/status/changed"
            | "externalAgentConfig/import/progress"
            | "externalAgentConfig/import/completed"
            | "fs/changed"
            | "model/verification"
            | "turn/moderationMetadata"
            | "model/safetyBuffering/updated"
            | "fuzzyFileSearch/sessionUpdated"
            | "fuzzyFileSearch/sessionCompleted"
            | "thread/realtime/started"
            | "thread/realtime/itemAdded"
            | "thread/realtime/item/started"
            | "thread/realtime/item/transcript/delta"
            | "thread/realtime/item/completed"
            | "thread/realtime/transcript/delta"
            | "thread/realtime/transcript/done"
            | "thread/realtime/outputAudio/delta"
            | "thread/realtime/sdp"
            | "thread/realtime/closed"
    )
}

pub fn normalize_codex_request(message: &Value) -> Option<NormalizedEvent> {
    let method = message.get("method")?.as_str()?;
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    if !method.ends_with("requestApproval")
        && method != "item/tool/requestUserInput"
        && method != "mcpServer/elicitation/request"
    {
        return None;
    }
    let is_question = matches!(
        method,
        "item/tool/requestUserInput" | "mcpServer/elicitation/request"
    );
    let mut event = with_data(
        if is_question {
            "question.requested"
        } else {
            "permission.requested"
        },
        &params,
        params.clone(),
    );
    event.item_id = params
        .get("itemId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.title = Some(match method {
        "item/fileChange/requestApproval" => approval_title(ApprovalSubject::FileChange),
        "item/tool/requestUserInput" => "Input required".into(),
        "mcpServer/elicitation/request" => "Tool input required".into(),
        _ => approval_title(ApprovalSubject::Command),
    });
    event.text = params
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.status = Some("pending".into());
    event.data["requestId"] = message.get("id").cloned().unwrap_or(Value::Null);
    event.data["requestMethod"] = Value::String(method.into());
    event.data["interactionKind"] = Value::String(
        if is_question { "question" } else { "permission" }.into(),
    );
    if !is_question {
        event.data["actions"] = permission_actions(true);
    }
    Some(event)
}

fn normalize_item(
    method: &str,
    params: &Value,
    state: &mut CodexStreamState,
) -> Vec<NormalizedEvent> {
    let item = params.get("item").cloned().unwrap_or_else(|| json!({}));
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if item_type == "contextCompaction" {
        // The boundary is a completed fact. Its opening half carries nothing a
        // reader can act on, and recording both halves would put two rows in
        // history for one compaction.
        if method == "item/started" || compaction_already_recorded(state, params) {
            return vec![];
        }
        let mut event = native_compaction("codex", json!({}));
        event.item_id = item.get("id").and_then(Value::as_str).map(str::to_owned);
        return vec![event];
    }
    let suffix = if method == "item/started" {
        "started"
    } else {
        "completed"
    };
    let kind = match item_type {
        "userMessage" | "agentMessage" => format!("message.{suffix}"),
        "reasoning" => format!("reasoning.{suffix}"),
        "plan" => format!("plan.{suffix}"),
        "fileChange" => format!("file_change.{suffix}"),
        "commandExecution" => format!("command.{suffix}"),
        "mcpToolCall" | "dynamicToolCall" | "collabAgentToolCall" | "webSearch" => {
            format!("tool.{suffix}")
        }
        "imageView" | "imageGeneration" => format!("artifact.{suffix}"),
        _ => format!("item.{suffix}"),
    };
    let mut event = with_data(&kind, params, item.clone());
    event.item_id = item.get("id").and_then(Value::as_str).map(str::to_owned);
    event.status = item
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| Some(suffix.into()));
    event.role = match item_type {
        "userMessage" => Some("user".into()),
        "agentMessage" => Some("assistant".into()),
        _ => None,
    };
    event.title = item
        .get("command")
        .or_else(|| item.get("tool"))
        .or_else(|| item.get("query"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.text = item
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let text = item
                .get("content")?
                .as_array()?
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("");
            (!text.is_empty()).then_some(text)
        });
    if item_type == "fileChange" {
        stamp_file_change_fields(&mut event.data);
        if event.title.is_none() {
            event.title = file_change_title(&event.data);
        }
    }
    vec![event]
}

#[allow(dead_code)]
pub fn normalize_claude_message(message: &Value) -> Vec<NormalizedEvent> {
    normalize_claude_message_with_state(message, &mut ClaudeStreamState::default())
}

#[derive(Debug, Default, Clone)]
pub struct ClaudeStreamState {
    pub active_message_id: Option<String>,
    pub active_reasoning_id: Option<String>,
    thinking_blocks: std::collections::BTreeMap<String, ClaudeThinkingBlock>,
    /// Open tool calls by `tool_use` id, so the eventual `tool_result` completes
    /// under the same normalized kind and carries a host-measured duration.
    tool_calls: HashMap<String, ClaudeToolCall>,
    /// Survives the per-result reset below: it is the one thing a turn needs
    /// from the turns before it.
    usage_cumulative: ClaudeUsageCumulative,
}

/// The running totals the previous `result` frame carried. The SDK documents
/// `total_cost_usd` and `modelUsage` as cumulative across the turns of one
/// query — "read the latest result rather than summing" — while the ledger
/// wants one turn's worth per row, so each result is reported as its
/// difference from the frame before it. A figure that goes backwards means the
/// process started fresh (a resume, a `/clear`, a crash restart) and the
/// frame is taken as-is. `usage` is per-turn already and passes through.
#[derive(Debug, Default, Clone)]
struct ClaudeUsageCumulative {
    total_cost_usd: Option<f64>,
    model_usage: HashMap<String, Value>,
}

const CLAUDE_MODEL_USAGE_COUNTERS: [&str; 5] = [
    "inputTokens",
    "outputTokens",
    "cacheReadInputTokens",
    "cacheCreationInputTokens",
    "webSearchRequests",
];
const CLAUDE_MODEL_USAGE_COST: &str = "costUSD";

fn cumulative_delta(current: f64, previous: Option<f64>) -> f64 {
    match previous {
        Some(previous) if current >= previous => current - previous,
        _ => current,
    }
}

fn claude_turn_cost(message: &Value, cumulative: &mut ClaudeUsageCumulative) -> Value {
    let Some(current) = message.get("total_cost_usd").and_then(Value::as_f64) else {
        return Value::Null;
    };
    let turn = cumulative_delta(current, cumulative.total_cost_usd);
    cumulative.total_cost_usd = Some(current);
    json!(turn)
}

fn claude_turn_model_usage(message: &Value, cumulative: &mut ClaudeUsageCumulative) -> Value {
    let Some(entries) = message.get("modelUsage").and_then(Value::as_object) else {
        return Value::Null;
    };
    let mut turn_entries = serde_json::Map::new();
    for (model, entry) in entries {
        let Some(current) = entry.as_object() else {
            turn_entries.insert(model.clone(), entry.clone());
            continue;
        };
        let previous = cumulative
            .model_usage
            .get(model)
            .and_then(Value::as_object);
        // One counter going backwards means this model's run started over;
        // its fields reset together rather than one at a time.
        let reset = previous.is_none_or(|previous| {
            CLAUDE_MODEL_USAGE_COUNTERS
                .iter()
                .chain([CLAUDE_MODEL_USAGE_COST].iter())
                .any(|key| {
                    matches!(
                        (current.get(*key).and_then(Value::as_f64), previous.get(*key).and_then(Value::as_f64)),
                        (Some(now), Some(before)) if now < before
                    )
                })
        });
        let mut turn = current.clone();
        for key in CLAUDE_MODEL_USAGE_COUNTERS {
            if let Some(now) = current.get(key).and_then(Value::as_f64) {
                let before = (!reset)
                    .then(|| previous.and_then(|previous| previous.get(key)).and_then(Value::as_f64))
                    .flatten();
                turn.insert(key.to_owned(), json!(cumulative_delta(now, before).round() as i64));
            }
        }
        if let Some(now) = current.get(CLAUDE_MODEL_USAGE_COST).and_then(Value::as_f64) {
            let before = (!reset)
                .then(|| previous.and_then(|previous| previous.get(CLAUDE_MODEL_USAGE_COST)).and_then(Value::as_f64))
                .flatten();
            turn.insert(CLAUDE_MODEL_USAGE_COST.to_owned(), json!(cumulative_delta(now, before)));
        }
        turn_entries.insert(model.clone(), Value::Object(turn));
    }
    cumulative.model_usage = entries
        .iter()
        .map(|(model, entry)| (model.clone(), entry.clone()))
        .collect();
    Value::Object(turn_entries)
}

#[derive(Debug, Default, Clone)]
struct ClaudeThinkingBlock {
    text: String,
    completed: bool,
}

fn claude_thinking_id(message_id: &str, index: u64) -> String {
    // Preserve the identity used by existing histories for the first block.
    if index == 0 {
        format!("reasoning-{message_id}")
    } else {
        format!("reasoning-{message_id}-block-{index}")
    }
}

fn complete_claude_thinking(id: &str, block: &mut ClaudeThinkingBlock) -> Option<NormalizedEvent> {
    if block.completed {
        return None;
    }
    block.completed = true;
    if block.text.is_empty() {
        return None;
    }
    let mut event = NormalizedEvent::new("reasoning.completed");
    event.item_id = Some(id.to_owned());
    event.status = Some("completed".into());
    event.text = Some(block.text.clone());
    Some(event)
}

#[derive(Debug, Clone, Copy)]
struct ClaudeToolCall {
    family: ClaudeToolFamily,
    started_at: Instant,
}

/// The normalized item family a Claude tool name belongs to. Mirrors the
/// Codex item-type split and the OpenCode tool-name split so all three
/// providers render through the same conversation cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaudeToolFamily {
    Command,
    FileChange,
    Tool,
}

impl ClaudeToolFamily {
    fn kind(self, suffix: &str) -> String {
        match self {
            Self::Command => format!("command.{suffix}"),
            Self::FileChange => format!("file_change.{suffix}"),
            Self::Tool => format!("tool.{suffix}"),
        }
    }
}

fn claude_tool_family(name: &str) -> ClaudeToolFamily {
    match name {
        "Bash" => ClaudeToolFamily::Command,
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => ClaudeToolFamily::FileChange,
        _ => ClaudeToolFamily::Tool,
    }
}

/// One started event for a Claude `tool_use` block, from either the streaming
/// `content_block_start` (input may still be empty) or the assistant snapshot
/// (full input). Both go out under the same item id, so the snapshot refines
/// the streamed card instead of duplicating it.
/// Where a Claude tool call is in its life when Bridge learns about it.
///
/// Claude Code streams a `tool_use` block's *start* as soon as the model
/// begins writing the call, then streams the arguments token by token; the
/// tool itself only runs once the whole assistant message has landed. A
/// long command or PR body can take minutes to write, and during that time
/// nothing is executing — Claude Code's own UI shows the model composing,
/// not a running tool.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClaudeToolPhase {
    /// `content_block_start`: the model is still writing the arguments.
    Preparing,
    /// The assistant snapshot: arguments complete, execution begins now.
    Running,
}

impl ClaudeToolPhase {
    fn as_str(self) -> &'static str {
        match self {
            ClaudeToolPhase::Preparing => "preparing",
            ClaudeToolPhase::Running => "running",
        }
    }
}

fn claude_tool_started(
    message: &Value,
    block: &Value,
    state: &mut ClaudeStreamState,
    phase: ClaudeToolPhase,
) -> NormalizedEvent {
    let tool_id = block
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_owned();
    let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
    let family = claude_tool_family(name);
    // The clock the card's duration reads starts when the tool actually runs.
    // The streamed start only registers the call; the snapshot (re)starts the
    // clock so minutes of argument streaming never show up as run time.
    let now = Instant::now();
    state
        .tool_calls
        .entry(tool_id.clone())
        .and_modify(|call| {
            if phase == ClaudeToolPhase::Running {
                call.started_at = now;
            }
        })
        .or_insert(ClaudeToolCall {
            family,
            started_at: now,
        });
    let mut event = with_data(&family.kind("started"), message, block.clone());
    event.item_id = Some(tool_id);
    event.title = Some(name.to_owned());
    event.status = Some("inProgress".into());
    event.data["phase"] = Value::String(phase.as_str().to_owned());
    let input = block.get("input").cloned().unwrap_or(Value::Null);
    match family {
        ClaudeToolFamily::Command => {
            if let Some(command) = input.get("command").and_then(Value::as_str) {
                // The command string is what the conversation card shows,
                // matching the Codex commandExecution title.
                event.data["command"] = Value::String(command.to_owned());
                event.title = Some(command.to_owned());
            }
        }
        ClaudeToolFamily::FileChange => {
            if let Some(patch) = synthesize_claude_patch(name, &input) {
                event.data["patch"] = Value::String(patch);
            }
        }
        ClaudeToolFamily::Tool => {}
    }
    event
}

/// A unified diff synthesized from a Claude file tool input. Claude never
/// ships a patch, so hunk positions are approximate (both sides anchor at
/// line 1); the `-`/`+` content is exact, which is what the inline patch and
/// diffstat render.
fn synthesize_claude_patch(name: &str, input: &Value) -> Option<String> {
    let path = input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .and_then(Value::as_str)?;
    let string_field = |value: &Value, field: &str| -> String {
        value
            .get(field)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let hunks: Vec<String> = match name {
        "Edit" => claude_diff_hunk(
            &string_field(input, "old_string"),
            &string_field(input, "new_string"),
        )
        .into_iter()
        .collect(),
        "MultiEdit" => input
            .get("edits")?
            .as_array()?
            .iter()
            .filter_map(|edit| {
                claude_diff_hunk(
                    &string_field(edit, "old_string"),
                    &string_field(edit, "new_string"),
                )
            })
            .collect(),
        "Write" => claude_diff_hunk("", &string_field(input, "content"))
            .into_iter()
            .collect(),
        "NotebookEdit" => claude_diff_hunk("", &string_field(input, "new_source"))
            .into_iter()
            .collect(),
        _ => return None,
    };
    if hunks.is_empty() {
        return None;
    }
    // A Write replaces or creates the whole file, so it reads as an addition.
    let old_file = if name == "Write" {
        "/dev/null".to_owned()
    } else {
        format!("a/{path}")
    };
    Some(format!(
        "--- {old_file}\n+++ b/{path}\n{}",
        hunks.join("\n")
    ))
}

/// One `@@` hunk turning `old` into `new`. The tool input carries no line
/// numbers, so an empty side ranges `-0,0`/`+0,0` and a present side `1,N`.
fn claude_diff_hunk(old: &str, new: &str) -> Option<String> {
    if old.is_empty() && new.is_empty() {
        return None;
    }
    let old_lines: Vec<&str> = if old.is_empty() {
        Vec::new()
    } else {
        old.lines().collect()
    };
    let new_lines: Vec<&str> = if new.is_empty() {
        Vec::new()
    } else {
        new.lines().collect()
    };
    let range = |lines: &[&str]| {
        if lines.is_empty() {
            "0,0".to_owned()
        } else {
            format!("1,{}", lines.len())
        }
    };
    let mut hunk = format!("@@ -{} +{} @@", range(&old_lines), range(&new_lines));
    for line in &old_lines {
        hunk.push_str(&format!("\n-{line}"));
    }
    for line in &new_lines {
        hunk.push_str(&format!("\n+{line}"));
    }
    Some(hunk)
}

/// The subject a provider asks approval for; one vocabulary so the same
/// action carries identical wording in every session, whichever provider
/// raised it.
enum ApprovalSubject<'a> {
    Command,
    FileChange,
    Tool(&'a str),
}

/// The shared approval-card title used by both the Claude and Codex paths.
fn approval_title(subject: ApprovalSubject) -> String {
    match subject {
        ApprovalSubject::Command => "Run this command?".into(),
        ApprovalSubject::FileChange => "Edit these files?".into(),
        ApprovalSubject::Tool(name) => {
            if let Some(rest) = name.strip_prefix("mcp__") {
                if let Some((server, tool)) = rest.split_once("__") {
                    return format!("Use {server} · {tool}?");
                }
            }
            match claude_tool_family(name) {
                ClaudeToolFamily::Command => approval_title(ApprovalSubject::Command),
                ClaudeToolFamily::FileChange => approval_title(ApprovalSubject::FileChange),
                ClaudeToolFamily::Tool => format!("Use {name}?"),
            }
        }
    }
}

pub fn normalize_claude_message_with_state(
    message: &Value,
    state: &mut ClaudeStreamState,
) -> Vec<NormalizedEvent> {
    let Some(kind) = message.get("type").and_then(Value::as_str) else {
        return vec![];
    };
    match kind {
        "system" => {
            // A new process starts its running totals from zero.
            if message.get("subtype").and_then(Value::as_str) == Some("init") {
                state.usage_cumulative = ClaudeUsageCumulative::default();
            }
            normalize_claude_system(message)
        }
        "stream_event" => normalize_claude_stream(message, state),
        "assistant" => normalize_claude_assistant(message, state),
        "user" => normalize_claude_user(message, state),
        "result" => {
            let mut events: Vec<_> = state
                .thinking_blocks
                .iter_mut()
                .filter_map(|(id, block)| complete_claude_thinking(id, block))
                .collect();
            let usage_cumulative = std::mem::take(&mut state.usage_cumulative);
            *state = ClaudeStreamState::default();
            state.usage_cumulative = usage_cumulative;
            events.extend(normalize_claude_result(message, state));
            events
        }
        "control_request" | "sdk_control_request" => normalize_claude_control_request(message)
            .into_iter()
            .collect(),
        _ => vec![],
    }
}

fn normalize_claude_system(message: &Value) -> Vec<NormalizedEvent> {
    let subtype = message
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("system");
    match subtype {
        "init" | "session_ready" => {
            let mut event = with_data("session.started", message, message.clone());
            event.status = Some("ready".into());
            vec![event]
        }
        "status" => {
            let status = message
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let mut event = with_data("session.status", message, message.clone());
            event.status = Some(status.into());
            if status == "requesting" {
                let mut turn = with_data(
                    "turn.started",
                    message,
                    json!({"turnId": message.get("uuid").cloned().unwrap_or(Value::Null)}),
                );
                turn.status = Some("working".into());
                return vec![event, turn];
            }
            vec![event]
        }
        "capability_discovery" => {
            let mut event = with_data("capability.discovery", message, message.clone());
            event.status = Some("degraded".into());
            event.title = Some("Claude capability discovery was incomplete".into());
            vec![event]
        }
        // Claude Code compacted its own context. This is the boundary Bridge
        // records rather than one it creates, and the only Claude frame that
        // reports the window actually shrinking: `pre_tokens` and
        // `post_tokens` are the provider's own figures.
        "compact_boundary" => {
            let metadata = message
                .get("compact_metadata")
                .cloned()
                .unwrap_or_else(|| json!({}));
            vec![native_compaction(
                "claude",
                json!({
                    "trigger": metadata.get("trigger").cloned(),
                    "preTokens": metadata.get("pre_tokens").cloned(),
                    "postTokens": metadata.get("post_tokens").cloned(),
                    "durationMs": metadata.get("duration_ms").cloned(),
                }),
            )]
        }
        // Hooks/notifications are noise in the conversation surface.
        _ => vec![],
    }
}

fn normalize_claude_stream(message: &Value, state: &mut ClaudeStreamState) -> Vec<NormalizedEvent> {
    let event = message.get("event").cloned().unwrap_or_else(|| json!({}));
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
    // Streams missing message_start still need one identity across block boundaries.
    let message_id = state
        .active_message_id
        .clone()
        .or_else(|| {
            message
                .get("session_id")
                .and_then(Value::as_str)
                .map(|session| format!("claude-live-{session}"))
        })
        .unwrap_or_else(|| "claude-live".into());
    match event_type {
        "message_start" => {
            if let Some(id) = event
                .pointer("/message/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                state.active_message_id = Some(id.clone());
                state.active_reasoning_id = Some(format!("reasoning-{id}"));
            }
            vec![]
        }
        "content_block_delta" => {
            let delta = event.get("delta").cloned().unwrap_or_else(|| json!({}));
            let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                    if text.is_empty() {
                        return vec![];
                    }
                    let mut normalized = NormalizedEvent::new("message.delta");
                    normalized.item_id = Some(message_id);
                    normalized.role = Some("assistant".into());
                    normalized.status = Some("streaming".into());
                    normalized.text = Some(text.to_owned());
                    vec![normalized]
                }
                "thinking_delta" | "reasoning_delta" => {
                    let text = delta
                        .get("thinking")
                        .or_else(|| delta.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if text.is_empty() {
                        return vec![];
                    }
                    let mut normalized = NormalizedEvent::new("reasoning.delta");
                    let id = claude_thinking_id(
                        &message_id,
                        event.get("index").and_then(Value::as_u64).unwrap_or(0),
                    );
                    let block = state.thinking_blocks.entry(id.clone()).or_default();
                    if block.completed {
                        return vec![];
                    }
                    block.text.push_str(text);
                    normalized.item_id = Some(id);
                    normalized.status = Some("streaming".into());
                    normalized.text = Some(text.to_owned());
                    vec![normalized]
                }
                _ => vec![],
            }
        }
        "content_block_start" => {
            let block = event
                .get("content_block")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match block.get("type").and_then(Value::as_str).unwrap_or("") {
                "tool_use" => vec![claude_tool_started(
                    message,
                    &block,
                    state,
                    ClaudeToolPhase::Preparing,
                )],
                "thinking" => {
                    let id = claude_thinking_id(
                        &message_id,
                        event.get("index").and_then(Value::as_u64).unwrap_or(0),
                    );
                    state.active_reasoning_id = Some(id.clone());
                    let thinking = state.thinking_blocks.entry(id.clone()).or_default();
                    let text = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                    if thinking.completed || text.is_empty() || !thinking.text.is_empty() {
                        return vec![];
                    }
                    thinking.text.push_str(text);
                    let mut normalized = NormalizedEvent::new("reasoning.delta");
                    normalized.item_id = Some(id);
                    normalized.status = Some("streaming".into());
                    normalized.text = Some(text.to_owned());
                    vec![normalized]
                }
                _ => vec![],
            }
        }
        "content_block_stop" => {
            let id = claude_thinking_id(
                &message_id,
                event.get("index").and_then(Value::as_u64).unwrap_or(0),
            );
            state
                .thinking_blocks
                .get_mut(&id)
                .and_then(|block| complete_claude_thinking(&id, block))
                .into_iter()
                .collect()
        }
        "message_stop" => {
            // A truncated stream may omit block_stop; retain its text durably.
            let prefix = format!("reasoning-{message_id}");
            state
                .thinking_blocks
                .iter_mut()
                .filter(|(id, _)| **id == prefix || id.starts_with(&format!("{prefix}-block-")))
                .filter_map(|(id, block)| complete_claude_thinking(id, block))
                .collect()
        }
        _ => vec![],
    }
}

fn normalize_claude_assistant(
    message: &Value,
    state: &mut ClaudeStreamState,
) -> Vec<NormalizedEvent> {
    let payload = message.get("message").cloned().unwrap_or_else(|| json!({}));
    let message_id = payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| state.active_message_id.clone())
        .or_else(|| {
            message
                .get("uuid")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "assistant".into());
    // A delayed snapshot must not redirect an already-started next message.
    if state.active_message_id.is_none() {
        state.active_message_id = Some(message_id.clone());
    }
    let content = payload
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut events = Vec::new();
    let text = content
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    if !text.is_empty() {
        let mut event = NormalizedEvent::new("message.completed");
        event.item_id = Some(message_id.clone());
        event.role = Some("assistant".into());
        event.status = Some("completed".into());
        event.text = Some(text);
        event.data = payload.clone();
        events.push(event);
    }
    for (block_index, part) in content.into_iter().enumerate() {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
        match part_type {
            "tool_use" => {
                events.push(claude_tool_started(
                    message,
                    &part,
                    state,
                    ClaudeToolPhase::Running,
                ));
            }
            "thinking" => {
                if let Some(thinking) = part.get("thinking").and_then(Value::as_str) {
                    if !thinking.is_empty() {
                        let id = claude_thinking_id(&message_id, block_index as u64);
                        let block = state.thinking_blocks.entry(id.clone()).or_default();
                        if block.text != thinking {
                            block.text = thinking.to_owned();
                            block.completed = false;
                        }
                        if let Some(event) = complete_claude_thinking(&id, block) {
                            events.push(event);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    events
}

fn normalize_claude_user(message: &Value, state: &mut ClaudeStreamState) -> Vec<NormalizedEvent> {
    let payload = message.get("message").cloned().unwrap_or_else(|| json!({}));
    let content = payload
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut events = Vec::new();
    for part in content {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "tool_result" => {
                let tool_id = part
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_owned();
                let call = state.tool_calls.remove(&tool_id);
                let family = call
                    .map(|call| call.family)
                    .unwrap_or(ClaudeToolFamily::Tool);
                let mut event = with_data(&family.kind("completed"), message, part.clone());
                event.item_id = Some(tool_id);
                if let Some(call) = call {
                    // Host-side wall time between tool_use and tool_result.
                    event.data["durationMs"] =
                        json!(u64::try_from(call.started_at.elapsed().as_millis()).unwrap_or(0));
                }
                event.status = Some(
                    if part
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        "failed"
                    } else {
                        "completed"
                    }
                    .into(),
                );
                event.text = part.get("content").and_then(|value| match value {
                    Value::String(text) => Some(text.clone()),
                    Value::Array(items) => Some(
                        items
                            .iter()
                            .filter_map(|item| item.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(""),
                    ),
                    _ => None,
                });
                // Keep tool cards compact in the GUI.
                if let Some(text) = &event.text {
                    event.data["aggregatedOutput"] = Value::String(text.clone());
                }
                events.push(event);
            }
            // User text echoes are already persisted by Bridge on send_turn.
            "text" => {}
            _ => {}
        }
    }
    events
}

fn normalize_claude_result(message: &Value, state: &mut ClaudeStreamState) -> Vec<NormalizedEvent> {
    let subtype = message
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("completed");
    let mut turn = with_data(
        "turn.completed",
        message,
        json!({"turn": {"status": subtype}, "result": message.get("result").cloned().unwrap_or(Value::Null)}),
    );
    turn.status = Some(if subtype == "success" {
        "completed".into()
    } else {
        subtype.into()
    });
    let mut events = vec![turn];
    if let Some(usage) = message.get("usage") {
        // `modelUsage` splits a multi-model turn per model; the ledger writes
        // one row per key and never the aggregate beside them. Cost and the
        // per-model counters arrive cumulative and leave here as this turn's
        // share (see `ClaudeUsageCumulative`).
        let total_cost_usd = claude_turn_cost(message, &mut state.usage_cumulative);
        let model_usage = claude_turn_model_usage(message, &mut state.usage_cumulative);
        events.push(with_data(
            "usage.updated",
            message,
            json!({
                "usage": usage,
                "totalCostUsd": total_cost_usd,
                "modelUsage": model_usage,
                "durationMs": message.get("duration_ms"),
                "durationApiMs": message.get("duration_api_ms"),
                "numTurns": message.get("num_turns"),
            }),
        ));
    }
    if let Some(denials) = message.get("permission_denials").and_then(Value::as_array) {
        for denial in denials {
            let redacted = json!({
                "tool_name": denial.get("tool_name").cloned(),
                "tool_use_id": denial.get("tool_use_id").cloned(),
            });
            let mut event = with_data("permission.denied", &redacted, redacted.clone());
            event.status = Some("denied".into());
            event.title = denial
                .get("tool_name")
                .and_then(Value::as_str)
                .map(|name| format!("Bridge denied {name}"));
            events.push(event);
        }
    }
    if message
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || subtype.contains("error")
    {
        let mut redacted = message.clone();
        if let Some(denials) = redacted
            .get_mut("permission_denials")
            .and_then(Value::as_array_mut)
        {
            for denial in denials {
                *denial = json!({
                    "tool_name": denial.get("tool_name").cloned(),
                    "tool_use_id": denial.get("tool_use_id").cloned(),
                });
            }
        }
        let mut error = with_data("error", &redacted, redacted.clone());
        error.status = Some("failed".into());
        error.text = message
            .get("result")
            .and_then(Value::as_str)
            .or_else(|| message.get("error").and_then(Value::as_str))
            .map(str::to_owned)
            .or_else(|| Some(format!("Claude turn ended with {subtype}")));
        events.push(error);
    }
    events
}

fn normalize_claude_control_request(message: &Value) -> Option<NormalizedEvent> {
    let request = message
        .get("request")
        .cloned()
        .or_else(|| message.get("control_request").cloned())
        .unwrap_or_else(|| message.clone());
    let subtype = request
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("permission");
    if subtype != "permission" && subtype != "can_use_tool" {
        return None;
    }
    let mut event = with_data("permission.requested", message, request.clone());
    event.item_id = request
        .get("tool_use_id")
        .or_else(|| request.get("request_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.title = Some(
        request
            .get("tool_name")
            .and_then(Value::as_str)
            .map(ApprovalSubject::Tool)
            .map(approval_title)
            .unwrap_or_else(|| "Use tool?".into()),
    );
    event.text = request
        .pointer("/tool_input/command")
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.status = Some("pending".into());
    event.data["requestId"] = message
        .get("request_id")
        .cloned()
        .or_else(|| request.get("request_id").cloned())
        .unwrap_or(Value::Null);
    event.data["command"] = request
        .pointer("/tool_input/command")
        .cloned()
        .unwrap_or(Value::Null);
    event.data["interactionKind"] = Value::String("permission".into());
    event.data["actions"] = permission_actions(true);
    Some(event)
}

fn is_opencode_file_tool(tool: &str) -> bool {
    matches!(tool, "edit" | "write" | "patch" | "apply_patch" | "multiedit")
}

/// Stamp Claude-shaped `path`/`patch`/`additions`/`deletions` (or `paths[]`
/// for a multi-file edit) onto a Codex `fileChange` or OpenCode file-tool
/// payload. The frontend used to guess at those keys in the raw provider
/// JSON; two of three providers did not put them where it looked, so the
/// row fell back to the word "files".
fn stamp_file_change_fields(data: &mut Value) {
    let extracted = extract_file_change(data);
    let Some(object) = data.as_object_mut() else {
        return;
    };
    if json_nonempty_str(object.get("path")).is_none() {
        if let Some(path) = extracted.paths.first() {
            object.insert("path".into(), json!(path));
        }
    }
    if extracted.paths.len() > 1 && object.get("paths").is_none() {
        object.insert("paths".into(), json!(extracted.paths));
    }
    if json_nonempty_str(object.get("patch")).is_none() {
        if let Some(patch) = extracted.patch {
            object.insert("patch".into(), json!(patch));
        }
    }
    if json_count(object.get("additions")).is_none() {
        if let Some(additions) = extracted.additions {
            object.insert("additions".into(), json!(additions));
        }
    }
    if json_count(object.get("deletions")).is_none() {
        if let Some(deletions) = extracted.deletions {
            object.insert("deletions".into(), json!(deletions));
        }
    }
}

struct ExtractedFileChange {
    paths: Vec<String>,
    patch: Option<String>,
    additions: Option<u64>,
    deletions: Option<u64>,
}

fn extract_file_change(data: &Value) -> ExtractedFileChange {
    let input = file_change_input(data);
    let metadata = file_change_metadata(data);
    let mut paths = Vec::new();
    push_path(&mut paths, json_text(data, &["path", "file_path", "filePath"]));
    push_path(
        &mut paths,
        json_text(input, &["file_path", "filePath", "path", "notebook_path"]),
    );
    push_path(
        &mut paths,
        json_text(metadata, &["filepath", "filePath", "file_path", "path"]),
    );
    push_path(
        &mut paths,
        metadata
            .get("filediff")
            .and_then(|filediff| json_text(filediff, &["file", "path"])),
    );
    collect_entry_paths(data.get("changes"), &mut paths);
    collect_entry_paths(input.get("changes"), &mut paths);
    collect_entry_paths(metadata.get("files"), &mut paths);
    collect_entry_paths(data.get("files"), &mut paths);
    if let Some(patch_text) = json_text(input, &["patchText", "patch_text"]) {
        for path in paths_from_apply_patch(patch_text) {
            push_path(&mut paths, Some(&path));
        }
    }

    let change_patch = join_change_patches(data.get("changes"))
        .or_else(|| join_change_patches(input.get("changes")));
    let file_patch = join_file_patches(metadata.get("files"))
        .or_else(|| join_file_patches(data.get("files")));
    let patch = first_patch(data)
        .or_else(|| {
            metadata
                .get("filediff")
                .and_then(|filediff| json_patch(filediff, &["patch", "diff"]))
        })
        .or_else(|| first_patch(metadata))
        .or_else(|| first_patch(input))
        .map(str::to_owned)
        .or(change_patch)
        .or(file_patch)
        .or_else(|| {
            json_patch(input, &["patchText", "patch_text"])
                .filter(|text| looks_like_patch(text))
                .map(str::to_owned)
        })
        .or_else(|| synthesize_opencode_file_patch(input, paths.first().map(String::as_str)));

    let mut additions = json_count(data.get("additions"))
        .or_else(|| json_count(metadata.get("additions")))
        .or_else(|| {
            metadata
                .get("filediff")
                .and_then(|filediff| json_count(filediff.get("additions")))
        })
        .or_else(|| sum_entry_counts(metadata.get("files"), "additions"))
        .or_else(|| sum_entry_counts(data.get("files"), "additions"))
        .or_else(|| sum_entry_counts(data.get("changes"), "additions"));
    let mut deletions = json_count(data.get("deletions"))
        .or_else(|| json_count(metadata.get("deletions")))
        .or_else(|| {
            metadata
                .get("filediff")
                .and_then(|filediff| json_count(filediff.get("deletions")))
        })
        .or_else(|| sum_entry_counts(metadata.get("files"), "deletions"))
        .or_else(|| sum_entry_counts(data.get("files"), "deletions"))
        .or_else(|| sum_entry_counts(data.get("changes"), "deletions"));
    if let Some(patch) = &patch {
        if additions.is_none() || deletions.is_none() {
            let (from_additions, from_deletions) = diffstat(patch);
            additions = additions.or(Some(from_additions));
            deletions = deletions.or(Some(from_deletions));
        }
    }

    ExtractedFileChange {
        paths,
        patch,
        additions,
        deletions,
    }
}

fn file_change_input(data: &Value) -> &Value {
    data.pointer("/state/input")
        .or_else(|| data.get("input"))
        .unwrap_or(&Value::Null)
}

fn file_change_metadata(data: &Value) -> &Value {
    data.pointer("/state/metadata")
        .or_else(|| data.get("metadata"))
        .unwrap_or(&Value::Null)
}

fn collect_entry_paths(entries: Option<&Value>, paths: &mut Vec<String>) {
    let Some(entries) = entries.and_then(Value::as_array) else {
        return;
    };
    for entry in entries {
        push_path(
            paths,
            json_text(entry, &["path", "file", "filePath", "file_path"]),
        );
    }
}

fn join_change_patches(entries: Option<&Value>) -> Option<String> {
    let entries = entries.and_then(Value::as_array)?;
    let patches: Vec<String> = entries
        .iter()
        .filter_map(|entry| {
            let diff = json_patch(entry, &["diff", "patch", "unifiedDiff"])?;
            let path = json_text(entry, &["path", "file", "filePath", "file_path"]).unwrap_or("file");
            let kind = json_text(entry, &["kind", "type"]);
            Some(normalize_provider_diff(path, kind, diff))
        })
        .collect();
    (!patches.is_empty()).then(|| patches.join("\n"))
}

fn join_file_patches(entries: Option<&Value>) -> Option<String> {
    let entries = entries.and_then(Value::as_array)?;
    let patches: Vec<&str> = entries
        .iter()
        .filter_map(|entry| json_patch(entry, &["patch", "diff", "unifiedDiff"]))
        .collect();
    (!patches.is_empty()).then(|| patches.join("\n"))
}

fn normalize_provider_diff(path: &str, kind: Option<&str>, diff: &str) -> String {
    if looks_like_patch(diff) {
        return diff.to_owned();
    }
    match kind {
        Some("add") | Some("added") => synthesize_file_patch(path, "", diff, true)
            .unwrap_or_else(|| diff.to_owned()),
        Some("delete") | Some("deleted") => synthesize_file_patch(path, diff, "", false)
            .unwrap_or_else(|| diff.to_owned()),
        _ => synthesize_file_patch(path, "", diff, false).unwrap_or_else(|| diff.to_owned()),
    }
}

fn synthesize_opencode_file_patch(input: &Value, path: Option<&str>) -> Option<String> {
    let path = path?;
    if let Some(content) = json_patch(input, &["content", "new_source"]) {
        return synthesize_file_patch(path, "", content, true);
    }
    let old = json_patch(input, &["oldString", "old_string"]).unwrap_or("");
    let new = json_patch(input, &["newString", "new_string"]).unwrap_or("");
    if old.is_empty() && new.is_empty() {
        if let Some(edits) = input.get("edits").and_then(Value::as_array) {
            let hunks: Vec<String> = edits
                .iter()
                .filter_map(|edit| {
                    claude_diff_hunk(
                        json_patch(edit, &["oldString", "old_string"]).unwrap_or(""),
                        json_patch(edit, &["newString", "new_string"]).unwrap_or(""),
                    )
                })
                .collect();
            if hunks.is_empty() {
                return None;
            }
            return Some(format!("--- a/{path}\n+++ b/{path}\n{}", hunks.join("\n")));
        }
        return None;
    }
    synthesize_file_patch(path, old, new, old.is_empty())
}

fn synthesize_file_patch(path: &str, old: &str, new: &str, created: bool) -> Option<String> {
    let hunk = claude_diff_hunk(old, new)?;
    let old_file = if created || old.is_empty() {
        "/dev/null".to_owned()
    } else {
        format!("a/{path}")
    };
    Some(format!("--- {old_file}\n+++ b/{path}\n{hunk}"))
}

fn paths_from_apply_patch(text: &str) -> Vec<String> {
    const MARKERS: [&str; 4] = [
        "*** Update File: ",
        "*** Add File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ];
    let mut paths = Vec::new();
    for line in text.lines() {
        for marker in MARKERS {
            if let Some(path) = line.strip_prefix(marker) {
                push_path(&mut paths, Some(path));
            }
        }
    }
    paths
}

fn first_patch(value: &Value) -> Option<&str> {
    json_patch(value, &["patch", "diff", "unifiedDiff"])
}

fn json_patch<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
    })
}

fn looks_like_patch(text: &str) -> bool {
    let sample = if text.len() > 4000 { &text[..4000] } else { text };
    sample.contains("\n@@")
        || sample.starts_with("@@")
        || sample.contains("\ndiff --git ")
        || sample.starts_with("diff --git ")
        || sample.contains("\n*** Update File: ")
        || sample.contains("\n*** Add File: ")
        || sample.starts_with("*** Begin Patch")
}

fn diffstat(patch: &str) -> (u64, u64) {
    let mut additions = 0_u64;
    let mut deletions = 0_u64;
    for line in patch.lines() {
        if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("@@")
            || line.starts_with("diff ")
            || line.starts_with("index ")
            || line.starts_with("Index:")
            || line.starts_with("====")
            || line.starts_with("***")
            || line.starts_with('\\')
        {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
        } else if line.starts_with('-') {
            deletions += 1;
        }
    }
    (additions, deletions)
}

fn push_path(paths: &mut Vec<String>, candidate: Option<&str>) {
    let Some(path) = candidate.map(str::trim).filter(|path| !path.is_empty()) else {
        return;
    };
    if !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_owned());
    }
}

fn json_text<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })
}

fn json_nonempty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn json_count(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_i64().filter(|count| *count >= 0).map(|count| count as u64))
}

fn sum_entry_counts(entries: Option<&Value>, key: &str) -> Option<u64> {
    let entries = entries.and_then(Value::as_array)?;
    let mut total = 0_u64;
    let mut found = false;
    for entry in entries {
        if let Some(count) = json_count(entry.get(key)) {
            total += count;
            found = true;
        }
    }
    found.then_some(total)
}

fn file_change_title(data: &Value) -> Option<String> {
    let path = json_nonempty_str(data.get("path"))
        .or_else(|| {
            data.get("paths")
                .and_then(Value::as_array)
                .and_then(|paths| paths.first())
                .and_then(Value::as_str)
        })?;
    Some(
        path.rsplit(['/', '\\'])
            .find(|part| !part.is_empty())
            .unwrap_or(path)
            .to_owned(),
    )
}

fn file_change_title_is_generic(title: Option<&str>) -> bool {
    matches!(
        title.map(str::trim),
        None | Some("") | Some("edit") | Some("write") | Some("patch") | Some("apply_patch") | Some("multiedit") | Some("fileChange") | Some("tool")
    )
}

fn with_data(kind: &str, params: &Value, data: Value) -> NormalizedEvent {
    let mut event = NormalizedEvent::new(kind);
    event.item_id = params
        .get("itemId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.data = data;
    event
}

/// The kind every harness's own compaction boundary normalizes to.
///
/// Bridge does not compact a live provider context: the harness that talks to
/// the model owns its window and shrinks it. This event is Bridge's durable
/// record that the shrink happened, and it is the only thing the "Context
/// compacted" card is ever drawn from. A Bridge checkpoint is a separate fact
/// with its own entry, because on a hot session it frees no provider tokens.
/// See `docs/compaction-and-resume.md`.
pub const NATIVE_COMPACTION_KIND: &str = "context.compacted";

/// One native compaction boundary, in the shape the transcript reads.
///
/// `facts` carries whatever the provider actually reported. Null members are
/// dropped rather than stored, so a harness that reports no token figures
/// produces an entry that says only which harness compacted, and the card
/// cannot claim a number the provider never sent.
/// Whether this turn's Codex compaction boundary is already in history.
///
/// Claims the turn on the first call, so the caller records the boundary once
/// however many ways Codex reports it. A frame with no turn id claims the
/// literal `"unknown"` turn, which still collapses a same-frame pair.
fn compaction_already_recorded(state: &mut CodexStreamState, params: &Value) -> bool {
    let turn = params
        .get("turnId")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    if state.compacted_turn.as_deref() == Some(turn.as_str()) {
        return true;
    }
    state.compacted_turn = Some(turn);
    false
}

fn native_compaction(harness: &str, facts: Value) -> NormalizedEvent {
    let mut data = json!({"harness": harness});
    if let (Some(target), Some(facts)) = (data.as_object_mut(), facts.as_object()) {
        for (key, value) in facts {
            if !value.is_null() {
                target.insert(key.clone(), value.clone());
            }
        }
    }
    let mut event = NormalizedEvent::new(NATIVE_COMPACTION_KIND);
    event.data = data;
    event.status = Some("completed".into());
    event.title = Some("Context compacted".into());
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_streaming_assistant_delta() {
        let events = normalize_codex_message(
            &json!({"method":"item/agentMessage/delta","params":{"itemId":"m1","delta":"hello"}}),
        );
        assert_eq!(events[0].kind, "message.delta");
        assert_eq!(events[0].role.as_deref(), Some("assistant"));
        assert_eq!(events[0].text.as_deref(), Some("hello"));
    }
    #[test]
    fn normalizes_codex_reasoning_deltas_with_stable_item_id() {
        let mut state = CodexStreamState::default();
        let _ = normalize_codex_message_with_state(
            &json!({"method":"item/started","params":{"item":{"type":"reasoning","id":"reasoning-42"}}}),
            &mut state,
        );
        let deltas = normalize_codex_message_with_state(
            &json!({"method":"item/reasoning/textDelta","params":{"delta":"Thinking line 1\n"}}),
            &mut state,
        );
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].kind, "reasoning.delta");
        assert_eq!(deltas[0].item_id.as_deref(), Some("reasoning-42"));
        assert_eq!(deltas[0].text.as_deref(), Some("Thinking line 1\n"));

        let summary = normalize_codex_message_with_state(
            &json!({"method":"item/reasoning/summaryPartAdded","params":{"summary":"Step completed"}}),
            &mut state,
        );
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].kind, "reasoning.delta");
        assert_eq!(summary[0].item_id.as_deref(), Some("reasoning-42"));
        assert_eq!(summary[0].text.as_deref(), Some("Step completed"));

        let completed = normalize_codex_message_with_state(
            &json!({"method":"item/completed","params":{"item":{"type":"reasoning","id":"reasoning-42","status":"completed"}}}),
            &mut state,
        );
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].kind, "reasoning.completed");
        assert_eq!(completed[0].status.as_deref(), Some("completed"));
        assert_eq!(state.active_reasoning_id, None);

        // Turn 1 fallback reasoning ID
        let t1_delta = normalize_codex_message_with_state(
            &json!({"method":"item/reasoning/textDelta","params":{"delta":"Turn 1 reasoning"}}),
            &mut state,
        );
        assert_eq!(t1_delta[0].item_id.as_deref(), Some("reasoning-1"));

        // Turn 1 completes, turn 2 starts
        let _ = normalize_codex_message_with_state(
            &json!({"method":"turn/completed","params":{"turn":{"status":"completed"}}}),
            &mut state,
        );
        let _ = normalize_codex_message_with_state(
            &json!({"method":"turn/started","params":{"turn":{"id":"turn-2"}}}),
            &mut state,
        );

        // Turn 2 fallback reasoning ID must be different from turn 1
        let t2_delta = normalize_codex_message_with_state(
            &json!({"method":"item/reasoning/textDelta","params":{"delta":"Turn 2 reasoning"}}),
            &mut state,
        );
        assert_eq!(t2_delta[0].item_id.as_deref(), Some("reasoning-2"));
    }
    #[test]
    fn normalizes_codex_turn_completed_with_failure_synthesizes_error() {
        let events = normalize_codex_message(&json!({
            "method": "turn/completed",
            "params": {
                "turn": {
                    "status": "failed",
                    "error": { "message": "Model hit rate limit or context overload" }
                }
            }
        }));
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "turn.completed");
        assert_eq!(events[0].status.as_deref(), Some("failed"));
        assert_eq!(events[1].kind, "error");
        assert_eq!(events[1].status.as_deref(), Some("failed"));
        assert_eq!(events[1].title.as_deref(), Some("Codex turn failed"));
        assert_eq!(events[1].text.as_deref(), Some("Model hit rate limit or context overload"));
    }
    #[test]
    fn claude_compact_boundary_becomes_a_durable_context_compaction() {
        let events = normalize_claude_message(&json!({
            "type":"system",
            "subtype":"compact_boundary",
            "session_id":"s1",
            "uuid":"u1",
            "compact_metadata":{
                "trigger":"auto",
                "pre_tokens":184_000,
                "post_tokens":22_500,
                "duration_ms":4_120
            }
        }));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NATIVE_COMPACTION_KIND);
        assert_eq!(events[0].status.as_deref(), Some("completed"));
        assert_eq!(events[0].data["harness"], "claude");
        assert_eq!(events[0].data["trigger"], "auto");
        assert_eq!(events[0].data["preTokens"], 184_000);
        assert_eq!(events[0].data["postTokens"], 22_500);
        assert_eq!(events[0].data["durationMs"], 4_120);
    }

    #[test]
    fn a_compaction_records_only_the_figures_the_provider_sent() {
        // A boundary that summarized everything reports no `post_tokens`. The
        // entry must omit the key rather than store a zero the card would then
        // render as "shrank to nothing".
        let events = normalize_claude_message(&json!({
            "type":"system",
            "subtype":"compact_boundary",
            "session_id":"s1",
            "uuid":"u1",
            "compact_metadata":{"trigger":"manual","pre_tokens":90_000}
        }));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data["preTokens"], 90_000);
        assert!(events[0].data.get("postTokens").is_none());
        assert!(events[0].data.get("durationMs").is_none());
        assert_eq!(events[0].data["trigger"], "manual");
    }

    #[test]
    fn codex_context_compaction_item_is_recorded_once_per_turn() {
        let mut state = CodexStreamState::default();
        let started = normalize_codex_message_with_state(
            &json!({"method":"item/started","params":{"threadId":"t1","turnId":"turn-1",
                    "item":{"id":"i1","type":"contextCompaction"}}}),
            &mut state,
        );
        assert!(
            started.is_empty(),
            "the opening half of a boundary carries nothing to record"
        );
        let completed = normalize_codex_message_with_state(
            &json!({"method":"item/completed","params":{"threadId":"t1","turnId":"turn-1",
                    "item":{"id":"i1","type":"contextCompaction"}}}),
            &mut state,
        );
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].kind, NATIVE_COMPACTION_KIND);
        assert_eq!(completed[0].data["harness"], "codex");
        assert_eq!(completed[0].item_id.as_deref(), Some("i1"));
        assert!(
            completed[0].data.get("preTokens").is_none(),
            "Codex reports no token figures with a boundary"
        );

        // The deprecated notification describes the same boundary.
        let echo = normalize_codex_message_with_state(
            &json!({"method":"thread/compacted","params":{"threadId":"t1","turnId":"turn-1"}}),
            &mut state,
        );
        assert!(echo.is_empty(), "one boundary is one entry");
    }

    #[test]
    fn codex_thread_compacted_is_no_longer_swallowed() {
        // The path an older Codex takes: the deprecated notification is the
        // only report it sends, and it used to be dropped entirely.
        let mut state = CodexStreamState::default();
        let events = normalize_codex_message_with_state(
            &json!({"method":"thread/compacted","params":{"threadId":"t1","turnId":"turn-1"}}),
            &mut state,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NATIVE_COMPACTION_KIND);
        assert_eq!(events[0].data["harness"], "codex");

        // A later turn compacting is its own boundary.
        let _ = normalize_codex_message_with_state(
            &json!({"method":"turn/started","params":{"threadId":"t1","turnId":"turn-2"}}),
            &mut state,
        );
        let next = normalize_codex_message_with_state(
            &json!({"method":"thread/compacted","params":{"threadId":"t1","turnId":"turn-2"}}),
            &mut state,
        );
        assert_eq!(next.len(), 1);
    }

    #[test]
    fn opencode_session_compacted_is_not_a_provider_unknown() {
        let mut state = OpenCodeStreamState::default();
        let events = normalize_opencode_message_with_state(
            &json!({"type":"session.compacted","properties":{"sessionID":"s1"}}),
            &mut state,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NATIVE_COMPACTION_KIND);
        assert_eq!(events[0].data["harness"], "opencode");
    }

    #[test]
    fn codex_internal_notifications_do_not_become_unknown_events() {
        for method in [
            "hook/started",
            "hook/completed",
            "process/exited",
            "item/commandExecution/terminalInteraction",
            "mcpServer/event/stream/notification",
            "fs/changed",
        ] {
            assert!(
                normalize_codex_message(&json!({"method": method, "params": {"value": 7}}))
                    .is_empty(),
                "{method} is documented internal traffic"
            );
        }
        let future =
            normalize_codex_message(&json!({"method":"future/newThing","params":{"value":7}}));
        assert_eq!(future.len(), 1);
        assert_eq!(future[0].kind, "provider.unknown");
    }

    #[test]
    fn codex_user_significant_notifications_remain_visible() {
        let rerouted = normalize_codex_message(&json!({
            "method":"model/rerouted",
            "params":{
                "threadId":"thread-1",
                "turnId":"turn-1",
                "fromModel":"gpt-5.6-sol",
                "toModel":"gpt-5.6-terra",
                "reason":"highRiskCyberActivity"
            }
        }));
        assert_eq!(rerouted[0].kind, "model.rerouted");
        assert_eq!(rerouted[0].title.as_deref(), Some("Model rerouted"));
        assert!(rerouted[0]
            .text
            .as_deref()
            .unwrap()
            .contains("gpt-5.6-terra"));

        let realtime_error = normalize_codex_message(&json!({
            "method":"thread/realtime/error",
            "params":{"threadId":"thread-1","message":"voice transport disconnected"}
        }));
        assert_eq!(realtime_error[0].kind, "error");
        assert_eq!(realtime_error[0].status.as_deref(), Some("failed"));
        assert_eq!(
            realtime_error[0].text.as_deref(),
            Some("voice transport disconnected")
        );
    }

    #[test]
    fn codex_progress_keeps_display_text_without_duplicating_params() {
        let events = normalize_codex_message(&json!({
            "method":"item/mcpToolCall/progress",
            "params":{"itemId":"tool-1","message":"Searching 20 files","large":"payload"}
        }));
        assert_eq!(events[0].kind, "tool.progress");
        assert_eq!(events[0].item_id.as_deref(), Some("tool-1"));
        assert_eq!(events[0].text.as_deref(), Some("Searching 20 files"));
        assert_eq!(events[0].data, json!({}));
    }
    #[test]
    fn converts_server_request_to_permission() {
        let event=normalize_codex_request(&json!({"id":42,"method":"item/commandExecution/requestApproval","params":{"itemId":"c1","command":"cargo test","reason":"needs access"}})).unwrap();
        assert_eq!(event.kind, "permission.requested");
        assert_eq!(event.data["interactionKind"], "permission");
        assert!(event.data["actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action["decision"] == "acceptForSession"));
        assert_eq!(event.data["requestId"], 42);
        assert_eq!(event.status.as_deref(), Some("pending"));
        assert_eq!(event.title.as_deref(), Some("Run this command?"));
    }
    #[test]
    fn rejects_invalid_normalized_roles() {
        let mut event = NormalizedEvent::new("message.completed");
        event.role = Some("provider-special-role".into());
        assert!(event
            .validate()
            .unwrap_err()
            .contains("unsupported normalized role"));
    }
    #[test]
    fn normalizes_command_output_delta() {
        let events = normalize_codex_message(
            &json!({"method":"item/commandExecution/outputDelta","params":{"itemId":"c1","delta":"ok\n"}}),
        );
        assert_eq!(events[0].kind, "command.output_delta");
        assert_eq!(events[0].text.as_deref(), Some("ok\n"));
    }

    #[test]
    fn normalizes_claude_text_delta() {
        let mut state = ClaudeStreamState::default();
        let _ = normalize_claude_message_with_state(
            &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_1"}}}),
            &mut state,
        );
        let events = normalize_claude_message_with_state(
            &json!({
                "type":"stream_event",
                "event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}
            }),
            &mut state,
        );
        assert_eq!(events[0].kind, "message.delta");
        assert_eq!(events[0].item_id.as_deref(), Some("msg_1"));
        assert_eq!(events[0].role.as_deref(), Some("assistant"));
        assert_eq!(events[0].text.as_deref(), Some("hi"));
    }

    #[test]
    fn coalesces_claude_stream_and_completed_into_one_item_id() {
        let mut state = ClaudeStreamState::default();
        let _ = normalize_claude_message_with_state(
            &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_9"}}}),
            &mut state,
        );
        let delta = normalize_claude_message_with_state(
            &json!({"type":"stream_event","uuid":"a","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}),
            &mut state,
        );
        let delta2 = normalize_claude_message_with_state(
            &json!({"type":"stream_event","uuid":"b","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}}),
            &mut state,
        );
        let completed = normalize_claude_message_with_state(
            &json!({"type":"assistant","message":{"id":"msg_9","content":[{"type":"text","text":"Hello"}]}}),
            &mut state,
        );
        assert_eq!(delta[0].item_id, delta2[0].item_id);
        assert_eq!(delta[0].item_id.as_deref(), Some("msg_9"));
        assert_eq!(completed[0].item_id.as_deref(), Some("msg_9"));
        assert_eq!(completed[0].kind, "message.completed");
    }

    #[test]
    fn claude_block_stop_completes_before_answer_and_reconciles_snapshot() {
        let mut state = ClaudeStreamState::default();
        let frames = [
            json!({"type":"message_start","message":{"id":"m1"}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Check "}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"facts."}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Answer."}}),
        ];
        let events: Vec<_> = frames
            .into_iter()
            .flat_map(|event| {
                normalize_claude_message_with_state(
                    &json!({"type":"stream_event","event":event}),
                    &mut state,
                )
            })
            .collect();
        assert_eq!(
            events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec![
                "reasoning.delta",
                "reasoning.delta",
                "reasoning.completed",
                "message.delta"
            ]
        );
        assert_eq!(events[2].text.as_deref(), Some("Check facts."));
        assert_eq!(events[2].item_id, events[0].item_id);
        let snapshot = normalize_claude_message_with_state(
            &json!({"type":"assistant","message":{"id":"m1","content":[
                {"type":"thinking","thinking":"Check facts."},{"type":"text","text":"Answer."}
            ]}}),
            &mut state,
        );
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].kind, "message.completed");
        // Duplicate stop and a stray delta cannot reopen the completed thought.
        for event in [
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"late"}}),
        ] {
            assert!(normalize_claude_message_with_state(
                &json!({"type":"stream_event","event":event}),
                &mut state
            )
            .is_empty());
        }
    }

    #[test]
    fn claude_thinking_fallback_identity_matches_across_block_lifecycle() {
        for session_id in [Some("session-1"), None] {
            for index in [0, 2] {
                for initial_text in [None, Some(""), Some("Check ")] {
                    for stop_type in ["content_block_stop", "message_stop"] {
                        let mut state = ClaudeStreamState::default();
                        let wrap = |event: Value| {
                            let mut message = json!({"type":"stream_event","event":event});
                            if let Some(session_id) = session_id {
                                message["session_id"] = json!(session_id);
                            }
                            message
                        };
                        let mut deltas = Vec::new();
                        if let Some(text) = initial_text {
                            deltas.extend(normalize_claude_message_with_state(
                                &wrap(json!({"type":"content_block_start","index":index,
                                    "content_block":{"type":"thinking","thinking":text}})),
                                &mut state,
                            ));
                        }
                        for text in ["the ", "facts."] {
                            deltas.extend(normalize_claude_message_with_state(
                                &wrap(json!({"type":"content_block_delta","index":index,
                                    "delta":{"type":"thinking_delta","thinking":text}})),
                                &mut state,
                            ));
                        }
                        let message_id = session_id
                            .map(|id| format!("claude-live-{id}"))
                            .unwrap_or_else(|| "claude-live".into());
                        let expected_id = if index == 0 {
                            format!("reasoning-{message_id}")
                        } else {
                            format!("reasoning-{message_id}-block-{index}")
                        };
                        for delta in &deltas {
                            assert_eq!(delta.kind, "reasoning.delta");
                            assert_eq!(delta.item_id.as_deref(), Some(expected_id.as_str()));
                        }
                        let stop = wrap(json!({"type":stop_type,"index":index}));
                        let completed = normalize_claude_message_with_state(&stop, &mut state);
                        assert_eq!(
                            completed.len(), 1,
                            "{session_id:?}, {index}, {initial_text:?}, {stop_type}"
                        );
                        assert_eq!(completed[0].kind, "reasoning.completed");
                        assert_eq!(completed[0].status.as_deref(), Some("completed"));
                        assert_eq!(completed[0].item_id.as_deref(), Some(expected_id.as_str()));
                        assert_eq!(
                            completed[0].text,
                            Some(format!("{}the facts.", initial_text.unwrap_or("")))
                        );
                        assert_eq!(state.thinking_blocks.len(), 1);
                        assert!(normalize_claude_message_with_state(&stop, &mut state).is_empty());
                        let answer = normalize_claude_message_with_state(
                            &wrap(json!({"type":"content_block_delta","index":index + 1,
                                "delta":{"type":"text_delta","text":"Answer."}})),
                            &mut state,
                        );
                        assert_eq!(answer[0].kind, "message.delta");
                        assert_eq!(answer[0].item_id.as_deref(), Some(message_id.as_str()));
                    }
                }
            }
        }
    }

    #[test]
    fn claude_thinking_blocks_and_delayed_snapshots_keep_message_identity() {
        let mut state = ClaudeStreamState::default();
        for id in ["m1", "m2"] {
            normalize_claude_message_with_state(
                &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":id}}}),
                &mut state,
            );
            for index in [0, 2] {
                let delta = normalize_claude_message_with_state(
                    &json!({"type":"stream_event","event":{"type":"content_block_delta","index":index,"delta":{"type":"thinking_delta","thinking":"thought"}}}),
                    &mut state,
                );
                let stop = normalize_claude_message_with_state(
                    &json!({"type":"stream_event","event":{"type":"content_block_stop","index":index}}),
                    &mut state,
                );
                assert_eq!(stop[0].item_id, delta[0].item_id);
                assert_eq!(
                    stop[0].item_id.as_deref(),
                    Some(claude_thinking_id(id, index).as_str())
                );
            }
        }
        let late = normalize_claude_message_with_state(
            &json!({"type":"assistant","message":{"id":"m1","content":[
                {"type":"thinking","thinking":"thought"},{"type":"text","text":"answer"},{"type":"thinking","thinking":"thought"}
            ]}}),
            &mut state,
        );
        assert_eq!(late.len(), 1);
        assert_eq!(state.active_message_id.as_deref(), Some("m2"));
    }

    #[test]
    fn claude_interrupted_thinking_retains_text_and_resets_state() {
        let mut state = ClaudeStreamState::default();
        normalize_claude_message_with_state(
            &json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m"}}}),
            &mut state,
        );
        normalize_claude_message_with_state(
            &json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"partial"}}}),
            &mut state,
        );
        // Stops for text/tool blocks must not settle another block.
        assert!(normalize_claude_message_with_state(
            &json!({"type":"stream_event","event":{"type":"content_block_stop","index":1}}),
            &mut state
        )
        .is_empty());
        let end = normalize_claude_message_with_state(
            &json!({"type":"result","is_error":true,"subtype":"error_during_execution"}),
            &mut state,
        );
        assert_eq!(end[0].kind, "reasoning.completed");
        assert_eq!(end[0].text.as_deref(), Some("partial"));
        assert!(state.thinking_blocks.is_empty());
    }

    #[test]
    fn skips_empty_claude_thinking_deltas() {
        let events = normalize_claude_message(&json!({
            "type":"stream_event",
            "event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}
        }));
        assert!(events.is_empty());
    }

    #[test]
    fn normalizes_claude_bash_as_command_with_duration() {
        let mut state = ClaudeStreamState::default();
        let started = normalize_claude_message_with_state(
            &json!({
                "type":"assistant",
                "message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pwd"}}]}
            }),
            &mut state,
        );
        assert_eq!(started[0].kind, "command.started");
        assert_eq!(started[0].title.as_deref(), Some("pwd"));
        assert_eq!(started[0].data["command"], "pwd");
        assert_eq!(started[0].status.as_deref(), Some("inProgress"));
        let completed = normalize_claude_message_with_state(
            &json!({
                "type":"user",
                "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"/tmp","is_error":false}]}
            }),
            &mut state,
        );
        assert_eq!(completed[0].kind, "command.completed");
        assert_eq!(completed[0].text.as_deref(), Some("/tmp"));
        assert_eq!(completed[0].data["aggregatedOutput"], "/tmp");
        assert!(completed[0].data["durationMs"].is_u64());
    }

    #[test]
    fn normalizes_claude_edit_as_file_change_and_unknown_tool_stays_tool() {
        let mut state = ClaudeStreamState::default();
        let started = normalize_claude_message_with_state(
            &json!({
                "type":"assistant",
                "message":{"id":"m1","content":[
                    {"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"src/lib.rs","old_string":"a","new_string":"b"}},
                    {"type":"tool_use","id":"t2","name":"WebFetch","input":{"url":"https://example.com"}}
                ]}
            }),
            &mut state,
        );
        assert_eq!(started[0].kind, "file_change.started");
        assert_eq!(started[1].kind, "tool.started");
        assert_eq!(started[1].title.as_deref(), Some("WebFetch"));
        let completed = normalize_claude_message_with_state(
            &json!({
                "type":"user",
                "message":{"content":[
                    {"type":"tool_result","tool_use_id":"t1","content":"ok"},
                    {"type":"tool_result","tool_use_id":"t2","content":"page"}
                ]}
            }),
            &mut state,
        );
        assert_eq!(completed[0].kind, "file_change.completed");
        assert_eq!(completed[1].kind, "tool.completed");
    }

    #[test]
    fn claude_stream_start_and_snapshot_keep_one_tool_item() {
        let mut state = ClaudeStreamState::default();
        // Streaming start arrives before the input has been assembled.
        let streamed = normalize_claude_message_with_state(
            &json!({
                "type":"stream_event",
                "event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":"Bash"}}
            }),
            &mut state,
        );
        assert_eq!(streamed[0].kind, "command.started");
        assert_eq!(streamed[0].title.as_deref(), Some("Bash"));
        // The snapshot repeats the block with full input under the same id.
        let snapshot = normalize_claude_message_with_state(
            &json!({
                "type":"assistant",
                "message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}
            }),
            &mut state,
        );
        assert_eq!(snapshot[0].kind, "command.started");
        assert_eq!(snapshot[0].item_id, streamed[0].item_id);
        assert_eq!(snapshot[0].title.as_deref(), Some("cargo test"));
    }

    #[test]
    fn claude_tool_phase_and_duration_start_at_the_snapshot() {
        let mut state = ClaudeStreamState::default();
        let streamed = normalize_claude_message_with_state(
            &json!({
                "type":"stream_event",
                "event":{"type":"content_block_start","content_block":{"type":"tool_use","id":"t1","name":"Bash"}}
            }),
            &mut state,
        );
        // The model is still writing the arguments: nothing runs yet.
        assert_eq!(streamed[0].data["phase"], "preparing");
        assert_eq!(streamed[0].status.as_deref(), Some("inProgress"));
        std::thread::sleep(std::time::Duration::from_millis(40));
        let snapshot = normalize_claude_message_with_state(
            &json!({
                "type":"assistant",
                "message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"gh pr create"}}]}
            }),
            &mut state,
        );
        assert_eq!(snapshot[0].data["phase"], "running");
        let completed = normalize_claude_message_with_state(
            &json!({
                "type":"user",
                "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}
            }),
            &mut state,
        );
        // The card's duration is run time, not the time the model spent
        // composing the call.
        let duration = completed[0].data["durationMs"].as_u64().unwrap();
        assert!(duration < 40, "duration {duration}ms includes argument streaming");
    }

    #[test]
    fn claude_tool_result_without_started_falls_back_to_tool_completed() {
        let completed = normalize_claude_message(&json!({
            "type":"user",
            "message":{"content":[{"type":"tool_result","tool_use_id":"unseen","content":"late"}]}
        }));
        assert_eq!(completed[0].kind, "tool.completed");
        assert!(completed[0].data.get("durationMs").is_none());
    }

    #[test]
    fn claude_edit_synthesizes_unified_diff() {
        let started = normalize_claude_message(&json!({
            "type":"assistant",
            "message":{"id":"m1","content":[{
                "type":"tool_use","id":"t1","name":"Edit",
                "input":{"file_path":"src/lib.rs","old_string":"fn a() {}","new_string":"fn a() { 1 }"}
            }]}
        }));
        let patch = started[0].data["patch"].as_str().expect("synthesized patch");
        assert!(patch.contains("--- a/src/lib.rs"));
        assert!(patch.contains("+++ b/src/lib.rs"));
        assert!(patch.contains("-fn a() {}"));
        assert!(patch.contains("+fn a() { 1 }"));
    }

    #[test]
    fn claude_write_synthesizes_full_file_addition() {
        let started = normalize_claude_message(&json!({
            "type":"assistant",
            "message":{"id":"m1","content":[{
                "type":"tool_use","id":"t1","name":"Write",
                "input":{"file_path":"src/new.rs","content":"pub fn n() {}"}
            }]}
        }));
        let patch = started[0].data["patch"].as_str().expect("synthesized patch");
        assert!(patch.contains("--- /dev/null"));
        assert!(patch.contains("+++ b/src/new.rs"));
        assert!(patch.contains("+pub fn n() {}"));
        assert!(!patch.contains("-pub fn n() {}"));
    }

    #[test]
    fn claude_multiedit_synthesizes_one_hunk_per_edit() {
        let started = normalize_claude_message(&json!({
            "type":"assistant",
            "message":{"id":"m1","content":[{
                "type":"tool_use","id":"t1","name":"MultiEdit",
                "input":{"file_path":"src/lib.rs","edits":[
                    {"old_string":"fn a() {}","new_string":"fn a() { 1 }"},
                    {"old_string":"fn b() {}","new_string":"fn b() { 2 }"}
                ]}
            }]}
        }));
        assert_eq!(started[0].kind, "file_change.started");
        let patch = started[0].data["patch"].as_str().expect("synthesized patch");
        assert!(patch.contains("--- a/src/lib.rs"));
        assert!(patch.contains("+++ b/src/lib.rs"));
        assert!(patch.contains("-fn a() {}"));
        assert!(patch.contains("+fn a() { 1 }"));
        assert!(patch.contains("-fn b() {}"));
        assert!(patch.contains("+fn b() { 2 }"));
    }

    #[test]
    fn claude_notebookedit_synthesizes_diff_from_notebook_path() {
        let started = normalize_claude_message(&json!({
            "type":"assistant",
            "message":{"id":"m1","content":[{
                "type":"tool_use","id":"t1","name":"NotebookEdit",
                "input":{"notebook_path":"analysis.ipynb","new_source":"print(1)"}
            }]}
        }));
        assert_eq!(started[0].kind, "file_change.started");
        let patch = started[0].data["patch"].as_str().expect("synthesized patch");
        assert!(patch.contains("--- a/analysis.ipynb"));
        assert!(patch.contains("+++ b/analysis.ipynb"));
        assert!(patch.contains("+print(1)"));
        assert!(!patch.contains("-print(1)"));
    }

    #[test]
    fn claude_and_codex_approval_titles_share_wording() {
        let claude = normalize_claude_control_request(&json!({
            "type":"control_request",
            "request":{"subtype":"can_use_tool","tool_name":"Bash","tool_input":{"command":"ls"}}
        }))
        .unwrap();
        assert_eq!(claude.title.as_deref(), Some("Run this command?"));
        let edit = normalize_claude_control_request(&json!({
            "type":"control_request",
            "request":{"subtype":"permission","tool_name":"Edit"}
        }))
        .unwrap();
        assert_eq!(edit.title.as_deref(), Some("Edit these files?"));
        let mcp = normalize_claude_control_request(&json!({
            "type":"control_request",
            "request":{"subtype":"permission","tool_name":"mcp__github__create_issue"}
        }))
        .unwrap();
        assert_eq!(mcp.title.as_deref(), Some("Use github · create_issue?"));
        let file = normalize_codex_request(&json!({
            "id":1,
            "method":"item/fileChange/requestApproval",
            "params":{"itemId":"f1"}
        }))
        .unwrap();
        assert_eq!(file.title.as_deref(), Some("Edit these files?"));
    }

    #[test]
    fn normalizes_claude_result_as_turn_completed() {
        let events = normalize_claude_message(&json!({
            "type":"result","subtype":"success","is_error":false,"result":"done","usage":{"input_tokens":1}
        }));
        assert!(events.iter().any(|event| event.kind == "turn.completed"));
        assert!(events.iter().any(|event| event.kind == "usage.updated"));
    }

    #[test]
    fn claude_permission_denials_become_durable_named_events() {
        let events = normalize_claude_message(&json!({
            "type":"result",
            "subtype":"success",
            "is_error":false,
            "result":"done",
            "permission_denials":[{
                "tool_name":"mcp__notion__create_page",
                "tool_use_id":"tool-1",
                "tool_input":{"title":"blocked"}
            }]
        }));
        let denied = events
            .iter()
            .find(|event| event.kind == "permission.denied")
            .unwrap();
        assert_eq!(denied.status.as_deref(), Some("denied"));
        assert_eq!(denied.title.as_deref(), Some("Bridge denied mcp__notion__create_page"));
        assert_eq!(denied.data["tool_use_id"], "tool-1");
        assert!(denied.data.get("tool_input").is_none());
        assert!(!denied.data.to_string().contains("blocked"));

        let failed = normalize_claude_message(&json!({
            "type":"result",
            "subtype":"error_during_execution",
            "is_error":true,
            "result":"denied",
            "permission_denials":[{
                "tool_name":"Write",
                "tool_use_id":"tool-2",
                "tool_input":{"secret":"must-not-persist"}
            }]
        }));
        let error = failed.iter().find(|event| event.kind == "error").unwrap();
        assert!(!error.data.to_string().contains("must-not-persist"));
        assert!(error.data["permission_denials"][0].get("tool_input").is_none());
    }

    #[test]
    fn claude_capability_discovery_diagnostics_are_not_dropped_as_hook_noise() {
        let events = normalize_claude_message(&json!({
            "type":"system",
            "subtype":"capability_discovery",
            "diagnostics":[{"stage":"plugins","status":"malformed","message":"invalid JSON"}]
        }));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "capability.discovery");
        assert_eq!(events[0].status.as_deref(), Some("degraded"));
        assert_eq!(events[0].data["diagnostics"][0]["stage"], "plugins");
    }

    #[test]
    fn codex_token_usage_reports_the_last_request_not_the_running_total() {
        // Shape and field names come from the app-server's own generated
        // schema (`codex app-server generate-json-schema`). `total` and `last`
        // carry deliberately different numbers: a regression to the cumulative
        // counter fails here rather than silently inflating the ledger.
        let events = normalize_codex_message(&json!({
            "method":"thread/tokenUsage/updated",
            "params":{
                "threadId":"thread-1",
                "turnId":"turn-1",
                "tokenUsage":{
                    "total":{"totalTokens":900,"inputTokens":800,"cachedInputTokens":700,"cacheWriteInputTokens":50,"outputTokens":100,"reasoningOutputTokens":40},
                    "last":{"totalTokens":90,"inputTokens":80,"cachedInputTokens":70,"cacheWriteInputTokens":5,"outputTokens":10,"reasoningOutputTokens":4},
                    "modelContextWindow":272000
                }
            }
        }));
        assert_eq!(events.len(), 1);
        let usage = &events[0].data["usage"];
        assert_eq!(events[0].kind, "usage.updated");
        assert_eq!(usage["input_tokens"], 80);
        assert_eq!(usage["output_tokens"], 10);
        assert_eq!(usage["cache_read_tokens"], 70);
        assert_eq!(usage["cache_write_tokens"], 5);
        assert_eq!(usage["reasoning_tokens"], 4);
        assert_eq!(usage["total_tokens"], 90);

        // The running total and the context window stay reachable beside the
        // normalized per-request slice.
        assert_eq!(events[0].data["tokenUsage"]["total"]["inputTokens"], 800);
        assert_eq!(events[0].data["tokenUsage"]["modelContextWindow"], 272_000);
        assert_eq!(events[0].data["turnId"], "turn-1");
    }

    fn claude_result(cost: f64, model_usage: Value) -> Value {
        json!({
            "type":"result","subtype":"success","session_id":"s",
            "usage":{"input_tokens":3,"cache_read_input_tokens":100,"cache_creation_input_tokens":10,"output_tokens":20},
            "total_cost_usd": cost,
            "modelUsage": model_usage
        })
    }

    fn usage_data(events: &[NormalizedEvent]) -> &Value {
        &events.iter().find(|event| event.kind == "usage.updated").expect("a usage event").data
    }

    #[test]
    fn claude_running_totals_become_per_turn_figures_across_results() {
        let mut state = ClaudeStreamState::default();
        let first = normalize_claude_message_with_state(
            &claude_result(0.50, json!({"claude-opus-5":{"inputTokens":10,"outputTokens":20,"cacheReadInputTokens":1000,"cacheCreationInputTokens":50,"costUSD":0.50,"contextWindow":200000}})),
            &mut state,
        );
        // The first result of a process is its own delta.
        assert_eq!(usage_data(&first)["totalCostUsd"], json!(0.50));
        assert_eq!(usage_data(&first)["modelUsage"]["claude-opus-5"]["cacheReadInputTokens"], json!(1000));

        let second = normalize_claude_message_with_state(
            &claude_result(0.80, json!({"claude-opus-5":{"inputTokens":14,"outputTokens":26,"cacheReadInputTokens":1600,"cacheCreationInputTokens":50,"costUSD":0.80,"contextWindow":200000}})),
            &mut state,
        );
        let data = usage_data(&second);
        assert!((data["totalCostUsd"].as_f64().unwrap() - 0.30).abs() < 1e-9);
        let model = &data["modelUsage"]["claude-opus-5"];
        assert_eq!(model["inputTokens"], json!(4));
        assert_eq!(model["outputTokens"], json!(6));
        assert_eq!(model["cacheReadInputTokens"], json!(600));
        assert_eq!(model["cacheCreationInputTokens"], json!(0));
        assert!((model["costUSD"].as_f64().unwrap() - 0.30).abs() < 1e-9);
        // Non-counters pass through untouched.
        assert_eq!(model["contextWindow"], json!(200000));
        // The per-turn `usage` block is not differenced.
        assert_eq!(data["usage"]["cache_read_input_tokens"], json!(100));
    }

    #[test]
    fn claude_totals_that_go_backwards_start_a_fresh_run() {
        let mut state = ClaudeStreamState::default();
        normalize_claude_message_with_state(&claude_result(2.0, json!({"m":{"inputTokens":100,"costUSD":2.0}})), &mut state);
        // A `/clear` or a restarted process reports a smaller running total:
        // that frame is a whole turn, not a negative one.
        let reset = normalize_claude_message_with_state(&claude_result(0.25, json!({"m":{"inputTokens":7,"costUSD":0.25}})), &mut state);
        assert_eq!(usage_data(&reset)["totalCostUsd"], json!(0.25));
        assert_eq!(usage_data(&reset)["modelUsage"]["m"]["inputTokens"], json!(7));
        // A new model appears with its own full figures.
        let added = normalize_claude_message_with_state(
            &claude_result(0.40, json!({"m":{"inputTokens":9,"costUSD":0.30},"haiku":{"inputTokens":50,"costUSD":0.10}})),
            &mut state,
        );
        assert_eq!(usage_data(&added)["modelUsage"]["m"]["inputTokens"], json!(2));
        assert_eq!(usage_data(&added)["modelUsage"]["haiku"]["inputTokens"], json!(50));
        // `system/init` marks a new process, whose totals begin at zero again.
        normalize_claude_message_with_state(&json!({"type":"system","subtype":"init","session_id":"s","tools":[]}), &mut state);
        let fresh = normalize_claude_message_with_state(&claude_result(0.05, json!({"m":{"inputTokens":3,"costUSD":0.05}})), &mut state);
        assert_eq!(usage_data(&fresh)["totalCostUsd"], json!(0.05));
        assert_eq!(usage_data(&fresh)["modelUsage"]["m"]["inputTokens"], json!(3));
    }

    #[test]
    fn codex_token_usage_without_a_last_breakdown_invents_nothing() {
        let events = normalize_codex_message(&json!({
            "method":"thread/tokenUsage/updated",
            "params":{"threadId":"thread-1","turnId":"turn-1","tokenUsage":{"modelContextWindow":272000}}
        }));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "usage.updated");
        // Explicitly empty rather than absent: an absent `usage` sends
        // `UsageReport::from_normalized` recursing into the raw frame, where it
        // would find the cumulative `tokenUsage.total` this case must reject.
        assert_eq!(events[0].data["usage"], json!({}));
    }

    fn opencode_step_finish(message_id: &str, tokens: Value, cost: Value) -> Value {
        json!({
            "type":"message.part.updated",
            "properties":{"part":{"id":format!("prt_{message_id}"),"messageID":message_id,"sessionID":"ses_1","type":"step-finish","tokens":tokens,"cost":cost}}
        })
    }

    #[test]
    fn opencode_usage_reports_cache_writes_alongside_reads() {
        let mut state = OpenCodeStreamState::default();
        let usage = normalize_opencode_message_with_state(
            &opencode_step_finish("msg_1", json!({"input":9,"output":2,"cache":{"read":6,"write":3}}), Value::Null),
            &mut state,
        );
        assert_eq!(usage[0].data["usage"]["cache_read_tokens"], 6);
        assert_eq!(usage[0].data["usage"]["cache_write_tokens"], 3);

        let no_write = normalize_opencode_message_with_state(
            &opencode_step_finish("msg_2", json!({"input":9,"output":2,"cache":{"read":6}}), Value::Null),
            &mut state,
        );
        assert!(no_write[0].data["usage"]["cache_write_tokens"].is_null());
    }

    #[test]
    fn opencode_message_updated_names_the_model_and_writes_no_usage_of_its_own() {
        let mut state = OpenCodeStreamState::default();
        // The message frame's `tokens` are the last step's and its `cost` the
        // running sum, so it must not become a row beside the step parts.
        let message = normalize_opencode_message_with_state(
            &json!({
                "type":"message.updated",
                "properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant","tokens":{"input":4,"output":2},"cost":0.03,"modelID":"claude-opus-4-6","providerID":"anthropic"}}
            }),
            &mut state,
        );
        assert!(message.iter().all(|event| event.kind != "usage.updated"));

        let step = normalize_opencode_message_with_state(
            &opencode_step_finish("msg_1", json!({"input":4,"output":2,"reasoning":1,"cache":{"read":3,"write":0}}), json!(0.01)),
            &mut state,
        );
        assert_eq!(step[0].kind, "usage.updated");
        assert_eq!(step[0].data["model"], "claude-opus-4-6");
        assert_eq!(step[0].data["provider"], "anthropic");
        assert_eq!(step[0].data["cost"], 0.01);
        assert_eq!(step[0].data["usage"]["reasoning_tokens"], 1);
    }

    #[test]
    fn normalizes_opencode_streaming_messages_and_usage() {
        let mut state = OpenCodeStreamState::default();
        normalize_opencode_message_with_state(
            &json!({
                "type":"message.updated",
                "properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant","modelID":"model","providerID":"provider"}}
            }),
            &mut state,
        );
        let usage = normalize_opencode_message_with_state(
            &opencode_step_finish("msg_1", json!({"input":4,"output":2,"reasoning":1,"cache":{"read":3,"write":0}}), json!(0.01)),
            &mut state,
        );
        assert_eq!(usage[0].kind, "usage.updated");
        assert_eq!(usage[0].data["usage"]["input_tokens"], 4);

        let delta = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.delta",
                "properties":{"sessionID":"ses_1","messageID":"msg_1","partID":"prt_1","field":"text","delta":"hello"}
            }),
            &mut state,
        );
        assert_eq!(delta[0].kind, "message.delta");
        assert_eq!(delta[0].item_id.as_deref(), Some("prt_1"));
        assert_eq!(delta[0].text.as_deref(), Some("hello"));
    }

    #[test]
    fn opencode_captured_wire_matches_frontend_replay() {
        let fixture: Value = serde_json::from_str(include_str!("../../../src/transcript/fixtures/opencode-wire.json")).unwrap();
        let mut state = OpenCodeStreamState::default();
        let actual: Vec<_> = fixture["wire"].as_array().unwrap().iter()
            .flat_map(|raw| normalize_opencode_message_with_state(raw, &mut state))
            .filter(|event| matches!(event.kind.as_str(), "turn.started" | "turn.completed" | "reasoning.started" | "reasoning.delta" | "reasoning.completed" | "message.delta" | "message.completed"))
            .map(|event| json!({"kind":event.kind,"itemId":event.item_id,"status":event.status,"role":event.role,"text":event.text})).collect();
        assert_eq!(json!(actual), fixture["expected"]);
    }

    #[test]
    fn opencode_status_and_model_steps_do_not_restart_a_turn() {
        let mut state = OpenCodeStreamState::default();
        let busy = json!({"id":"event-not-turn", "type":"session.status", "properties":{"sessionID":"ses_1","status":{"type":"busy"}}});
        let first = normalize_opencode_message_with_state(&busy, &mut state);
        assert_eq!(first[0].kind, "turn.started");
        assert!(first[0].data.get("turnId").is_none());
        for status in ["busy", "retry", "busy"] {
            assert!(normalize_opencode_message_with_state(&json!({"type":"session.status", "properties":{"sessionID":"ses_1","status":{"type":status}}}), &mut state).is_empty());
            assert!(normalize_opencode_message_with_state(&json!({"type":"message.part.updated", "properties":{"sessionID":"ses_1","part":{"id":"step-not-turn","messageID":"msg_1","type":"step-start"}}}), &mut state).is_empty());
        }
        let idle = json!({"type":"session.status", "properties":{"sessionID":"ses_1","status":{"type":"idle"}}});
        assert_eq!(normalize_opencode_message_with_state(&idle, &mut state)[0].kind, "turn.completed");
        assert!(normalize_opencode_message_with_state(&json!({"type":"session.idle","properties":{"sessionID":"ses_1"}}), &mut state).is_empty());
        assert!(normalize_opencode_message_with_state(&idle, &mut state).is_empty());
        assert_eq!(normalize_opencode_message_with_state(&busy, &mut state)[0].kind, "turn.started");
        assert_eq!(normalize_opencode_message_with_state(&json!({"type":"session.error","properties":{"sessionID":"ses_1","error":{"message":"aborted"}}}), &mut state)[0].kind, "error");
        // An idle following an error must not finalize the failed turn twice.
        assert!(normalize_opencode_message_with_state(&idle, &mut state).is_empty());
        assert_eq!(normalize_opencode_message_with_state(&busy, &mut state)[0].kind, "turn.started");
    }

    #[test]
    fn opencode_reasoning_snapshots_replace_and_do_not_reopen() {
        let mut state = OpenCodeStreamState::default();
        let snapshot = |text: &str, end: Value| json!({"type":"message.part.updated", "properties":{
            "sessionID":"ses_1", "part":{"id":"r", "messageID":"m", "type":"reasoning", "text":text, "time":{"start":1,"end":end}}
        }});
        let start = normalize_opencode_message_with_state(&snapshot("First", Value::Null), &mut state);
        assert_eq!(start[0].kind, "reasoning.started");
        let middle = normalize_opencode_message_with_state(&snapshot("First second", Value::Null), &mut state);
        assert_eq!(middle[0].kind, "reasoning.started");
        assert_eq!(middle[0].text.as_deref(), Some("First second"));
        let end = normalize_opencode_message_with_state(&snapshot("First second", json!(2)), &mut state);
        assert_eq!(end[0].kind, "reasoning.completed");
        assert!(normalize_opencode_message_with_state(&snapshot("First", Value::Null), &mut state).is_empty());
        // A repeated terminal is an idempotent full replacement, never a delta.
        assert_eq!(normalize_opencode_message_with_state(&snapshot("First second", json!(2)), &mut state)[0], end[0]);
    }

    #[test]
    fn opencode_part_routing_is_scoped_to_message_and_rejects_non_prose() {
        let mut state = OpenCodeStreamState::default();
        for (id, role) in [("a", "assistant"), ("u", "user")] {
            normalize_opencode_message_with_state(&json!({"type":"message.updated","properties":{"info":{"id":id,"role":role}}}), &mut state);
        }
        for (message, id, kind) in [("a", "r", "reasoning"), ("a", "t", "text"), ("a", "tool", "tool"), ("u", "ur", "reasoning")] {
            normalize_opencode_message_with_state(&json!({"type":"message.part.updated","properties":{"part":{"messageID":message,"id":id,"type":kind,"text":""}}}), &mut state);
        }
        let delta = |message: &str, id: &str, field: &str| json!({"type":"message.part.delta","properties":{"messageID":message,"partID":id,"field":field,"delta":"chunk"}});
        assert_eq!(normalize_opencode_message_with_state(&delta("a", "r", "text"), &mut state)[0].kind, "reasoning.delta");
        assert_eq!(normalize_opencode_message_with_state(&delta("a", "t", "text"), &mut state)[0].kind, "message.delta");
        assert!(normalize_opencode_message_with_state(&delta("a", "tool", "text"), &mut state).is_empty());
        assert!(normalize_opencode_message_with_state(&delta("a", "t", "metadata"), &mut state).is_empty());
        assert!(normalize_opencode_message_with_state(&delta("u", "ur", "text"), &mut state).is_empty());
        assert!(normalize_opencode_message_with_state(&delta("unknown", "r", "text"), &mut state).is_empty());
    }

    /// OpenCode's processor uses field="text" for BOTH reasoning and prose.
    /// The part-start snapshot is the discriminator, not the delta field.
    #[test]
    fn opencode_text_field_preserves_reasoning_part_identity() {
        let mut state = OpenCodeStreamState::default();
        normalize_opencode_message_with_state(&json!({"type":"message.updated", "properties":{
            "sessionID":"ses_1", "info":{"id":"msg_1", "role":"assistant"}
        }}), &mut state);
        normalize_opencode_message_with_state(&json!({"type":"message.part.updated", "properties":{
            "sessionID":"ses_1", "part":{"id":"prt_r", "messageID":"msg_1", "type":"reasoning", "text":"", "time":{"start":1}}
        }}), &mut state);
        let delta = normalize_opencode_message_with_state(&json!({"type":"message.part.delta", "properties":{
            "sessionID":"ses_1", "messageID":"msg_1", "partID":"prt_r", "field":"text", "delta":"Checking the facts."
        }}), &mut state);
        assert_eq!(delta[0].kind, "reasoning.delta");
        assert_eq!(delta[0].item_id.as_deref(), Some("prt_r"));
        assert_eq!(delta[0].status.as_deref(), Some("streaming"));
        let completed = normalize_opencode_message_with_state(&json!({"type":"message.part.updated", "properties":{
            "sessionID":"ses_1", "part":{"id":"prt_r", "messageID":"msg_1", "type":"reasoning", "text":"Checking the facts.", "time":{"start":1,"end":2}}
        }}), &mut state);
        assert_eq!(completed[0].kind, "reasoning.completed");
        assert_eq!(completed[0].item_id, delta[0].item_id);
        let answer = normalize_opencode_message_with_state(&json!({"type":"message.part.delta", "properties":{
            "sessionID":"ses_1", "messageID":"msg_1", "partID":"prt_text", "field":"text", "delta":"Here is the answer."
        }}), &mut state);
        assert_eq!(answer[0].kind, "message.delta");
        // A delayed delta cannot reopen a completed thought.
        assert!(normalize_opencode_message_with_state(&json!({"type":"message.part.delta", "properties":{
            "sessionID":"ses_1", "messageID":"msg_1", "partID":"prt_r", "field":"text", "delta":"late"
        }}), &mut state).is_empty());
    }

    #[test]
    fn normalizes_opencode_reasoning_deltas_before_message_updated() {
        let mut state = OpenCodeStreamState::default();
        let delta = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.delta",
                "properties":{"sessionID":"ses_1","messageID":"msg_early","partID":"prt_r1","field":"reasoning","delta":"Reasoning thought..."}
            }),
            &mut state,
        );
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0].kind, "reasoning.delta");
        assert_eq!(delta[0].role.as_deref(), Some("assistant"));
        assert_eq!(delta[0].text.as_deref(), Some("Reasoning thought..."));

        // Non-reasoning delta with unknown role is NOT assumed to be assistant and does not cache
        let text_delta_unknown = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.delta",
                "properties":{"sessionID":"ses_1","messageID":"msg_user_candidate","partID":"prt_t1","field":"text","delta":"user message"}
            }),
            &mut state,
        );
        assert!(text_delta_unknown.is_empty());
    }

    #[test]
    fn normalizes_opencode_reasoning_streaming_midstream_snapshot() {
        let mut state = OpenCodeStreamState::default();
        // Mid-stream reasoning part with only time.start and no end/status
        let part_streaming = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.updated",
                "properties":{
                    "sessionID":"ses_1",
                    "part":{
                        "id":"prt_r0",
                        "messageID":"msg_early",
                        "type":"reasoning",
                        "text":"Ongoing reasoning...",
                        "time":{"start":100}
                    }
                }
            }),
            &mut state,
        );
        assert_eq!(part_streaming.len(), 1);
        assert_eq!(part_streaming[0].kind, "reasoning.started");
        assert_eq!(part_streaming[0].status.as_deref(), Some("inProgress"));
        assert_eq!(part_streaming[0].text.as_deref(), Some("Ongoing reasoning..."));
    }

    #[test]
    fn normalizes_opencode_reasoning_field_variants_and_completion() {
        let mut state = OpenCodeStreamState::default();
        let part_completed = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.updated",
                "properties":{
                    "sessionID":"ses_1",
                    "part":{
                        "id":"prt_r2",
                        "messageID":"msg_early",
                        "type":"reasoning",
                        "reasoning":"Concluded reasoning",
                        "time":{"start":100,"end":200}
                    }
                }
            }),
            &mut state,
        );
        assert_eq!(part_completed.len(), 1);
        assert_eq!(part_completed[0].kind, "reasoning.completed");
        assert_eq!(part_completed[0].text.as_deref(), Some("Concluded reasoning"));
        assert_eq!(part_completed[0].status.as_deref(), Some("completed"));

        let status_completed = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.updated",
                "properties":{
                    "sessionID":"ses_1",
                    "part":{
                        "id":"prt_r3",
                        "messageID":"msg_early",
                        "type":"reasoning",
                        "text":"Finished thinking",
                        "state":{"status":"completed"}
                    }
                }
            }),
            &mut state,
        );
        assert_eq!(status_completed.len(), 1);
        assert_eq!(status_completed[0].kind, "reasoning.completed");
        assert_eq!(status_completed[0].text.as_deref(), Some("Finished thinking"));
    }

    #[test]
    fn normalizes_opencode_tools_permissions_and_turn_state() {
        let mut state = OpenCodeStreamState::default();
        let _ = normalize_opencode_message_with_state(
            &json!({
                "type":"message.updated",
                "properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant"}}
            }),
            &mut state,
        );
        let tool = normalize_opencode_message_with_state(
            &json!({
                "type":"message.part.updated",
                "properties":{"sessionID":"ses_1","part":{"id":"prt_2","sessionID":"ses_1","messageID":"msg_1","type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"pwd"},"output":"/tmp","title":"Run pwd"}}}
            }),
            &mut state,
        );
        assert_eq!(tool[0].kind, "command.completed");
        assert_eq!(tool[0].text.as_deref(), Some("/tmp"));

        let permission = normalize_opencode_message_with_state(
            &json!({
                "type":"permission.v2.asked",
                "properties":{"id":"per_1","sessionID":"ses_1","action":"bash","resources":["git status"],"source":{"callID":"call_1"}}
            }),
            &mut state,
        );
        assert_eq!(permission[0].kind, "permission.requested");
        assert_eq!(permission[0].data["interactionKind"], "permission");
        assert_eq!(permission[0].data["requestId"], "per_1");

        let busy = normalize_opencode_message_with_state(
            &json!({
                "id":"evt_1","type":"session.status","properties":{"sessionID":"ses_1","status":{"type":"busy"}}
            }),
            &mut state,
        );
        assert_eq!(busy[0].kind, "turn.started");
        let idle = normalize_opencode_message_with_state(
            &json!({
                "type":"session.idle","properties":{"sessionID":"ses_1"}
            }),
            &mut state,
        );
        assert_eq!(idle[0].kind, "turn.completed");
    }

    /// OpenCode's `question` tool is a different channel from `permission`:
    /// it must never be treated as an approval an auto-approve policy can
    /// grant, or a bare accept/decline could answer a question with no text.
    #[test]
    fn normalizes_opencode_question_asked_without_leaking_it_as_an_approval() {
        let mut state = OpenCodeStreamState::default();
        let question = normalize_opencode_message_with_state(
            &json!({
                "id":"evt_1",
                "type":"question.asked",
                "properties":{
                    "id":"req_1",
                    "sessionID":"ses_1",
                    "questions":[{
                        "question":"Your workspace is stale against origin/main. How do you want to proceed?",
                        "header":"Stale workspace",
                        "options":[{"label":"Rebase","description":"Rebase onto origin/main"}],
                    }],
                    "tool":{"messageID":"msg_1","callID":"call_1"},
                },
            }),
            &mut state,
        );
        assert_eq!(question.len(), 1);
        let event = &question[0];
        assert_eq!(event.kind, "question.requested");
        assert_eq!(event.data["interactionKind"], "question");
        assert_eq!(event.item_id.as_deref(), Some("call_1"));
        assert_eq!(event.title.as_deref(), Some("Stale workspace"));
        assert_eq!(
            event.text.as_deref(),
            Some("Your workspace is stale against origin/main. How do you want to proceed?")
        );
        assert_eq!(event.status.as_deref(), Some("pending"));
        assert_eq!(event.data["requestId"], "req_1");
        assert_eq!(
            event.data["requestMethod"],
            OPENCODE_QUESTION_REQUEST_METHOD
        );
        assert_eq!(event.data["questions"][0]["header"], "Stale workspace");
        assert!(
            !event.data["requestMethod"]
                .as_str()
                .unwrap()
                .ends_with("requestApproval"),
            "a question must never satisfy the bypass-policy auto-grant check"
        );
    }

    /// A question can be settled by something other than this Bridge process
    /// answering it — a decline, or a different client on the same OpenCode
    /// session. Whatever settled it, `question.replied`/`question.rejected`
    /// must carry the provider's `requestID` so `live_turn.rs` can find and
    /// resolve the matching pending row instead of leaving it stuck open.
    #[test]
    fn normalizes_question_replied_and_rejected_with_the_settling_requestid() {
        let mut state = OpenCodeStreamState::default();
        let replied = normalize_opencode_message_with_state(
            &json!({
                "type": "question.replied",
                "properties": {"sessionID": "ses_1", "requestID": "req_1", "answers": [["Rebase"]]},
            }),
            &mut state,
        );
        assert_eq!(replied.len(), 1);
        assert_eq!(replied[0].kind, "question.settled");
        assert_eq!(replied[0].status.as_deref(), Some("answered"));
        assert_eq!(replied[0].data["requestId"], "req_1");

        let rejected = normalize_opencode_message_with_state(
            &json!({
                "type": "question.rejected",
                "properties": {"sessionID": "ses_1", "requestID": "req_2"},
            }),
            &mut state,
        );
        assert_eq!(rejected[0].kind, "question.settled");
        assert_eq!(rejected[0].status.as_deref(), Some("rejected"));
        assert_eq!(rejected[0].data["requestId"], "req_2");
    }

    fn fixture_case<'a>(doc: &'a Value, id: &str) -> &'a Value {
        doc["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == id)
            .unwrap_or_else(|| panic!("missing fixture case {id}"))
            .get("wire")
            .unwrap()
    }

    #[test]
    fn normalizes_codex_file_change_fixture_into_canonical_fields() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../testing/fixtures/file-change/codex-fileChange.json"
        ))
        .unwrap();

        let update = normalize_codex_message(fixture_case(&fixture, "codex-update"));
        assert_eq!(update[0].kind, "file_change.completed");
        assert_eq!(update[0].data["path"], "src/lib.rs");
        assert_eq!(update[0].title.as_deref(), Some("lib.rs"));
        assert_eq!(
            update[0].data["patch"].as_str().unwrap(),
            "@@ -1,1 +1,1 @@\n-fn a() {}\n+fn a() { 1 }\n"
        );
        assert_eq!(update[0].data["additions"], 1);
        assert_eq!(update[0].data["deletions"], 1);
        assert!(update[0].data.get("paths").is_none());

        let added = normalize_codex_message(fixture_case(&fixture, "codex-add"));
        assert_eq!(added[0].kind, "file_change.started");
        assert_eq!(added[0].data["path"], "APPROVAL_DEMO.txt");
        let add_patch = added[0].data["patch"].as_str().unwrap();
        assert!(add_patch.contains("--- /dev/null"));
        assert!(add_patch.contains("+++ b/APPROVAL_DEMO.txt"));
        assert!(add_patch.contains("+Hello from Codex!"));
        assert_eq!(added[0].data["additions"], 1);
        assert_eq!(added[0].data["deletions"], 0);

        let multi = normalize_codex_message(fixture_case(&fixture, "codex-multifile"));
        assert_eq!(multi[0].kind, "file_change.completed");
        assert_eq!(multi[0].data["path"], "src/lib.rs");
        assert_eq!(
            multi[0].data["paths"],
            json!(["src/lib.rs", "src/main.rs"])
        );
        let multi_patch = multi[0].data["patch"].as_str().unwrap();
        assert!(multi_patch.contains("+fn a() { 1 }"));
        assert!(multi_patch.contains("+fn main() { a() }"));
        assert_eq!(multi[0].data["additions"], 2);
        assert_eq!(multi[0].data["deletions"], 2);
    }

    #[test]
    fn normalizes_opencode_file_tool_fixtures_into_canonical_fields() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../testing/fixtures/file-change/opencode-file-tools.json"
        ))
        .unwrap();
        let mut state = OpenCodeStreamState::default();
        let _ = normalize_opencode_message_with_state(&fixture["setup"], &mut state);

        let edit = normalize_opencode_message_with_state(
            fixture_case(&fixture, "opencode-edit"),
            &mut state,
        );
        assert_eq!(edit[0].kind, "file_change.completed");
        assert_eq!(edit[0].data["path"], "src/lib.rs");
        assert!(edit[0].data["patch"].as_str().unwrap().contains("+fn a() { 1 }"));
        assert_eq!(edit[0].data["additions"], 1);
        assert_eq!(edit[0].data["deletions"], 1);

        let write = normalize_opencode_message_with_state(
            fixture_case(&fixture, "opencode-write"),
            &mut state,
        );
        assert_eq!(write[0].kind, "file_change.completed");
        assert_eq!(write[0].data["path"], "src/new.rs");
        let write_patch = write[0].data["patch"].as_str().unwrap();
        assert!(write_patch.contains("--- /dev/null"));
        assert!(write_patch.contains("+++ b/src/new.rs"));
        assert!(write_patch.contains("+pub fn n() {}"));
        assert_eq!(write[0].data["additions"], 1);
        assert_eq!(write[0].data["deletions"], 0);

        let patch = normalize_opencode_message_with_state(
            fixture_case(&fixture, "opencode-apply-patch"),
            &mut state,
        );
        assert_eq!(patch[0].kind, "file_change.completed");
        assert_eq!(patch[0].data["path"], "src/lib.rs");
        assert_eq!(
            patch[0].data["paths"],
            json!(["src/lib.rs", "src/main.rs"])
        );
        assert!(patch[0].data["patch"].as_str().unwrap().contains("+fn main() { a() }"));
        assert_eq!(patch[0].data["additions"], 2);
        assert_eq!(patch[0].data["deletions"], 2);
    }
}
