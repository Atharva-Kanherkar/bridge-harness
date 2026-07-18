use serde_json::{json, Value};
use std::collections::HashMap;

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
                    let mut event = with_data("turn.started", &properties, properties.clone());
                    event.status = Some(
                        if status == "retry" {
                            "retrying"
                        } else {
                            "working"
                        }
                        .into(),
                    );
                    event.data["turnId"] = message.get("id").cloned().unwrap_or(Value::Null);
                    vec![event]
                }
                "idle" => {
                    let mut event = with_data("turn.completed", &properties, properties.clone());
                    event.status = Some("completed".into());
                    vec![event]
                }
                _ => vec![],
            }
        }
        "session.idle" => {
            let mut event = with_data("turn.completed", &properties, properties.clone());
            event.status = Some("completed".into());
            vec![event]
        }
        "message.updated" => {
            let info = properties.get("info").cloned().unwrap_or_else(|| json!({}));
            let message_id = info.get("id").and_then(Value::as_str).unwrap_or_default();
            let role = info.get("role").and_then(Value::as_str).unwrap_or_default();
            if !message_id.is_empty() && !role.is_empty() {
                state.message_roles.insert(message_id.into(), role.into());
            }
            if role != "assistant" {
                return vec![];
            }
            let Some(tokens) = info.get("tokens") else {
                return vec![];
            };
            let mut event = with_data(
                "usage.updated",
                &properties,
                json!({
                    "usage": {
                        "input_tokens": tokens.get("input").cloned().unwrap_or(Value::Null),
                        "output_tokens": tokens.get("output").cloned().unwrap_or(Value::Null),
                        "cached_input_tokens": tokens.pointer("/cache/read").cloned().unwrap_or(Value::Null),
                        "reasoning_tokens": tokens.get("reasoning").cloned().unwrap_or(Value::Null),
                    },
                    "cost": info.get("cost").cloned().unwrap_or(Value::Null),
                    "model": info.get("modelID").cloned().unwrap_or(Value::Null),
                    "provider": info.get("providerID").cloned().unwrap_or(Value::Null),
                }),
            );
            event.item_id = (!message_id.is_empty()).then(|| message_id.into());
            vec![event]
        }
        "message.part.delta" => {
            let message_id = properties
                .get("messageID")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if state.message_roles.get(message_id).map(String::as_str) != Some("assistant") {
                return vec![];
            }
            let field = properties
                .get("field")
                .and_then(Value::as_str)
                .unwrap_or("text");
            let mut event = with_data(
                if field.contains("reasoning") {
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
            event.text = properties
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "message.part.updated" => normalize_opencode_part(&properties, state),
        "session.diff" => vec![with_data("diff.updated", &properties, properties.clone())],
        "todo.updated" => {
            let mut event = with_data("plan.updated", &properties, properties.clone());
            event.title = Some("OpenCode plan".into());
            vec![event]
        }
        "permission.v2.asked" | "permission.asked" => {
            let mut event = with_data("approval.requested", &properties, properties.clone());
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
            event.title = Some(format!("Approve {action}"));
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
            vec![event]
        }
        "session.error" => {
            let mut event = with_data("error", &properties, properties.clone());
            event.status = Some("failed".into());
            event.text = properties
                .pointer("/error/data/message")
                .or_else(|| properties.pointer("/error/message"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| properties.get("error").map(Value::to_string));
            vec![event]
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
    state: &OpenCodeStreamState,
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
    let is_assistant = state.message_roles.get(message_id).map(String::as_str) == Some("assistant");
    let item_id = part.get("id").and_then(Value::as_str).map(str::to_owned);
    match part_type {
        "text" if is_assistant && part.pointer("/time/end").is_some() => {
            let mut event = with_data("message.completed", &part, part.clone());
            event.item_id = item_id;
            event.role = Some("assistant".into());
            event.status = Some("completed".into());
            event.text = part.get("text").and_then(Value::as_str).map(str::to_owned);
            vec![event]
        }
        "reasoning" if is_assistant && part.pointer("/time/end").is_some() => {
            let mut event = with_data("reasoning.completed", &part, part.clone());
            event.item_id = item_id;
            event.status = Some("completed".into());
            event.text = part.get("text").and_then(Value::as_str).map(str::to_owned);
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
            } else if matches!(tool, "edit" | "write" | "patch") {
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
            vec![event]
        }
        "patch" => vec![with_data("diff.updated", &part, part.clone())],
        "step-start" => {
            let mut event = with_data("turn.started", &part, part.clone());
            event.data["turnId"] = part.get("id").cloned().unwrap_or(Value::Null);
            event.status = Some("working".into());
            vec![event]
        }
        "step-finish" => {
            let mut event = with_data("usage.updated", &part, json!({"usage": part.get("tokens")}));
            event.item_id = item_id;
            vec![event]
        }
        _ => vec![],
    }
}

impl NormalizedEvent {
    fn new(kind: &str) -> Self {
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

pub fn normalize_codex_message(message: &Value) -> Vec<NormalizedEvent> {
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
            let mut event = with_data("turn.started", &params, params.clone());
            event.status = Some("working".into());
            vec![event]
        }
        "turn/completed" => {
            let status = params
                .pointer("/turn/status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let mut event = with_data("turn.completed", &params, params.clone());
            event.status = Some(status.into());
            vec![event]
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
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "item/commandExecution/outputDelta" => {
            let mut event = with_data("command.output_delta", &params, params.clone());
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            event.status = Some("inProgress".into());
            vec![event]
        }
        "item/fileChange/outputDelta" => {
            let mut event = with_data("diff.delta", &params, params.clone());
            event.text = params
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_owned);
            vec![event]
        }
        "item/mcpToolCall/progress" => {
            let mut event = with_data("tool.progress", &params, params.clone());
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
        "thread/tokenUsage/updated" => vec![with_data("usage.updated", &params, params.clone())],
        "turn/diff/updated" | "item/fileChange/patchUpdated" => {
            vec![with_data("diff.updated", &params, params.clone())]
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
        "item/started" | "item/completed" => normalize_item(method, &params),
        _ => {
            let mut event = with_data("provider.unknown", &params, params.clone());
            event.title = Some(method.into());
            vec![event]
        }
    }
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
    let mut event = with_data("approval.requested", &params, params.clone());
    event.item_id = params
        .get("itemId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.title = Some(
        match method {
            "item/fileChange/requestApproval" => "Approve file changes",
            "item/tool/requestUserInput" => "Input required",
            "mcpServer/elicitation/request" => "Tool input required",
            _ => "Approve command",
        }
        .into(),
    );
    event.text = params
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.status = Some("pending".into());
    event.data["requestId"] = message.get("id").cloned().unwrap_or(Value::Null);
    event.data["requestMethod"] = Value::String(method.into());
    Some(event)
}

fn normalize_item(method: &str, params: &Value) -> Vec<NormalizedEvent> {
    let item = params.get("item").cloned().unwrap_or_else(|| json!({}));
    let item_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
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
}

pub fn normalize_claude_message_with_state(
    message: &Value,
    state: &mut ClaudeStreamState,
) -> Vec<NormalizedEvent> {
    let Some(kind) = message.get("type").and_then(Value::as_str) else {
        return vec![];
    };
    match kind {
        "system" => normalize_claude_system(message),
        "stream_event" => normalize_claude_stream(message, state),
        "assistant" => normalize_claude_assistant(message, state),
        "user" => normalize_claude_user(message),
        "result" => {
            *state = ClaudeStreamState::default();
            normalize_claude_result(message)
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
        // Hooks/notifications are noise in the conversation surface.
        _ => vec![],
    }
}

fn normalize_claude_stream(message: &Value, state: &mut ClaudeStreamState) -> Vec<NormalizedEvent> {
    let event = message.get("event").cloned().unwrap_or_else(|| json!({}));
    let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
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
                    normalized.item_id = state
                        .active_reasoning_id
                        .clone()
                        .or_else(|| Some(format!("reasoning-{message_id}")));
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
                "tool_use" => {
                    let tool_id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_owned();
                    let mut normalized = with_data("tool.started", message, block.clone());
                    normalized.item_id = Some(tool_id);
                    normalized.title = block.get("name").and_then(Value::as_str).map(str::to_owned);
                    normalized.status = Some("inProgress".into());
                    vec![normalized]
                }
                "thinking" => {
                    let message_id = state
                        .active_message_id
                        .clone()
                        .unwrap_or_else(|| "claude-live".into());
                    state.active_reasoning_id = Some(format!("reasoning-{message_id}"));
                    vec![]
                }
                _ => vec![],
            }
        }
        "message_stop" => {
            // Keep active ids until the assistant snapshot or result arrives so
            // completed text can replace the same bubble.
            vec![]
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
    state.active_message_id = Some(message_id.clone());
    state.active_reasoning_id = Some(format!("reasoning-{message_id}"));
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
    for part in content {
        let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
        match part_type {
            "tool_use" => {
                let tool_id = part
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_owned();
                let mut event = with_data("tool.started", message, part.clone());
                event.item_id = Some(tool_id);
                event.title = part.get("name").and_then(Value::as_str).map(str::to_owned);
                event.status = Some("inProgress".into());
                events.push(event);
            }
            "thinking" => {
                if let Some(thinking) = part.get("thinking").and_then(Value::as_str) {
                    if !thinking.is_empty() {
                        let mut event = NormalizedEvent::new("reasoning.completed");
                        event.item_id = Some(format!("reasoning-{message_id}"));
                        event.status = Some("completed".into());
                        event.text = Some(thinking.to_owned());
                        events.push(event);
                    }
                }
            }
            _ => {}
        }
    }
    events
}

fn normalize_claude_user(message: &Value) -> Vec<NormalizedEvent> {
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
                let mut event = with_data("tool.completed", message, part.clone());
                event.item_id = Some(tool_id);
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

fn normalize_claude_result(message: &Value) -> Vec<NormalizedEvent> {
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
        events.push(with_data(
            "usage.updated",
            message,
            json!({"usage": usage, "totalCostUsd": message.get("total_cost_usd")}),
        ));
    }
    if message
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || subtype.contains("error")
    {
        let mut error = with_data("error", message, message.clone());
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
    let mut event = with_data("approval.requested", message, request.clone());
    event.item_id = request
        .get("tool_use_id")
        .or_else(|| request.get("request_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    event.title = Some(
        request
            .get("tool_name")
            .and_then(Value::as_str)
            .map(|name| format!("Approve {name}"))
            .unwrap_or_else(|| "Approve tool".into()),
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
    Some(event)
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
    fn normalizes_tool_without_leaking_provider_type() {
        let events = normalize_codex_message(
            &json!({"method":"item/started","params":{"item":{"type":"mcpToolCall","id":"t1","tool":"search","status":"inProgress"}}}),
        );
        assert_eq!(events[0].kind, "tool.started");
        assert_eq!(events[0].title.as_deref(), Some("search"));
    }
    #[test]
    fn preserves_unknown_provider_event() {
        let events =
            normalize_codex_message(&json!({"method":"future/newThing","params":{"value":7}}));
        assert_eq!(events[0].kind, "provider.unknown");
        assert_eq!(events[0].title.as_deref(), Some("future/newThing"));
        assert_eq!(events[0].data["value"], 7);
    }
    #[test]
    fn converts_server_request_to_approval() {
        let event=normalize_codex_request(&json!({"id":42,"method":"item/commandExecution/requestApproval","params":{"itemId":"c1","command":"cargo test","reason":"needs access"}})).unwrap();
        assert_eq!(event.kind, "approval.requested");
        assert_eq!(event.data["requestId"], 42);
        assert_eq!(event.status.as_deref(), Some("pending"));
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
    fn skips_empty_claude_thinking_deltas() {
        let events = normalize_claude_message(&json!({
            "type":"stream_event",
            "event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}
        }));
        assert!(events.is_empty());
    }

    #[test]
    fn normalizes_claude_tool_and_result() {
        let mut state = ClaudeStreamState::default();
        let started = normalize_claude_message_with_state(
            &json!({
                "type":"assistant",
                "message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"pwd"}}]}
            }),
            &mut state,
        );
        assert_eq!(started[0].kind, "tool.started");
        assert_eq!(started[0].title.as_deref(), Some("Bash"));
        let completed = normalize_claude_message(&json!({
            "type":"user",
            "message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"/tmp","is_error":false}]}
        }));
        assert_eq!(completed[0].kind, "tool.completed");
        assert_eq!(completed[0].text.as_deref(), Some("/tmp"));
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
    fn normalizes_opencode_streaming_messages_and_usage() {
        let mut state = OpenCodeStreamState::default();
        let usage = normalize_opencode_message_with_state(&json!({
            "type":"message.updated",
            "properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant","tokens":{"input":4,"output":2,"reasoning":1,"cache":{"read":3,"write":0}},"cost":0.01,"modelID":"model","providerID":"provider"}}
        }), &mut state);
        assert_eq!(usage[0].kind, "usage.updated");
        assert_eq!(usage[0].data["usage"]["input_tokens"], 4);

        let delta = normalize_opencode_message_with_state(&json!({
            "type":"message.part.delta",
            "properties":{"sessionID":"ses_1","messageID":"msg_1","partID":"prt_1","field":"text","delta":"hello"}
        }), &mut state);
        assert_eq!(delta[0].kind, "message.delta");
        assert_eq!(delta[0].item_id.as_deref(), Some("prt_1"));
        assert_eq!(delta[0].text.as_deref(), Some("hello"));
    }

    #[test]
    fn normalizes_opencode_tools_permissions_and_turn_state() {
        let mut state = OpenCodeStreamState::default();
        let _ = normalize_opencode_message_with_state(&json!({
            "type":"message.updated",
            "properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant"}}
        }), &mut state);
        let tool = normalize_opencode_message_with_state(&json!({
            "type":"message.part.updated",
            "properties":{"sessionID":"ses_1","part":{"id":"prt_2","sessionID":"ses_1","messageID":"msg_1","type":"tool","tool":"bash","state":{"status":"completed","input":{"command":"pwd"},"output":"/tmp","title":"Run pwd"}}}
        }), &mut state);
        assert_eq!(tool[0].kind, "command.completed");
        assert_eq!(tool[0].text.as_deref(), Some("/tmp"));

        let permission = normalize_opencode_message_with_state(&json!({
            "type":"permission.v2.asked",
            "properties":{"id":"per_1","sessionID":"ses_1","action":"bash","resources":["git status"],"source":{"callID":"call_1"}}
        }), &mut state);
        assert_eq!(permission[0].kind, "approval.requested");
        assert_eq!(permission[0].data["requestId"], "per_1");

        let busy = normalize_opencode_message_with_state(&json!({
            "id":"evt_1","type":"session.status","properties":{"sessionID":"ses_1","status":{"type":"busy"}}
        }), &mut state);
        assert_eq!(busy[0].kind, "turn.started");
        let idle = normalize_opencode_message_with_state(&json!({
            "type":"session.idle","properties":{"sessionID":"ses_1"}
        }), &mut state);
        assert_eq!(idle[0].kind, "turn.completed");
    }
}
