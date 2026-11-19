//! Codex rollouts (`<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-*.jsonl`, and
//! `archived_sessions/`) read into transfer-bundle chats, at the fidelity the
//! person saw them.
//!
//! Shapes were taken from a real store of 313 rollouts (codex-cli 0.159.3):
//! the first record is `session_meta`, and every rollout carries `event_msg`
//! `item_completed` events, the items Codex showed: `UserMessage`,
//! `AgentMessage`, `Reasoning`, `CommandExecution`, `FileChange`,
//! `McpToolCall`, `ImageView`, `Extension` (web search) and
//! `SubAgentActivity`. The chat is built from those. The model-level
//! `response_item` copy is not: it holds the injected `<environment_context>`
//! and AGENTS.md blocks as user messages, and an `exec` tool whose input is
//! JavaScript rather than the commands it ran.

use crate::import_bundle::{
    AgentRole, BundleChat, BundleEntry, BundleEntryKind, BundleTool, EntriesSource, NativeRef,
    TitleSource, ToolStatus,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

/// Rollouts larger than this are reported, not read. The reader streams, so
/// this is a guard, not the working limit.
pub const MAX_ROLLOUT_BYTES: u64 = 512 * 1024 * 1024;

/// One line larger than this is skipped and counted.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

/// Record types that are Codex's own bookkeeping, or a copy of what the
/// `item_completed` events already carry. Skipped without a count.
const BOOKKEEPING: &[&str] = &[
    "response_item",
    "turn_context",
    "world_state",
    "token_usage_record",
    "inter_agent_communication_metadata",
];

/// `event_msg` payloads other than `item_completed` that are bookkeeping.
const EVENT_BOOKKEEPING: &[&str] = &[
    "task_started",
    "task_complete",
    "token_count",
    "thread_settings_applied",
    "turn_aborted",
    "item_started",
    "agent_message_delta",
    "agent_reasoning_delta",
];

/// Item types that are status, not conversation.
const ITEM_BOOKKEEPING: &[&str] = &["SubAgentActivity", "ContextCompaction", "EnteredReviewMode", "ExitedReviewMode"];

/// What the first record says about a rollout, read without the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RolloutMeta {
    pub thread_id: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    /// The thread that spawned this one, for a subagent.
    pub parent_thread_id: Option<String>,
    pub agent_nickname: Option<String>,
    pub originator: Option<String>,
}

/// Read only the first record of a rollout.
pub fn read_meta(path: &Path) -> Option<RolloutMeta> {
    let mut reader = BufReader::new(File::open(path).ok()?);
    let mut buffer = Vec::new();
    reader.by_ref().take(MAX_LINE_BYTES as u64).read_until(b'\n', &mut buffer).ok()?;
    let record: Value = serde_json::from_slice(&buffer).ok()?;
    (record.get("type").and_then(Value::as_str) == Some("session_meta"))
        .then(|| meta_from(record.get("payload").unwrap_or(&Value::Null)))
}

fn meta_from(payload: &Value) -> RolloutMeta {
    let text = |key: &str| payload.get(key).and_then(Value::as_str).filter(|value| !value.is_empty()).map(str::to_owned);
    let spawned_by = payload
        .pointer("/source/subagent/thread_spawn/parent_thread_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    RolloutMeta {
        thread_id: text("id"),
        cwd: text("cwd"),
        started_at: text("timestamp"),
        parent_thread_id: spawned_by.or_else(|| text("parent_thread_id")),
        agent_nickname: text("agent_nickname"),
        originator: text("originator"),
    }
}

#[derive(Debug)]
pub struct ParsedRollout {
    pub chat: BundleChat,
    pub meta: RolloutMeta,
    /// Records and items Bridge does not carry, by reason.
    pub skipped: BTreeMap<String, u32>,
    /// Messages the person typed in Codex.
    pub typed_messages: u32,
}

