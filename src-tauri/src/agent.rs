use serde_json::{json, Value};

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
}
