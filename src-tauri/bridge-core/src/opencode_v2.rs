//! OpenCode 2.x serves a different API from the one Bridge was built against:
//! every route moved under `/api`, the server's basic-auth user is fixed to
//! `opencode`, and `/api/event` publishes fine-grained session events
//! (`session.text.delta`, `session.tool.called`, `session.execution.*`) in
//! place of 1.x's part snapshots. This module keeps the rest of Bridge on one
//! shape: it translates each 2.x frame into the 1.x frames the normalizer,
//! frame filter and usage ledger already understand, and builds the 2.x
//! request bodies and catalog.
//!
//! The authoritative 2.x wire shapes live upstream in
//! `packages/schema/src/session-event.ts`, `permission.ts` and `form.ts`.

use crate::opencode_adapter::{OpenCodeAuthMethod, OpenCodeCatalog, OpenCodeModel, OpenCodeProvider};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

/// A request the server is waiting on, remembered so a reply can name the
/// session it belongs to. 2.x scopes permission and form replies by session,
/// and a subagent's request belongs to its child session, not the root.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PendingRequest {
    Permission { session_id: String },
    Form { session_id: String, fields: Vec<Value> },
}

pub(crate) type PendingRequests = Arc<Mutex<HashMap<String, PendingRequest>>>;

#[derive(Debug, Clone)]
struct ToolCall {
    name: String,
    input: Value,
    metadata: Value,
    started: Value,
}

/// Turns one 2.x `/api/event` frame into zero or more 1.x frames. Stateful
/// because 2.x tool events carry only what changed, while a 1.x tool part is a
/// full snapshot (name and input included) every time.
pub(crate) struct Translator {
    tools: HashMap<String, ToolCall>,
    pending: PendingRequests,
}

impl Translator {
    pub(crate) fn new(pending: PendingRequests) -> Self {
        Self {
            tools: HashMap::new(),
            pending,
        }
    }

