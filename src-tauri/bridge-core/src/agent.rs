use serde_json::{json, Value};
use std::collections::HashMap;

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
            let role = state
                .message_roles
                .get(message_id)
                .map(String::as_str)
                .unwrap_or("assistant");
            if role != "assistant" {
                return vec![];
            }
            if !message_id.is_empty() && !state.message_roles.contains_key(message_id) {
                state
                    .message_roles
                    .insert(message_id.to_string(), "assistant".to_string());
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
            let mut event = with_data("error", &properties, properties.clone());
            event.status = Some("failed".into());
            event.title = Some("OpenCode error".into());
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
    let is_assistant = state
        .message_roles
        .get(message_id)
        .map(String::as_str)
        .unwrap_or("assistant")
        == "assistant";
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
        "reasoning" if is_assistant => {
            let status = part
                .pointer("/state/status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let is_running = status == "running" || status == "inProgress";
            let finished = part.pointer("/time/end").is_some()
                || !is_running
                || part.get("completed").and_then(Value::as_bool).unwrap_or(false);
            let kind = if finished {
                "reasoning.completed"
            } else {
                "reasoning.delta"
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
            let mut event = with_data("turn.started", &params, params.clone());
            event.status = Some("working".into());
            vec![event]
        }
        "turn/completed" => {
            state.active_reasoning_id = None;
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
                    let id = "reasoning-1".to_string();
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
                    let id = "reasoning-1".to_string();
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
            let mut event = with_data("model.rerouted", &params, params.clone());
            event.title = Some("Model rerouted".into());
            event.text = Some(match reason {
                Some(reason) => format!("Codex switched from {from_model} to {to_model}: {reason}"),
                None => format!("Codex switched from {from_model} to {to_model}."),
            });
            event.status = Some("completed".into());
            vec![event]
        }
        "process/exited" => {
            let exit_code = params
                .get("exitCode")
                .or_else(|| params.get("exit_code"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            if exit_code != 0 {
                let mut event = with_data("error", &params, params.clone());
                event.title = Some("Process exited with error".into());
                event.text = Some(format!("Codex process exited with code {exit_code}."));
                event.status = Some("failed".into());
                vec![event]
            } else {
                vec![]
            }
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
                        .unwrap_or("reasoning-1");
                    state.active_reasoning_id = Some(id.to_string());
                } else {
                    state.active_reasoning_id = None;
                }
            }
            normalize_item(method, &params)
        }
        _ if is_codex_internal_notification(method) => vec![],
        _ => {
            let mut event = with_data("provider.unknown", &params, params.clone());
            event.title = Some(method.into());
            vec![event]
        }
    }
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
            | "thread/compacted"
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
    event.data["interactionKind"] = Value::String(
        if is_question { "question" } else { "permission" }.into(),
    );
    if !is_question {
        event.data["actions"] = permission_actions(true);
    }
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
    event.data["interactionKind"] = Value::String("permission".into());
    event.data["actions"] = permission_actions(true);
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
    fn normalizes_codex_process_exited_with_error() {
        let events = normalize_codex_message(&json!({
            "method": "process/exited",
            "params": {
                "exitCode": 137
            }
        }));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "error");
        assert_eq!(events[0].status.as_deref(), Some("failed"));
        assert!(events[0].text.as_deref().unwrap().contains("137"));

        let ok_exit = normalize_codex_message(&json!({
            "method": "process/exited",
            "params": {
                "exitCode": 0
            }
        }));
        assert!(ok_exit.is_empty());
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
    fn codex_internal_notifications_do_not_become_unknown_events() {
        for method in [
            "hook/started",
            "hook/completed",
            "item/commandExecution/terminalInteraction",
            "mcpServer/event/stream/notification",
            "fs/changed",
            "thread/compacted",
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
        let usage = normalize_opencode_message_with_state(
            &json!({
                "type":"message.updated",
                "properties":{"sessionID":"ses_1","info":{"id":"msg_1","role":"assistant","tokens":{"input":4,"output":2,"reasoning":1,"cache":{"read":3,"write":0}},"cost":0.01,"modelID":"model","providerID":"provider"}}
            }),
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
}