/// What the reader has gathered so far.
#[derive(Debug, Default)]
struct Reading {
    entries: Vec<BundleEntry>,
    meta: RolloutMeta,
    skipped: BTreeMap<String, u32>,
    typed_messages: u32,
}

/// Where a rollout is read from, and what the reader needs besides the file.
pub struct RolloutSource<'a> {
    pub path: &'a Path,
    /// The Codex home it belongs to, recorded on the native reference.
    pub home: &'a Path,
    /// Codex's own titles, by thread id (`session_index.jsonl`).
    pub titles: &'a HashMap<String, String>,
    pub archived: bool,
    pub label: String,
}

pub fn read_rollout(source: &RolloutSource<'_>) -> std::io::Result<ParsedRollout> {
    let size = std::fs::metadata(source.path)?.len();
    if size > MAX_ROLLOUT_BYTES {
        return Err(std::io::Error::other(format!(
            "rollout is {size} bytes, over the {MAX_ROLLOUT_BYTES} byte import limit"
        )));
    }
    let mut parsed = Reading::default();
    let mut model = None;
    let mut effort = None;
    let mut last_at = None;
    let mut lines = BufReader::new(File::open(source.path)?);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = lines.by_ref().take(MAX_LINE_BYTES as u64 + 1).read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        if buffer.len() > MAX_LINE_BYTES && !buffer.ends_with(b"\n") {
            bump(&mut parsed.skipped, "oversized_line");
            let mut sink = Vec::new();
            lines.read_until(b'\n', &mut sink)?;
            continue;
        }
        let Ok(record) = serde_json::from_slice::<Value>(&buffer) else {
            if !buffer.iter().all(u8::is_ascii_whitespace) {
                bump(&mut parsed.skipped, "malformed_line");
            }
            continue;
        };
        let at = record.get("timestamp").and_then(Value::as_str).map(str::to_owned);
        if at.is_some() {
            last_at.clone_from(&at);
        }
        let payload = record.get("payload").unwrap_or(&Value::Null);
        match record.get("type").and_then(Value::as_str).unwrap_or("") {
            "session_meta" if parsed.meta.thread_id.is_none() => parsed.meta = meta_from(payload),
            "session_meta" => {}
            "turn_context" => {
                model = payload.get("model").and_then(Value::as_str).map(str::to_owned).or(model);
                effort = payload.get("effort").and_then(Value::as_str).map(str::to_owned).or(effort);
            }
            "compacted" => {
                let summary = payload.get("message").and_then(Value::as_str).unwrap_or("").trim();
                parsed.entries.push(entry(
                    &format!("compaction-{}", parsed.entries.len()),
                    at.or_else(|| last_at.clone()),
                    BundleEntryKind::CompactionBoundary,
                    Some(if summary.is_empty() { "Codex compacted the conversation".to_owned() } else { summary.to_owned() }),
                    None,
                ));
            }
            "event_msg" => match payload.get("type").and_then(Value::as_str).unwrap_or("") {
                "item_completed" => item(&mut parsed, payload.get("item").unwrap_or(&Value::Null), at.or_else(|| last_at.clone())),
                kind if EVENT_BOOKKEEPING.contains(&kind) => {}
                kind => bump(&mut parsed.skipped, &format!("event:{kind}")),
            },
            kind if BOOKKEEPING.contains(&kind) => {}
            "" => bump(&mut parsed.skipped, "untyped_record"),
            kind => bump(&mut parsed.skipped, &format!("record:{kind}")),
        }
    }
    let meta = &parsed.meta;
    let thread_id = meta.thread_id.clone().unwrap_or_default();
    let title = source.titles.get(&thread_id).filter(|title| !title.trim().is_empty()).cloned();
    let is_subagent = meta.parent_thread_id.is_some();
    let chat = BundleChat {
        key: if thread_id.is_empty() { source.path.to_string_lossy().into_owned() } else { format!("codex:{thread_id}") },
        cwd: meta.cwd.clone(),
        title_source: if title.is_some() { TitleSource::Native } else { TitleSource::Derived },
        title: title.or_else(|| is_subagent.then(|| meta.agent_nickname.clone()).flatten()),
        workspace_key: None,
        pinned: false,
        pull_requests: Vec::new(),
        legacy_native_id: None,
        legacy_content_hash: None,
        created_at: meta.started_at.clone(),
        updated_at: last_at,
        archived: source.archived,
        harness: "codex".into(),
        model,
        effort,
        native_ref: (!thread_id.is_empty()).then(|| NativeRef {
            harness: "codex".into(),
            session_id: thread_id.clone(),
            config_root: Some(source.home.to_string_lossy().into_owned()),
            transcript_path: Some(source.path.to_string_lossy().into_owned()),
        }),
        parent_chat_key: meta.parent_thread_id.as_ref().map(|parent| format!("codex:{parent}")),
        parent_native_key: meta.parent_thread_id.as_ref().map(|parent| format!("codex:{parent}")),
        agent_role: is_subagent.then_some(AgentRole::Subagent),
        entries_source: EntriesSource::Native,
        source_ref: source.path.to_string_lossy().into_owned(),
        source_label: source.label.clone(),
        entries: parsed.entries,
    };
    Ok(ParsedRollout { chat, meta: parsed.meta, skipped: parsed.skipped, typed_messages: parsed.typed_messages })
}