    pub(crate) fn translate(&mut self, frame: &Value) -> Vec<Value> {
        let Some(kind) = frame.get("type").and_then(Value::as_str) else {
            return vec![];
        };
        let empty = json!({});
        let data = frame.get("data").unwrap_or(&empty);
        let created = frame.get("created").cloned().unwrap_or(Value::Null);
        let session = data.get("sessionID").cloned().unwrap_or(Value::Null);
        let message = data.get("assistantMessageID").cloned().unwrap_or(Value::Null);
        match kind {
            "server.connected" => vec![v1("server.connected", json!({}))],
            // Not published by 2.1, but shaped exactly like 1.x if it returns.
            "session.status" | "session.idle" => vec![v1(kind, data.clone())],
            "session.created" => vec![v1(
                "session.created",
                json!({
                    "sessionID": session,
                    "info": {
                        "id": session,
                        "parentID": data.get("parentID"),
                        "agent": data.get("agent"),
                        "title": data.get("title"),
                    },
                }),
            )],
            "session.execution.started" => vec![status(&session, json!({"type": "busy"}))],
            // An interrupt is the user's own stop, not a failure.
            "session.execution.succeeded" | "session.execution.interrupted" => {
                vec![status(&session, json!({"type": "idle"}))]
            }
            "session.execution.failed" => vec![v1(
                "session.error",
                json!({"sessionID": session, "error": error_v1(data.get("error"))}),
            )],
            "session.retry.scheduled" => vec![status(
                &session,
                json!({
                    "type": "retry",
                    "attempt": data.get("attempt"),
                    "message": data.pointer("/error/message"),
                    "next": data.get("at"),
                }),
            )],
            "session.step.started" => vec![
                message_updated(&session, &message, data.get("model")),
                part(&session, json!({
                    "id": format!("{}:step-start", str_of(&message)),
                    "messageID": message,
                    "sessionID": session,
                    "type": "step-start",
                })),
            ],
            "session.step.ended" | "session.step.failed" => {
                // A failed step still billed whatever it consumed; its error
                // reaches the turn through `session.execution.failed`.
                let Some(tokens) = data.get("tokens") else {
                    return vec![];
                };
                vec![part(&session, json!({
                    "id": format!("{}:step-finish", str_of(&message)),
                    "messageID": message,
                    "sessionID": session,
                    "type": "step-finish",
                    "reason": data.get("finish"),
                    "cost": data.get("cost").cloned().unwrap_or(json!(0)),
                    "tokens": inclusive_tokens(tokens),
                }))]
            }
            "session.text.started" | "session.reasoning.started" => {
                vec![part(&session, prose_part(kind, data, &created, None))]
            }
            "session.text.ended" | "session.reasoning.ended" => {
                vec![part(&session, prose_part(kind, data, &created, Some(&created)))]
            }
            "session.text.delta" | "session.reasoning.delta" => vec![v1(
                "message.part.delta",
                json!({
                    "sessionID": session,
                    "messageID": message,
                    "partID": prose_part_id(kind, data),
                    "field": "text",
                    "delta": data.get("delta"),
                }),
            )],
            "session.tool.input.started" => {
                let Some(id) = data.get("id").and_then(Value::as_str) else {
                    return vec![];
                };
                let call = ToolCall {
                    name: data.get("name").and_then(Value::as_str).unwrap_or("tool").to_owned(),
                    input: json!({}),
                    metadata: Value::Null,
                    started: created.clone(),
                };
                let state = json!({"status": "pending", "input": {}});
                let frame = tool_part(&session, &message, id, &call.name, state);
                self.tools.insert(id.to_owned(), call);
                vec![frame]
            }
            "session.tool.called" | "session.tool.progress" => {
                let Some(id) = data.get("id").and_then(Value::as_str) else {
                    return vec![];
                };
                let call = self.tools.entry(id.to_owned()).or_insert_with(|| ToolCall {
                    name: "tool".into(),
                    input: json!({}),
                    metadata: Value::Null,
                    started: created.clone(),
                });
                if let Some(input) = data.get("input") {
                    call.input = input.clone();
                }
                if let Some(metadata) = data.get("metadata") {
                    call.metadata = metadata.clone();
                }
                let mut state = json!({
                    "status": "running",
                    "input": call.input,
                    "time": {"start": call.started},
                });
                if !call.metadata.is_null() {
                    state["metadata"] = call.metadata.clone();
                }
                vec![tool_part(&session, &message, id, &call.name, state)]
            }
            "session.tool.success" | "session.tool.failed" => {
                let Some(id) = data.get("id").and_then(Value::as_str) else {
                    return vec![];
                };
                let call = self.tools.remove(id).unwrap_or(ToolCall {
                    name: "tool".into(),
                    input: json!({}),
                    metadata: Value::Null,
                    started: created.clone(),
                });
                let state = settled_tool_state(
                    kind == "session.tool.success",
                    &call.input,
                    data,
                    &call.started,
                    &created,
                );
                vec![tool_part(&session, &message, id, &call.name, state)]
            }
            "session.compaction.ended" => vec![v1("session.compacted", json!({"sessionID": session}))],
            "permission.asked" => {
                let (Some(id), Some(owner)) = (
                    data.get("id").and_then(Value::as_str),
                    session.as_str(),
                ) else {
                    return vec![];
                };
                self.pending.lock().unwrap().insert(
                    id.to_owned(),
                    PendingRequest::Permission { session_id: owner.to_owned() },
                );
                vec![v1(
                    "permission.asked",
                    json!({
                        "id": id,
                        "sessionID": session,
                        "action": data.get("action"),
                        "permission": data.get("action"),
                        "resources": data.get("resources"),
                        "patterns": data.get("resources"),
                        "metadata": data.get("metadata"),
                        "source": {
                            "callID": data.pointer("/source/id"),
                            "messageID": data.pointer("/source/messageID"),
                        },
                    }),
                )]
            }
            "permission.replied" => {
                if let Some(id) = data.get("requestID").and_then(Value::as_str) {
                    self.pending.lock().unwrap().remove(id);
                }
                vec![]
            }
            "form.created" => {
                let form = data.get("form").unwrap_or(&empty);
                let (Some(id), Some(owner)) = (
                    form.get("id").and_then(Value::as_str),
                    form.get("sessionID").and_then(Value::as_str),
                ) else {
                    return vec![];
                };
                let fields = form
                    .get("fields")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let title = form.get("title").and_then(Value::as_str).unwrap_or("Question");
                let questions = fields
                    .iter()
                    .map(|field| field_question(field, title))
                    .collect::<Vec<_>>();
                self.pending.lock().unwrap().insert(
                    id.to_owned(),
                    PendingRequest::Form { session_id: owner.to_owned(), fields },
                );
                vec![v1(
                    "question.asked",
                    json!({
                        "id": id,
                        "sessionID": owner,
                        "questions": questions,
                        "tool": {
                            "callID": form.pointer("/metadata/tool/id"),
                            "messageID": form.pointer("/metadata/tool/messageID"),
                        },
                    }),
                )]
            }
            "form.replied" | "form.cancelled" => {
                let Some(id) = data.get("id").and_then(Value::as_str) else {
                    return vec![];
                };
                self.pending.lock().unwrap().remove(id);
                vec![v1(
                    if kind == "form.replied" {
                        "question.replied"
                    } else {
                        "question.rejected"
                    },
                    json!({"sessionID": data.get("sessionID"), "requestID": id}),
                )]
            }
            // Inbox bookkeeping, usage running totals, instruction hashes,
            // shells, renames and every non-session event say nothing a
            // transcript needs.
            _ => vec![],
        }
    }
}

/// 2.x counts `input` without either cache bucket and `reasoning` outside
/// `output` (its total is the sum of all five). Bridge reads OpenCode usage
/// as 1.x reported it, cache-inclusive input and reasoning within output, so
/// the figures are restated in those terms.
pub(crate) fn inclusive_tokens(tokens: &Value) -> Value {
    let count = |pointer: &str| tokens.pointer(pointer).and_then(Value::as_i64).unwrap_or(0);
    let (read, write) = (count("/cache/read"), count("/cache/write"));
    json!({
        "input": count("/input") + read + write,
        "output": count("/output") + count("/reasoning"),
        "reasoning": count("/reasoning"),
        "cache": {"read": read, "write": write},
    })
}

