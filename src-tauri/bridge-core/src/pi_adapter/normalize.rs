use crate::agent::{self, NormalizedEvent};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// One state per supervised Pi session. Provider timestamps give messages
/// stable IDs across resume; the nonce also makes timestamp-less frames safe.
#[derive(Default)]
pub(super) struct StreamState {
    message: String,
    sequence: u64,
    nonce: String,
    active: bool,
    failed: bool,
    blocks: HashMap<String, (String, String)>,
    terminals: HashMap<String, String>,
    finished_messages: HashSet<String>,
    finished_tools: HashSet<String>,
    tools: HashMap<String, (String, Value)>,
}
fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_owned()
}
fn event(
    kind: &str,
    id: Option<String>,
    text: Option<String>,
    status: &str,
    data: Value,
) -> NormalizedEvent {
    let mut event = NormalizedEvent::new(kind);
    event.item_id = id;
    event.text = text;
    event.status = Some(status.into());
    event.data = data;
    event
}
impl StreamState {
    fn message_id(&mut self, message: &Value) -> String {
        if let Some(timestamp) = message["timestamp"].as_u64() {
            return format!("pi-message-{timestamp}");
        }
        if self.message.is_empty() {
            if self.nonce.is_empty() {
                self.nonce = uuid::Uuid::new_v4().to_string();
            }
            self.sequence += 1;
            self.message = format!("pi-message-{}-{}", self.nonce, self.sequence);
        }
        self.message.clone()
    }
    fn terminal(
        &mut self,
        message: &str,
        index: u64,
        kind: &str,
        text: String,
    ) -> Option<NormalizedEvent> {
        let id = format!("{message}-{index}");
        self.blocks.remove(&id);
        if self.terminals.get(&id) == Some(&text) {
            return None;
        }
        self.terminals.insert(id.clone(), text.clone());
        let mut output = event(
            if kind == "thinking" {
                "reasoning.completed"
            } else {
                "message.completed"
            },
            Some(id),
            Some(text),
            "completed",
            json!({}),
        );
        if kind != "thinking" {
            output.role = Some("assistant".into());
        }
        Some(output)
    }
    fn flush(&mut self) -> Vec<NormalizedEvent> {
        self.blocks
            .drain()
            .map(|(id, (kind, text))| {
                let mut output = event(
                    if kind == "thinking" {
                        "reasoning.completed"
                    } else {
                        "message.completed"
                    },
                    Some(id),
                    Some(text),
                    "completed",
                    json!({}),
                );
                if kind != "thinking" {
                    output.role = Some("assistant".into());
                }
                output
            })
            .collect()
    }
    fn tool(&mut self, value: &Value, phase: &str) -> Vec<NormalizedEvent> {
        let id = string(value, "toolCallId");
        if id.is_empty() || self.finished_tools.contains(&id) {
            return vec![];
        }
        let previous = self
            .tools
            .entry(id.clone())
            .or_insert((string(value, "toolName"), json!({})));
        if !string(value, "toolName").is_empty() {
            previous.0 = string(value, "toolName");
        }
        if let Some(args) = value.get("args") {
            previous.1 = args.clone();
        }
        let (name, args) = previous.clone();
        let family = if name == "bash" { "command" } else { "tool" };
        let result = value.get("result").or_else(|| value.get("partialResult"));
        let text = result
            .and_then(|value| value["content"].as_array())
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        let failed = value["isError"] == true;
        let mut output = event(
            &format!("{family}.{phase}"),
            Some(id.clone()),
            text.clone(),
            if phase == "completed" {
                if failed {
                    "failed"
                } else {
                    "completed"
                }
            } else {
                "inProgress"
            },
            json!({"name":name,"input":args,"output":text,"is_error":failed}),
        );
        output.title = Some(if name == "bash" {
            args["command"].as_str().unwrap_or("bash").to_owned()
        } else {
            name
        });
        if let Some(duration) = value["durationMs"].as_u64() {
            output.data["durationMs"] = json!(duration);
        }
        if family == "command" {
            output.data["command"] = args["command"].clone();
        }
        if phase == "completed" {
            self.finished_tools.insert(id.clone());
            self.tools.remove(&id);
        }
        vec![output]
    }
    pub fn normalize(&mut self, value: &Value) -> Vec<NormalizedEvent> {
        match value["type"].as_str().unwrap_or_default() {
            "agent_start" => {
                if self.active {
                    return vec![];
                }
                self.active = true;
                self.failed = false;
                // Bound deduplication history while retaining the current run.
                self.terminals.clear();
                self.finished_messages.clear();
                self.finished_tools.clear();
                vec![event("turn.started", None, None, "working", json!({}))]
            }
            "agent_end" | "turn_start" | "turn_end" => vec![],
            "agent_settled" | "bridge_handled" => {
                if !self.active && value["type"] != "bridge_handled" {
                    return vec![];
                }
                self.active = false;
                let mut events = self.flush();
                let kind = if self.failed {
                    "turn.failed"
                } else {
                    "turn.completed"
                };
                events.push(event(
                    kind,
                    None,
                    None,
                    if self.failed {
                        "failed"
                    } else if value["aborted"] == true {
                        "interrupted"
                    } else {
                        "completed"
                    },
                    json!({}),
                ));
                events
            }
            "bridge_error" => {
                self.failed = true;
                self.active = false;
                let mut events = self.flush();
                events.push(event(
                    "turn.failed",
                    None,
                    Some(string(value, "message")),
                    "failed",
                    json!({}),
                ));
                events
            }
            "message_start" if value["message"]["role"] == "assistant" => {
                self.message.clear();
                self.message = self.message_id(&value["message"]);
                vec![]
            }
            "message_update" => {
                let update = &value["assistantMessageEvent"];
                let index = update["contentIndex"].as_u64().unwrap_or(0);
                let message = self.message_id(&Value::Null);
                let id = format!("{message}-{index}");
                match update["type"].as_str().unwrap_or_default() {
                    "text_delta" | "thinking_delta" => {
                        if self.terminals.contains_key(&id)
                            || self.finished_messages.contains(&message)
                        {
                            return vec![];
                        }
                        let thinking = update["type"] == "thinking_delta";
                        let text = string(update, "delta");
                        let block = self.blocks.entry(id.clone()).or_insert((
                            if thinking { "thinking" } else { "text" }.into(),
                            String::new(),
                        ));
                        block.1.push_str(&text);
                        let mut output = event(
                            if thinking {
                                "reasoning.delta"
                            } else {
                                "message.delta"
                            },
                            Some(id),
                            Some(text),
                            "streaming",
                            json!({}),
                        );
                        if !thinking {
                            output.role = Some("assistant".into());
                        }
                        vec![output]
                    }
                    "text_end" | "thinking_end" => self
                        .terminal(
                            &message,
                            index,
                            if update["type"] == "thinking_end" {
                                "thinking"
                            } else {
                                "text"
                            },
                            string(update, "content"),
                        )
                        .into_iter()
                        .collect(),
                    _ => vec![], // Execution events own the tool's lifecycle.
                }
            }
            "message_end" if value["message"]["role"] == "assistant" => {
                let message = &value["message"];
                let id = self.message_id(message);
                if !self.finished_messages.insert(id.clone()) {
                    return vec![];
                }
                let mut events = Vec::new();
                for (index, block) in message["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let kind = block["type"].as_str().unwrap_or_default();
                    if matches!(kind, "text" | "thinking") {
                        if let Some(output) = self.terminal(
                            &id,
                            index as u64,
                            kind,
                            string(
                                block,
                                if kind == "thinking" {
                                    "thinking"
                                } else {
                                    "text"
                                },
                            ),
                        ) {
                            events.push(output);
                        }
                    }
                }
                if message["stopReason"] == "error" {
                    // Pi retries errors after message_end. Keep the run alive;
                    // only agent_settled turns the terminal failure into idle.
                    self.failed = true;
                    events.push(event(
                        "error",
                        None,
                        Some("Pi provider request failed; check Pi's provider setup".into()),
                        "failed",
                        json!({}),
                    ));
                } else if message["stopReason"] != "aborted" {
                    self.failed = false;
                }
                if let Some(usage) = message.get("usage").filter(|usage| usage.is_object()) {
                    let data = json!({"input_tokens":usage["input"].as_u64().unwrap_or(0),"output_tokens":usage["output"].as_u64().unwrap_or(0),"cache_read_tokens":usage["cacheRead"].as_u64().unwrap_or(0),"cache_write_tokens":usage["cacheWrite"].as_u64().unwrap_or(0),"totalCostUsd":usage.pointer("/cost/total").and_then(Value::as_f64),"model":message["model"],"provider":message["provider"],"messageId":id});
                    events.push(event(
                        "usage.updated",
                        Some(format!("{id}-usage")),
                        None,
                        "completed",
                        data,
                    ));
                }
                events
            }
            "tool_execution_start" => self.tool(value, "started"),
            "tool_execution_update" => self.tool(value, "progress"),
            "tool_execution_end" => self.tool(value, "completed"),
            "extension_ui_request" if value["method"] == "confirm" => {
                let title = value["title"]
                    .as_str()
                    .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    .unwrap_or(Value::Null);
                if title["bridgePermission"] != 1 {
                    return vec![];
                }
                let mut output = event(
                    "approval.requested",
                    title["toolCallId"].as_str().map(str::to_owned),
                    value["message"].as_str().map(str::to_owned),
                    "pending",
                    json!({"requestId":value["id"],"requestMethod":"pi.tool_permission","interactionKind":"permission","actions":[{"id":"decline","decision":"decline","label":"Decline"},{"id":"accept","decision":"accept","label":"Allow once"}]}),
                );
                output.title = Some(format!(
                    "Allow {}",
                    title["toolName"].as_str().unwrap_or("tool")
                ));
                vec![output]
            }
            "compaction_start" => vec![agent::native_compacting("pi")],
            "compaction_end" => {
                if value["aborted"] == true || value.get("result").is_none() {
                    return vec![event(
                        "error",
                        None,
                        Some("Pi context compaction did not complete".into()),
                        "failed",
                        json!({}),
                    )];
                }
                vec![event(
                    "context.compacted",
                    None,
                    None,
                    "completed",
                    json!({"harness":"pi","preTokens":value.pointer("/result/tokensBefore"),"postTokens":value.pointer("/result/estimatedTokensAfter"),"willRetry":value["willRetry"] == true}),
                )]
            }
            "auto_retry_start"
            | "summarization_retry_scheduled"
            | "summarization_retry_attempt_start" => vec![event(
                "provider.status",
                None,
                Some("Pi is retrying the provider request".into()),
                "retrying",
                json!({"attempt":value["attempt"].as_u64(),"maxAttempts":value["maxAttempts"].as_u64()}),
            )],
            "auto_retry_end" if value["success"] == false => {
                self.failed = true;
                vec![event(
                    "error",
                    None,
                    Some("Pi exhausted its provider retries".into()),
                    "failed",
                    json!({}),
                )]
            }
            "auto_retry_end" | "summarization_retry_finished" => {
                vec![event("provider.status", None, None, "working", json!({}))]
            }
            "extension_error" => {
                self.failed = true;
                vec![event(
                    "error",
                    None,
                    Some(
                        "Pi's Bridge permission extension failed; tool execution was denied".into(),
                    ),
                    "failed",
                    json!({}),
                )]
            }
            _ => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_settled_finishes_the_run_and_duplicate_terminals_are_ignored() {
        let mut state = StreamState::default();
        assert_eq!(
            state.normalize(&json!({"type":"agent_start"}))[0].kind,
            "turn.started"
        );
        state.normalize(
            &json!({"type":"message_start","message":{"role":"assistant","timestamp":42}}),
        );
        state.normalize(&json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"Consider"}}));
        let thought = state.normalize(&json!({"type":"message_update","assistantMessageEvent":{"type":"thinking_end","contentIndex":0,"content":"Consider fully"}}));
        assert_eq!(thought[0].kind, "reasoning.completed");
        assert_eq!(thought[0].text.as_deref(), Some("Consider fully"));
        let end = json!({"type":"message_end","message":{"role":"assistant","timestamp":42,"model":"local-model","provider":"llama","content":[{"type":"thinking","thinking":"Consider fully"},{"type":"text","text":"Hello"}],"usage":{"input":12,"output":8,"cacheRead":3,"cacheWrite":4,"cost":{"total":0.01}},"stopReason":"stop"}});
        let events = state.normalize(&end);
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            ["message.completed", "usage.updated"]
        );
        assert_eq!(events[1].data["cache_read_tokens"], 3);
        assert_eq!(events[1].data["totalCostUsd"], 0.01);
        assert!(state.normalize(&end).is_empty());
        assert!(state
            .normalize(&json!({"type":"agent_end","willRetry":true}))
            .is_empty());
        assert_eq!(
            state.normalize(&json!({"type":"agent_settled","aborted":false}))[0].kind,
            "turn.completed"
        );
        assert!(state.normalize(&json!({"type":"agent_settled"})).is_empty());
    }
    #[test]
    fn interruption_flushes_partial_blocks_and_preserves_item_ids() {
        let mut state = StreamState::default();
        state.normalize(&json!({"type":"agent_start"}));
        let delta = state.normalize(&json!({"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Partial"}}));
        let events = state.normalize(&json!({"type":"agent_settled","aborted":true}));
        assert_eq!(events[0].item_id, delta[0].item_id);
        assert_eq!(events[0].text.as_deref(), Some("Partial"));
        assert_eq!(events[1].status.as_deref(), Some("interrupted"));
    }
    #[test]
    fn tools_retries_compaction_and_permissions_use_shared_events() {
        let mut state = StreamState::default();
        let start = json!({"type":"tool_execution_start","toolCallId":"t","toolName":"bash","args":{"command":"ls"}});
        assert_eq!(state.normalize(&start)[0].kind, "command.started");
        let end = json!({"type":"tool_execution_end","toolCallId":"t","toolName":"bash","result":{"content":[{"type":"text","text":"files"}]},"isError":true,"durationMs":2});
        assert_eq!(state.normalize(&end)[0].status.as_deref(), Some("failed"));
        assert!(state.normalize(&end).is_empty());
        assert!(state.normalize(&start).is_empty());
        assert_eq!(
            state
                .normalize(&json!({"type":"auto_retry_start","attempt":2,"errorMessage":"secret"}))
                [0]
            .status
            .as_deref(),
            Some("retrying")
        );
        assert_eq!(
            state.normalize(&json!({"type":"compaction_start"}))[0].kind,
            "context.compacting"
        );
        let compact=state.normalize(&json!({"type":"compaction_end","result":{"tokensBefore":100,"estimatedTokensAfter":10}}));
        assert_eq!(compact[0].kind, "context.compacted");
        assert_eq!(compact[0].data["preTokens"], 100);
        let approval=state.normalize(&json!({"type":"extension_ui_request","method":"confirm","id":"p","title":"{\"bridgePermission\":1,\"toolCallId\":\"t\",\"toolName\":\"read\"}","message":"{}"}));
        assert_eq!(approval[0].kind, "approval.requested");
        assert_eq!(approval[0].data["requestId"], "p");
        assert!(!format!(
            "{:?}",
            state.normalize(&json!({"type":"extension_error","error":"SECRET"}))
        )
        .contains("SECRET"));
    }
}