fn bump(skipped: &mut BTreeMap<String, u32>, reason: &str) {
    *skipped.entry(reason.to_owned()).or_insert(0) += 1;
}

fn entry(id: &str, at: Option<String>, kind: BundleEntryKind, text: Option<String>, tool: Option<BundleTool>) -> BundleEntry {
    BundleEntry { source_id: id.to_owned(), at: at.unwrap_or_default(), kind, text, tool, attachments: Vec::new() }
}

/// The text of a content list: `[{type, text}, ...]`.
fn content_text(content: &Value) -> String {
    content
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Codex wraps commands as `["bash", "-lc", "<script>"]`; show the script.
pub fn command_line(command: &Value) -> String {
    match command {
        Value::String(text) => text.clone(),
        Value::Array(parts) => {
            let words: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
            match words.as_slice() {
                [shell, flag, script] if matches!(*flag, "-lc" | "-c") && shell.ends_with("sh") => (*script).to_owned(),
                _ => words.join(" "),
            }
        }
        _ => String::new(),
    }
}

fn item(parsed: &mut Reading, item: &Value, at: Option<String>) {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .map_or_else(|| format!("item-{}", parsed.entries.len()), str::to_owned);
    let text_of = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    let tool = |name: String, input: Value, output: Option<String>, failed: bool| BundleTool {
        call_id: id.clone(),
        name,
        input,
        output,
        status: if failed { ToolStatus::Failed } else { ToolStatus::Completed },
    };
    let new = match kind {
        "UserMessage" => {
            let text = content_text(item.get("content").unwrap_or(&Value::Null));
            if text.trim().is_empty() {
                None
            } else {
                parsed.typed_messages += 1;
                Some(entry(&id, at, BundleEntryKind::UserMessage, Some(text), None))
            }
        }
        "AgentMessage" => {
            let text = content_text(item.get("content").unwrap_or(&Value::Null));
            (!text.trim().is_empty()).then(|| entry(&id, at, BundleEntryKind::AssistantMessage, Some(text), None))
        }
        "Reasoning" => {
            let summary: Vec<&str> = item
                .get("summary_text")
                .and_then(Value::as_array)
                .map(|parts| parts.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            // Most Codex reasoning is encrypted and has no summary: nothing
            // the person saw, so nothing to carry.
            let text = summary.join("\n");
            (!text.trim().is_empty()).then(|| entry(&id, at, BundleEntryKind::Reasoning, Some(text), None))
        }
        "CommandExecution" => {
            let command = command_line(item.get("command").unwrap_or(&Value::Null));
            let failed = item.get("exit_code").and_then(Value::as_i64).is_some_and(|code| code != 0)
                || matches!(item.get("status").and_then(Value::as_str), Some("failed" | "declined"));
            let output = item.get("aggregated_output").and_then(Value::as_str).map(str::to_owned);
            Some(entry(&id, at, BundleEntryKind::ToolCall, None, Some(tool("shell".into(), json!({ "command": command, "workdir": text_of("cwd") }), output, failed))))
        }
        "FileChange" => {
            let changes = item.get("changes").and_then(Value::as_object).cloned().unwrap_or_default();
            let files: Vec<Value> = changes
                .iter()
                .map(|(path, change)| json!({ "path": path, "kind": change.get("type").cloned().unwrap_or(Value::Null) }))
                .collect();
            let diff = changes
                .values()
                .filter_map(|change| change.get("unified_diff").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            let first = changes.keys().next().cloned().unwrap_or_default();
            let failed = matches!(item.get("status").and_then(Value::as_str), Some("failed" | "declined"));
            Some(entry(&id, at, BundleEntryKind::ToolCall, None, Some(tool("apply_patch".into(), json!({ "file_path": first, "files": files }), (!diff.is_empty()).then_some(diff), failed))))
        }
        "McpToolCall" => {
            let server = text_of("server");
            let name = text_of("tool");
            let result = item.get("result").unwrap_or(&Value::Null);
            let output = content_text(result.get("content").unwrap_or(&Value::Null));
            let failed = result.get("isError").and_then(Value::as_bool).unwrap_or(false)
                || matches!(item.get("status").and_then(Value::as_str), Some("failed"));
            Some(entry(&id, at, BundleEntryKind::ToolCall, None, Some(tool(format!("mcp__{server}__{name}"), item.get("arguments").cloned().unwrap_or(Value::Null), (!output.is_empty()).then_some(output), failed))))
        }
        "ImageView" => Some(entry(&id, at, BundleEntryKind::ToolCall, None, Some(tool("view_image".into(), json!({ "file_path": text_of("path") }), None, false)))),
        "Extension" => {
            let query = item.get("query").and_then(Value::as_str).map(str::to_owned);
            match query {
                Some(query) => {
                    let results: Vec<String> = item
                        .get("results")
                        .and_then(Value::as_array)
                        .map(|results| {
                            results
                                .iter()
                                .filter_map(|result| {
                                    let title = result.get("title").and_then(Value::as_str)?;
                                    let url = result.get("url").and_then(Value::as_str).unwrap_or("");
                                    Some(format!("{title} {url}").trim().to_owned())
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    Some(entry(&id, at, BundleEntryKind::ToolCall, None, Some(tool("websearch".into(), json!({ "query": query }), (!results.is_empty()).then(|| results.join("\n")), false))))
                }
                // A timing-only extension item carries nothing to show.
                None => None,
            }
        }
        kind if ITEM_BOOKKEEPING.contains(&kind) => None,
        "" => {
            bump(&mut parsed.skipped, "untyped_item");
            None
        }
        kind => {
            bump(&mut parsed.skipped, &format!("item:{kind}"));
            None
        }
    };
    if let Some(new) = new {
        parsed.entries.push(new);
    }
}

/// Codex's own titles, by thread id: `session_index.jsonl`, newest wins.
pub fn read_titles(home: &Path) -> HashMap<String, String> {
    let mut titles = HashMap::new();
    let Ok(file) = File::open(home.join("session_index.jsonl")) else {
        return titles;
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Map<String, Value>>(&line) else {
            continue;
        };
        if let (Some(id), Some(name)) = (
            record.get("id").and_then(Value::as_str),
            record.get("thread_name").and_then(Value::as_str),
        ) {
            titles.insert(id.to_owned(), name.to_owned());
        }
    }
    titles
}

#[cfg(test)]
#[path = "rollout/tests.rs"]
mod tests;