fn v1(kind: &str, properties: Value) -> Value {
    json!({"type": kind, "properties": properties})
}

fn str_of(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn status(session: &Value, status: Value) -> Value {
    v1("session.status", json!({"sessionID": session, "status": status}))
}

fn part(session: &Value, part: Value) -> Value {
    v1("message.part.updated", json!({"sessionID": session, "part": part}))
}

/// 1.x errors are `{name, data: {message}}`; 2.x's are `{type, message}`.
fn error_v1(error: Option<&Value>) -> Value {
    let error = error.cloned().unwrap_or_else(|| json!({}));
    json!({
        "name": error.get("type").cloned().unwrap_or(json!("UnknownError")),
        "data": {"message": error.get("message").cloned().unwrap_or(json!("OpenCode failed"))},
    })
}

/// 1.x tied a model to an assistant message with `message.updated`; the
/// normalizer needs that role before it admits prose deltas, and the model
/// before it can attribute a step's tokens.
fn message_updated(session: &Value, message: &Value, model: Option<&Value>) -> Value {
    v1(
        "message.updated",
        json!({
            "sessionID": session,
            "info": {
                "id": message,
                "sessionID": session,
                "role": "assistant",
                "modelID": model.and_then(|model| model.get("id")),
                "providerID": model.and_then(|model| model.get("providerID")),
            },
        }),
    )
}

fn prose_type(kind: &str) -> &'static str {
    if kind.starts_with("session.reasoning") {
        "reasoning"
    } else {
        "text"
    }
}

/// 2.x numbers prose blocks per message and kind instead of naming them, so
/// the part id is derived; resync derives the same id from message content.
fn prose_part_id(kind: &str, data: &Value) -> String {
    prose_id(
        str_of(data.get("assistantMessageID").unwrap_or(&Value::Null)),
        prose_type(kind),
        data.get("ordinal").and_then(Value::as_u64).unwrap_or(0),
    )
}

fn prose_id(message: &str, kind: &str, ordinal: u64) -> String {
    format!("{message}:{kind}:{ordinal}")
}

fn prose_part(kind: &str, data: &Value, start: &Value, end: Option<&Value>) -> Value {
    let mut time = json!({"start": start});
    if let Some(end) = end {
        time["end"] = end.clone();
    }
    // `completed` finishes the part even when a frame carries no timestamp.
    json!({
        "id": prose_part_id(kind, data),
        "messageID": data.get("assistantMessageID"),
        "sessionID": data.get("sessionID"),
        "type": prose_type(kind),
        "text": data.get("text").cloned().unwrap_or(json!("")),
        "time": time,
        "completed": end.is_some(),
    })
}

fn tool_part(session: &Value, message: &Value, id: &str, name: &str, state: Value) -> Value {
    part(session, json!({
        "id": id,
        "callID": id,
        "messageID": message,
        "sessionID": session,
        "type": "tool",
        "tool": name,
        "state": state,
    }))
}

/// The text a 2.x tool result carries, as 1.x's single `output` string.
fn content_text(content: Option<&Value>) -> Option<String> {
    let text = content?
        .as_array()?
        .iter()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn settled_tool_state(succeeded: bool, input: &Value, data: &Value, start: &Value, end: &Value) -> Value {
    let mut state = json!({
        "status": if succeeded { "completed" } else { "error" },
        "input": input,
        "time": {"start": start, "end": end},
    });
    if let Some(metadata) = data.get("metadata") {
        state["metadata"] = metadata.clone();
    }
    if succeeded {
        state["output"] = json!(content_text(data.get("content")).unwrap_or_default());
    } else {
        state["error"] = json!(data
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Tool failed"));
        if let Some(output) = content_text(data.get("content")) {
            state["output"] = json!(output);
        }
    }
    state
}

/// One 2.x form field as a 1.x `question` entry, which is what Bridge's
/// question card renders.
fn field_question(field: &Value, form_title: &str) -> Value {
    let title = field.get("title").and_then(Value::as_str);
    let question = field
        .get("description")
        .and_then(Value::as_str)
        .or(title)
        .or_else(|| field.get("url").and_then(Value::as_str))
        .unwrap_or(form_title);
    let options = field
        .get("options")
        .and_then(Value::as_array)
        .map(|options| {
            options
                .iter()
                .map(|option| {
                    json!({
                        "label": option.get("label").or_else(|| option.get("value")),
                        "description": option.get("description").cloned().unwrap_or(json!("")),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| match field.get("type").and_then(Value::as_str) {
            Some("boolean") => vec![
                json!({"label": "Yes", "description": ""}),
                json!({"label": "No", "description": ""}),
            ],
            _ => vec![],
        });
    json!({
        "header": title.unwrap_or(form_title),
        "question": question,
        "options": options,
        "multiple": field.get("type").and_then(Value::as_str) == Some("multiselect"),
        "custom": field.get("custom").and_then(Value::as_bool).unwrap_or(true),
    })
}

/// Bridge answers a question card with one list of chosen labels per
/// question, in order. A 2.x form wants `{fieldKey: value}` typed by field,
/// with option values rather than their labels.
pub(crate) fn form_answer(fields: &[Value], answers: &Value) -> Result<Value, String> {
    let mut answer = serde_json::Map::new();
    for (index, field) in fields.iter().enumerate() {
        let Some(key) = field.get("key").and_then(Value::as_str) else {
            continue;
        };
        let chosen = match answers.get(index) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            Some(Value::String(text)) => vec![text.clone()],
            _ => vec![],
        };
        let chosen = chosen
            .into_iter()
            .map(|label| option_value(field, &label))
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>();
        if chosen.is_empty() {
            continue;
        }
        let value = match field.get("type").and_then(Value::as_str) {
            Some("multiselect") => json!(chosen),
            Some("boolean") => {
                let text = chosen[0].to_ascii_lowercase();
                json!(matches!(text.as_str(), "yes" | "true" | "y" | "1"))
            }
            Some("number" | "integer") => {
                let number = chosen[0].trim().parse::<f64>().map_err(|_| {
                    format!("OpenCode expects a number for {key}, not {:?}", chosen[0])
                })?;
                json!(number)
            }
            _ => json!(chosen.join(", ")),
        };
        answer.insert(key.to_owned(), value);
    }
    Ok(Value::Object(answer))
}

fn option_value(field: &Value, label: &str) -> String {
    field
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|option| option.get("label").and_then(Value::as_str) == Some(label))
        .and_then(|option| option.get("value").and_then(Value::as_str))
        .unwrap_or(label)
        .to_owned()
}

/// Snapshots that rebuild a reconnect gap from `/api/session/{id}/message`,
/// with the same part ids the live stream used so the normalizer replaces
/// rather than duplicates.
pub(crate) fn message_snapshots(session_id: &str, messages: &[Value]) -> Vec<Value> {
    let session = json!(session_id);
    let mut frames = Vec::new();
    for message in messages {
        if message.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message_id) = message.get("id").and_then(Value::as_str) else {
            continue;
        };
        let id = json!(message_id);
        frames.push(message_updated(&session, &id, message.get("model")));
        let mut ordinals = HashMap::<&str, u64>::new();
        for block in message.get("content").and_then(Value::as_array).into_iter().flatten() {
            match block.get("type").and_then(Value::as_str) {
                Some(kind @ ("text" | "reasoning")) => {
                    let ordinal = ordinals.entry(kind).or_default();
                    let created = block.pointer("/time/created").cloned().unwrap_or(Value::Null);
                    let completed = block
                        .pointer("/time/completed")
                        .cloned()
                        .or_else(|| message.pointer("/time/completed").cloned())
                        .unwrap_or(Value::Null);
                    frames.push(part(&session, json!({
                        "id": prose_id(message_id, kind, *ordinal),
                        "messageID": message_id,
                        "sessionID": session_id,
                        "type": kind,
                        "text": block.get("text").cloned().unwrap_or(json!("")),
                        "time": {"start": created, "end": completed},
                    })));
                    *ordinal += 1;
                }
                Some("tool") => {
                    let Some(call_id) = block.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let state = block.get("state").cloned().unwrap_or_else(|| json!({}));
                    let input = state.get("input").cloned().unwrap_or_else(|| json!({}));
                    let start = block.pointer("/time/created").cloned().unwrap_or(Value::Null);
                    let end = block.pointer("/time/completed").cloned().unwrap_or(Value::Null);
                    let tool_state = match state.get("status").and_then(Value::as_str) {
                        Some("completed") => settled_tool_state(true, &input, &state, &start, &end),
                        Some("error" | "failed") => settled_tool_state(false, &input, &state, &start, &end),
                        Some(status) => json!({"status": if status == "pending" { "pending" } else { "running" }, "input": input}),
                        None => json!({"status": "running", "input": input}),
                    };
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                    frames.push(tool_part(&session, &id, call_id, name, tool_state));
                }
                _ => {}
            }
        }
    }
    frames
}

/// 2.x permission rules: `{action, resource, effect}` and `shell` where 1.x
/// said `{permission, pattern, action}` and `bash`.
pub(crate) fn permission_rules(rules: &Value) -> Value {
    Value::Array(
        rules
            .as_array()
            .into_iter()
            .flatten()
            .map(|rule| {
                let action = match rule.get("permission").and_then(Value::as_str) {
                    Some("bash") => "shell",
                    Some(other) => other,
                    None => "*",
                };
                json!({
                    "action": action,
                    "resource": rule.get("pattern").cloned().unwrap_or(json!("*")),
                    "effect": rule.get("action").cloned().unwrap_or(json!("ask")),
                })
            })
            .collect(),
    )
}

/// The 2.x catalog: integrations hold credentials and auth methods, providers
/// are what those integrations made available, and models name their provider.
pub(crate) fn parse_catalog(
    integrations: &Value,
    providers: &Value,
    models: &Value,
    default_model: Option<&Value>,
    executable_path: &str,
    version: &str,
) -> Result<OpenCodeCatalog, String> {
    let integrations = integrations
        .as_array()
        .ok_or("OpenCode returned no integration list")?;
    let providers = providers.as_array().ok_or("OpenCode returned no provider list")?;
    let models = models.as_array().ok_or("OpenCode returned no model list")?;
    let integration = |id: &str| {
        integrations
            .iter()
            .find(|item| item.get("id").and_then(Value::as_str) == Some(id))
    };
    let default = default_model.and_then(|model| {
        Some((
            model.get("providerID")?.as_str()?.to_owned(),
            model.get("id")?.as_str()?.to_owned(),
        ))
    });
    let mut covered = HashSet::new();
    let mut normalized = Vec::new();
    for provider in providers {
        let Some(id) = provider.get("id").and_then(Value::as_str) else {
            continue;
        };
        let integration_id = provider
            .get("integrationID")
            .and_then(Value::as_str)
            .unwrap_or(id);
        covered.insert(integration_id.to_owned());
        covered.insert(id.to_owned());
        let owner = integration(integration_id);
        let mut provider_models = models
            .iter()
            .filter(|model| model.get("providerID").and_then(Value::as_str) == Some(id))
            .filter(|model| model.get("enabled").and_then(Value::as_bool) != Some(false))
            .filter_map(|model| normalize_model(id, model))
            .collect::<Vec<_>>();
        provider_models.sort_by(|left, right| left.id.cmp(&right.id));
        normalized.push(OpenCodeProvider {
            id: id.to_owned(),
            name: provider
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(id)
                .to_owned(),
            connected: true,
            source: owner.map(connection_source).or_else(|| Some("config".into())),
            environment_variables: owner.map(environment_variables).unwrap_or_default(),
            default_model: default
                .as_ref()
                .filter(|(provider_id, _)| provider_id == id)
                .map(|(provider_id, model)| format!("{provider_id}/{model}")),
            auth_methods: owner.map(auth_methods).unwrap_or_default(),
            models: provider_models,
        });
    }
    for item in integrations {
        let Some(id) = item.get("id").and_then(Value::as_str) else {
            continue;
        };
        if covered.contains(id) {
            continue;
        }
        normalized.push(OpenCodeProvider {
            id: id.to_owned(),
            name: item.get("name").and_then(Value::as_str).unwrap_or(id).to_owned(),
            connected: false,
            source: None,
            environment_variables: environment_variables(item),
            default_model: None,
            auth_methods: auth_methods(item),
            models: Vec::new(),
        });
    }
    normalized.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(OpenCodeCatalog {
        executable_path: executable_path.to_owned(),
        version: version.to_owned(),
        providers: normalized,
    })
}

fn connection_source(integration: &Value) -> String {
    let connections = integration
        .get("connections")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let has = |kind: &str| {
        connections
            .iter()
            .any(|connection| connection.get("type").and_then(Value::as_str) == Some(kind))
    };
    if has("credential") {
        "api".into()
    } else if has("env") {
        "env".into()
    } else {
        "config".into()
    }
}

fn environment_variables(integration: &Value) -> Vec<String> {
    integration
        .get("methods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|method| method.get("type").and_then(Value::as_str) == Some("env"))
        .flat_map(|method| method.get("names").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect()
}

/// Only the methods Bridge can drive: a pasted key (1.x called it `api`) and
/// a browser sign-in. Environment and command methods are configured outside.
fn auth_methods(integration: &Value) -> Vec<OpenCodeAuthMethod> {
    integration
        .get("methods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|method| {
            let (kind, fallback) = match method.get("type").and_then(Value::as_str)? {
                "key" => ("api", "API key"),
                "oauth" => ("oauth", "Sign in"),
                _ => return None,
            };
            Some(OpenCodeAuthMethod {
                kind: kind.into(),
                label: method
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or(fallback)
                    .to_owned(),
            })
        })
        .collect()
}

fn normalize_model(provider_id: &str, model: &Value) -> Option<OpenCodeModel> {
    let model_id = model.get("id")?.as_str()?.to_owned();
    let variants = model
        .get("variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|variant| variant.get("id").and_then(Value::as_str))
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let inputs = model
        .pointer("/capabilities/input")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let cost = model.pointer("/cost/0");
    Some(OpenCodeModel {
        id: format!("{provider_id}/{model_id}"),
        provider_id: provider_id.to_owned(),
        label: model
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&model_id)
            .to_owned(),
        reasoning: model.pointer("/compatibility/reasoningField").is_some() || !variants.is_empty(),
        variants,
        tool_call: model
            .pointer("/capabilities/tools")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        attachment: inputs.iter().any(|input| input.as_str() != Some("text")),
        context_window: model.pointer("/limit/context").and_then(Value::as_u64),
        output_limit: model.pointer("/limit/output").and_then(Value::as_u64),
        input_cost: cost.and_then(|cost| cost.get("input")).and_then(Value::as_f64),
        output_cost: cost.and_then(|cost| cost.get("output")).and_then(Value::as_f64),
        model_id,
    })
}

/// The integration that owns a provider's credentials: providers name it,
/// and an integration that has not produced a provider yet is its own id.
pub(crate) fn integration_for(providers: &Value, provider_id: &str) -> String {
    providers
        .as_array()
        .into_iter()
        .flatten()
        .find(|provider| provider.get("id").and_then(Value::as_str) == Some(provider_id))
        .and_then(|provider| provider.get("integrationID").and_then(Value::as_str))
        .unwrap_or(provider_id)
        .to_owned()
}

/// Stored credentials (not environment variables) an integration holds.
pub(crate) fn stored_credentials(integrations: &Value, integration_id: &str) -> Vec<String> {
    integrations
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item.get("id").and_then(Value::as_str) == Some(integration_id))
        .flat_map(|item| item.get("connections").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter(|connection| connection.get("type").and_then(Value::as_str) == Some("credential"))
        .filter_map(|connection| connection.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{normalize_opencode_message_with_state, NormalizedEvent, OpenCodeStreamState};

    fn fixture() -> Vec<Value> {
        serde_json::from_str(include_str!(
            "../../../testing/fixtures/opencode-v2-sse-turn.json"
        ))
        .expect("fixture is valid JSON")
    }

    fn replay(frames: &[Value]) -> (Vec<NormalizedEvent>, PendingRequests) {
        let pending = PendingRequests::default();
        let mut translator = Translator::new(pending.clone());
        let mut state = OpenCodeStreamState::default();
        let events = frames
            .iter()
            .flat_map(|frame| translator.translate(frame))
            .flat_map(|frame| normalize_opencode_message_with_state(&frame, &mut state))
            .collect();
        (events, pending)
    }

    fn kinds(events: &[NormalizedEvent]) -> Vec<&str> {
        events.iter().map(|event| event.kind.as_str()).collect()
    }

    #[test]
    fn a_recorded_turn_streams_prose_tools_usage_and_completes() {
        let (events, _) = replay(&fixture());
        let kinds = kinds(&events);
        assert_eq!(kinds[..2], ["session.started", "turn.started"]);
        assert_eq!(kinds.last(), Some(&"turn.completed"));
        assert_eq!(kinds.iter().filter(|kind| **kind == "turn.started").count(), 1);
        let streamed = events
            .iter()
            .filter(|event| event.kind == "message.delta")
            .filter_map(|event| event.text.as_deref())
            .collect::<String>();
        assert_eq!(streamed, "I'll read the note first, then run the shell command.Read note.txt (\"hello world\") and wrote `hi` to out.txt successfully.");
        let replies = events
            .iter()
            .filter(|event| event.kind == "message.completed")
            .filter_map(|event| event.text.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(replies, [
            "I'll read the note first, then run the shell command.",
            "Read note.txt (\"hello world\") and wrote `hi` to out.txt successfully.",
        ]);
        assert!(events.iter().any(|event| event.kind == "reasoning.delta"));
        assert!(events.iter().any(|event| event.kind == "reasoning.completed"));

        let read = events
            .iter()
            .rfind(|event| event.kind == "tool.completed" && event.title.as_deref() == Some("read"))
            .expect("the read tool settles");
        assert_eq!(read.status.as_deref(), Some("completed"));
        assert!(read.text.as_deref().unwrap().contains("1: hello world"));
        assert_eq!(read.data["state"]["input"]["path"], "/private/tmp/oc2probe/note.txt");

        let shell = events
            .iter()
            .rfind(|event| event.kind == "command.completed")
            .expect("the shell tool is a command row");
        assert_eq!(shell.data["state"]["input"]["command"], "echo hi > out.txt");
        assert_eq!(shell.text.as_deref(), Some("(no output)"));

        let usage = events
            .iter()
            .filter(|event| event.kind == "usage.updated")
            .collect::<Vec<_>>();
        assert_eq!(usage.len(), 2, "one usage row per model step");
        assert_eq!(usage[0].data["usage"]["input_tokens"], 19804 + 730, "cache-inclusive");
        assert_eq!(usage[0].data["usage"]["cache_read_tokens"], 730);
        assert_eq!(usage[1].data["usage"]["output_tokens"], 20 + 46, "reasoning within output");
        assert_eq!(usage[1].data["usage"]["reasoning_tokens"], 46);
        assert_eq!(usage[0].data["model"], "minimax-m3.1-flash-free");
        assert_eq!(usage[0].data["provider"], "opencode-go");
        assert!(!kinds.contains(&"provider.unknown"));
    }

    #[test]
    fn failed_executions_fail_the_turn_and_interrupts_end_it() {
        let ses = "ses_1";
        let started = json!({"type":"session.execution.started","data":{"sessionID":ses}});
        let (events, _) = replay(&[
            started.clone(),
            json!({"type":"session.execution.failed","data":{"sessionID":ses,"error":{"type":"provider","message":"rate limited"}}}),
        ]);
        assert_eq!(kinds(&events), ["turn.started", "error", "turn.completed"]);
        assert_eq!(events[1].text.as_deref(), Some("rate limited"));
        assert_eq!(events[2].status.as_deref(), Some("failed"));

        let (events, _) = replay(&[
            started,
            json!({"type":"session.tool.failed","data":{"sessionID":ses,"assistantMessageID":"msg_1","id":"call_1","error":{"type":"aborted","message":"Tool execution interrupted"},"executed":false}}),
            json!({"type":"session.execution.interrupted","data":{"sessionID":ses,"reason":"user"}}),
        ]);
        assert_eq!(kinds(&events).last(), Some(&"turn.completed"));
        assert_eq!(events.last().unwrap().status.as_deref(), Some("completed"));
    }

    #[test]
    fn permissions_keep_their_tool_and_remember_the_owning_session() {
        let (events, pending) = replay(&[json!({"type":"permission.asked","data":{
            "id":"per_1","sessionID":"ses_child","action":"shell","resources":["echo perm > p.txt"],
            "save":["echo *"],"source":{"type":"tool","messageID":"msg_1","id":"call_1"}}})]);
        assert_eq!(kinds(&events), ["permission.requested"]);
        assert_eq!(events[0].item_id.as_deref(), Some("call_1"));
        assert_eq!(events[0].text.as_deref(), Some("echo perm > p.txt"));
        assert_eq!(events[0].data["requestId"], "per_1");
        assert_eq!(
            pending.lock().unwrap().get("per_1"),
            Some(&PendingRequest::Permission { session_id: "ses_child".into() })
        );
    }

    #[test]
    fn question_forms_render_as_question_cards_and_settle() {
        let form = json!({"type":"form.created","data":{"form":{
            "id":"frm_1","sessionID":"ses_1","title":"Questions",
            "metadata":{"kind":"question","tool":{"messageID":"msg_1","id":"call_q"}},
            "fields":[{"key":"q0","title":"Color preference","description":"Which color do you prefer?","type":"string",
                "options":[{"value":"Red","label":"Red","description":"You prefer red."},{"value":"Blue","label":"Blue","description":"You prefer blue."}],"custom":true}]}}});
        let (events, pending) = replay(&[
            form,
            json!({"type":"form.replied","data":{"id":"frm_1","sessionID":"ses_1","answer":{"q0":"Blue"}}}),
        ]);
        assert_eq!(kinds(&events), ["question.requested", "question.settled"]);
        assert_eq!(events[0].item_id.as_deref(), Some("call_q"));
        assert_eq!(events[0].title.as_deref(), Some("Color preference"));
        assert_eq!(events[0].text.as_deref(), Some("Which color do you prefer?"));
        assert_eq!(events[0].data["questions"][0]["options"][1]["label"], "Blue");
        assert_eq!(events[1].data["requestId"], "frm_1");
        assert!(pending.lock().unwrap().is_empty(), "a settled form is forgotten");
    }

    #[test]
    fn answers_become_typed_form_values() {
        let fields = vec![
            json!({"key":"color","type":"string","options":[{"value":"red","label":"Red"}]}),
            json!({"key":"tags","type":"multiselect","options":[{"value":"a","label":"Alpha"},{"value":"b","label":"Beta"}]}),
            json!({"key":"ok","type":"boolean"}),
            json!({"key":"count","type":"integer"}),
            json!({"key":"skipped","type":"string"}),
        ];
        let answer = form_answer(
            &fields,
            &json!([["Red"], ["Alpha", "Beta"], ["Yes"], ["3"], []]),
        )
        .unwrap();
        assert_eq!(answer, json!({"color":"red","tags":["a","b"],"ok":true,"count":3.0}));
        assert_eq!(
            form_answer(&fields[..1], &json!([["something else"]])).unwrap(),
            json!({"color":"something else"}),
            "a custom answer passes through"
        );
        assert!(form_answer(&fields[3..4], &json!([["many"]])).is_err());
    }

    #[test]
    fn subagent_sessions_are_tagged_and_never_end_the_root_turn() {
        let (events, _) = replay(&[
            json!({"type":"session.execution.started","data":{"sessionID":"ses_root"}}),
            json!({"type":"session.created","data":{"sessionID":"ses_kid","parentID":"ses_root","agent":"explore","slug":"x","version":"2.1.1"}}),
            json!({"type":"session.execution.started","data":{"sessionID":"ses_kid"}}),
            json!({"type":"session.step.started","data":{"sessionID":"ses_kid","assistantMessageID":"msg_k","agent":"explore","model":{"id":"m","providerID":"p"}}}),
            json!({"type":"session.text.started","data":{"sessionID":"ses_kid","assistantMessageID":"msg_k","ordinal":0}}),
            json!({"type":"session.text.ended","data":{"sessionID":"ses_kid","assistantMessageID":"msg_k","ordinal":0,"text":"found 3 files"}}),
            json!({"type":"session.execution.succeeded","data":{"sessionID":"ses_kid"}}),
        ]);
        assert_eq!(kinds(&events).iter().filter(|kind| **kind == "turn.completed").count(), 0);
        let reply = events.iter().find(|event| event.kind == "message.completed").unwrap();
        assert_eq!(reply.data["subagent"]["sessionId"], "ses_kid");
        assert_eq!(reply.data["subagent"]["agent"], "explore");
    }

    #[test]
    fn resync_snapshots_reuse_the_live_part_ids() {
        let message = json!({
            "id":"msg_1","type":"assistant","model":{"id":"m","providerID":"p"},
            "time":{"created":1,"completed":3},
            "content":[
                {"type":"reasoning","text":"thinking"},
                {"type":"text","text":"first"},
                {"type":"tool","id":"call_1","name":"read","state":{"status":"completed","input":{"path":"a"},"content":[{"type":"text","text":"body"}]}},
                {"type":"text","text":"second"}
            ]
        });
        let frames = message_snapshots("ses_1", &[json!({"id":"msg_0","type":"user","text":"hi"}), message]);
        let ids = frames
            .iter()
            .filter_map(|frame| frame.pointer("/properties/part/id").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(ids, ["msg_1:reasoning:0", "msg_1:text:0", "call_1", "msg_1:text:1"]);
        assert_eq!(frames[0]["type"], "message.updated");
        let mut state = OpenCodeStreamState::default();
        let events = frames
            .iter()
            .flat_map(|frame| normalize_opencode_message_with_state(frame, &mut state))
            .collect::<Vec<_>>();
        let replies = events
            .iter()
            .filter(|event| event.kind == "message.completed")
            .filter_map(|event| event.text.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(replies, ["first", "second"]);
        assert!(events.iter().any(|event| event.kind == "tool.completed" && event.text.as_deref() == Some("body")));
    }

    #[test]
    fn permission_rules_use_the_v2_vocabulary() {
        let rules = permission_rules(&json!([
            {"permission":"*","pattern":"*","action":"allow"},
            {"permission":"bash","pattern":"*","action":"ask"},
        ]));
        assert_eq!(rules, json!([
            {"action":"*","resource":"*","effect":"allow"},
            {"action":"shell","resource":"*","effect":"ask"},
        ]));
    }

    #[test]
    fn catalog_lists_available_providers_and_connectable_integrations() {
        let integrations = json!([
            {"id":"opencode-go","name":"OpenCode Go","methods":[{"type":"key"},{"type":"env","names":["OPENCODE_API_KEY"]}],
             "connections":[{"type":"credential","id":"cred_1","label":"API key","method":"key"}]},
            {"id":"anthropic","name":"Anthropic","methods":[{"type":"key"},{"type":"oauth","id":"max","label":"Claude Max"}],"connections":[]},
        ]);
        let providers = json!([
            {"id":"opencode-go","integrationID":"opencode-go","name":"OpenCode Go","activation":"auto","package":"x"},
            {"id":"ollama","name":"Ollama","activation":"enabled","package":"x"},
        ]);
        let models = json!([
            {"id":"kimi-k3","modelID":"kimi-k3","providerID":"opencode-go","name":"Kimi K3",
             "capabilities":{"tools":true,"input":["text","image"],"output":["text"]},
             "variants":[{"id":"low"},{"id":"high"}],"cost":[{"input":0.6,"output":2.5,"cache":{"read":0,"write":0}}],
             "limit":{"context":262144,"output":32768},"enabled":true,"status":"active"},
            {"id":"off","providerID":"opencode-go","enabled":false,"capabilities":{"tools":true,"input":["text"],"output":["text"]}},
            {"id":"llama","providerID":"ollama","name":"Llama","capabilities":{"tools":false,"input":["text"],"output":["text"]}},
        ]);
        let catalog = parse_catalog(
            &integrations,
            &providers,
            &models,
            Some(&json!({"id":"kimi-k3","providerID":"opencode-go"})),
            "/bin/opencode",
            "opencode v2.1.1",
        )
        .unwrap();
        let ids = catalog.providers.iter().map(|provider| provider.id.as_str()).collect::<Vec<_>>();
        assert_eq!(ids, ["ollama", "opencode-go", "anthropic"]);
        let go = &catalog.providers[1];
        assert!(go.connected);
        assert_eq!(go.source.as_deref(), Some("api"));
        assert_eq!(go.default_model.as_deref(), Some("opencode-go/kimi-k3"));
        assert_eq!(go.environment_variables, ["OPENCODE_API_KEY"]);
        assert_eq!(go.models.len(), 1, "disabled models are hidden");
        let kimi = &go.models[0];
        assert_eq!(kimi.id, "opencode-go/kimi-k3");
        assert_eq!(kimi.variants, ["low", "high"]);
        assert!(kimi.attachment && kimi.tool_call && kimi.reasoning);
        assert_eq!(kimi.output_cost, Some(2.5));
        let anthropic = &catalog.providers[2];
        assert!(!anthropic.connected);
        assert_eq!(
            anthropic.auth_methods,
            [
                OpenCodeAuthMethod { kind: "api".into(), label: "API key".into() },
                OpenCodeAuthMethod { kind: "oauth".into(), label: "Claude Max".into() },
            ]
        );
        assert_eq!(integration_for(&providers, "opencode-go"), "opencode-go");
        assert_eq!(integration_for(&providers, "anthropic"), "anthropic");
        assert_eq!(stored_credentials(&integrations, "opencode-go"), ["cred_1"]);
        assert!(stored_credentials(&integrations, "anthropic").is_empty());
    }
}
