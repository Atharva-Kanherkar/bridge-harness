//! What a model reads from one history entry, estimated once when the entry
//! is written.
//!
//! `session_entries.token_estimate` was empty on every row, so nothing could
//! say which kinds of entry fill a window. This fills it at write time with
//! the method the prompt compiler already names, [`METHOD`]: UTF-8 bytes of
//! the text divided by four, rounded up. No tokenizer.
//!
//! Only the text a model reads is counted: message prose, a tool call's
//! input, a tool's output, and the summaries Bridge itself injects. The JSON
//! envelope, ids, `providerMeta`, `_bridgeRepoState` and image attachments are
//! not. Every adapter shapes its payload differently, so each kind lists the
//! fields it reads in order and takes the first one present.
//!
//! Each row is estimated on its own. One provider item can be written more
//! than once (OpenCode stores every tool-part update as a start, Claude a
//! `preparing` and a `running` start), so a report summing a kind counts each
//! `provider_event_id` once, with its largest estimate. Deduplicating here
//! would cost a query per frame.
//!
//! `None` means "no model-readable text known for this entry": either the
//! kind is bookkeeping (turn boundaries, usage, status, streaming deltas whose
//! terminal event carries the whole text) or none of the kind's fields were
//! present. Readers treat `NULL` as unknown, as they must for rows written
//! before this existed.
//!
//! This runs inside the append transaction for every adapter frame, so it is
//! a length computation over the payload already in hand and nothing more: no
//! query, no branch load, and JSON inputs are sized by a counting writer
//! rather than serialized into a buffer.

use serde_json::Value;
use std::io;

/// The estimate's method label. Shared with prompt accounting so the two
/// numbers are comparable.
pub const METHOD: &str = crate::prompt_compiler::TOKEN_ESTIMATE_SOURCE;

/// A tool call's input: Claude `input`, OpenCode `state.input`, Codex MCP
/// `arguments`, file-change `changes`, shell `command`, web search `query`,
/// and Bridge's own normalized `patch` last.
const INPUT_FIELDS: [&str; 7] = [
    "/data/input",
    "/data/state/input",
    "/data/arguments",
    "/data/changes",
    "/data/command",
    "/data/query",
    "/data/patch",
];

/// Estimated tokens a model reads from an entry of `kind` with `payload`.
pub fn estimate(kind: &str, payload: &Value) -> Option<i64> {
    model_readable_bytes(kind, payload)
        .map(|bytes| i64::try_from(bytes.div_ceil(4)).unwrap_or(i64::MAX))
}

/// UTF-8 bytes of the text a model reads from this entry, or `None` when the
/// kind carries none or its shape is unknown.
pub fn model_readable_bytes(kind: &str, payload: &Value) -> Option<usize> {
    match kind {
        "user.message" | "assistant.message" | "reasoning.completed" | "session.context"
        | "delegation.result" => first_text(payload, &["/text", "/data/text"]),
        // The call's input is counted where the call starts and its output
        // where it completes, so neither is counted twice. Claude reattaches
        // the command to the completion; that copy is ignored.
        "tool.started" | "command.started" | "file_change.started" => {
            // Claude writes a start at `content_block_start`, while the model
            // is still writing the arguments (`{}` so far), and another when
            // they are complete. Only the second carries the input.
            if payload.pointer("/data/phase").and_then(Value::as_str) == Some("preparing") {
                return None;
            }
            first_value(payload, &INPUT_FIELDS)
        }
        "tool.completed" | "command.completed" | "file_change.completed" => tool_output(payload),
        "checkpoint" | "compaction" => {
            sum_fields(payload, &["summary", "decisions", "filesTouched", "openWork"])
        }
        "branch.summary" => sum_fields(payload, &["summary"]),
        "handoff.brief" => first_text(payload, &["/text", "/summary"]),
        "worker.result" => sum_fields(
            payload,
            &[
                "summary",
                "suggestedNextAction",
                "decisions",
                "risks",
                "remainingWork",
                "filesChanged",
            ],
        ),
        _ => None,
    }
}

/// Root `text` is the adapters' normalized output. Without it, the raw shape:
/// Codex `aggregatedOutput`, OpenCode `state.output`, a plain `output`, Claude
/// `content` blocks, Codex MCP `result`.
fn tool_output(payload: &Value) -> Option<usize> {
    if let Some(bytes) = first_text(
        payload,
        &["/text", "/data/aggregatedOutput", "/data/state/output"],
    ) {
        return Some(bytes);
    }
    if let Some(output) = present(payload.pointer("/data/output")) {
        return Some(value_bytes(output));
    }
    if let Some(content) = present(payload.pointer("/data/content")) {
        return Some(content_bytes(content));
    }
    present(payload.pointer("/data/result")).map(|result| match result.get("content") {
        Some(content @ Value::Array(_)) => content_bytes(content),
        _ => value_bytes(result),
    })
}

/// A content field: a string, or blocks whose `text` is what the model reads
/// (an image block has none and counts zero).
fn content_bytes(content: &Value) -> usize {
    match content {
        Value::String(text) => text.len(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .map(str::len)
            .sum(),
        other => json_bytes(other),
    }
}

fn first_text(payload: &Value, pointers: &[&str]) -> Option<usize> {
    pointers
        .iter()
        .find_map(|pointer| payload.pointer(pointer)?.as_str())
        .map(str::len)
}

fn first_value(payload: &Value, pointers: &[&str]) -> Option<usize> {
    pointers
        .iter()
        .find_map(|pointer| present(payload.pointer(pointer)))
        .map(value_bytes)
}

/// Strings by length and lists by their items; `None` only when every field
/// is absent.
fn sum_fields(payload: &Value, fields: &[&str]) -> Option<usize> {
    let mut found = false;
    let mut bytes = 0usize;
    for field in fields {
        let Some(value) = present(payload.get(*field)) else {
            continue;
        };
        found = true;
        bytes = bytes.saturating_add(match value {
            Value::Array(items) => items.iter().map(value_bytes).sum(),
            other => value_bytes(other),
        });
    }
    found.then_some(bytes)
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

fn value_bytes(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        other => json_bytes(other),
    }
}

/// Compact JSON length without building the string: a model receives a tool
/// input as JSON, so its serialized size is the size it reads.
fn json_bytes(value: &Value) -> usize {
    let mut counter = ByteCounter(0);
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => counter.0,
        // Writing to a counter cannot fail and a `Value` always serializes;
        // if it ever did, the count so far is still a lower bound.
        Err(_) => counter.0,
    }
}

struct ByteCounter(usize);

impl io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tokens(bytes: usize) -> Option<i64> {
        Some(bytes.div_ceil(4) as i64)
    }

    #[test]
    fn prose_kinds_count_only_their_text() {
        let payload = json!({
            "protocolVersion": 1,
            "itemId": "msg-1",
            "role": "user",
            "text": "x".repeat(40),
            "traceId": "00000000-0000-0000-0000-000000000000",
            "providerMeta": {"raw": "y".repeat(4_000)},
            "_bridgeRepoState": {"status": "clean", "branch": "z".repeat(4_000)},
            "data": {"attachments": [{"type": "image", "data": "a".repeat(40_000)}]},
        });
        assert_eq!(estimate("user.message", &payload), Some(10));
        assert_eq!(estimate("assistant.message", &json!({"text": "hello"})), Some(2));
        assert_eq!(estimate("session.context", &json!({"text": "x".repeat(8)})), Some(2));
        // Codex reasoning without a summary has no readable text at the root;
        // OpenCode keeps it under data.
        assert_eq!(
            estimate("reasoning.completed", &json!({"text": null, "data": {"text": "x".repeat(12)}})),
            Some(3)
        );
        assert_eq!(estimate("reasoning.completed", &json!({"text": null, "data": {}})), None);
    }

    #[test]
    fn tool_input_and_output_are_counted_once_each() {
        let input = json!({"file_path": "/tmp/a.rs", "old_string": "a", "new_string": "b"});
        let started = json!({"title": "Edit", "data": {"id": "toolu_1", "name": "Edit", "input": input}});
        let input_bytes = serde_json::to_string(&input).unwrap().len();
        assert_eq!(estimate("tool.started", &started), tokens(input_bytes));

        let command = json!({"data": {"command": "cargo test --workspace", "cwd": "/repo"}});
        assert_eq!(estimate("command.started", &command), tokens("cargo test --workspace".len()));

        // Claude reattaches the command to the completion; only the output
        // counts there.
        let completed = json!({
            "text": "ok\n".repeat(10),
            "data": {"command": "x".repeat(4_000), "tool_use_id": "toolu_1", "content": "ok\n".repeat(10)},
        });
        assert_eq!(estimate("command.completed", &completed), tokens(30));
    }

    #[test]
    fn output_falls_back_through_adapter_shapes() {
        let codex = json!({"text": null, "data": {"aggregatedOutput": "x".repeat(20), "command": "ls"}});
        assert_eq!(estimate("command.completed", &codex), Some(5));
        let opencode = json!({"data": {"state": {"status": "completed", "output": "y".repeat(16), "input": {"a": 1}}}});
        assert_eq!(estimate("tool.completed", &opencode), Some(4));
        let blocks = json!({"data": {"content": [
            {"type": "text", "text": "x".repeat(8)},
            {"type": "image", "source": {"data": "z".repeat(10_000)}},
            {"type": "text", "text": "y".repeat(8)},
        ]}});
        assert_eq!(estimate("tool.completed", &blocks), Some(4));
        let mcp = json!({"data": {"result": {"content": [{"type": "text", "text": "w".repeat(12)}], "structured_content": null}}});
        assert_eq!(estimate("tool.completed", &mcp), Some(3));
    }

    #[test]
    fn bookkeeping_and_unknown_kinds_are_null() {
        let loud = json!({"text": "x".repeat(400), "data": {"input": "y".repeat(400)}});
        for kind in [
            "turn.started",
            "turn.completed",
            "usage.updated",
            "session.status",
            "session.started",
            "message.started",
            "reasoning.started",
            "command.output_delta",
            "message.delta",
            "subagent.status",
            "diff.updated",
            "approval.requested",
            "permission.denied",
            "question.requested",
            "error",
            "compaction.requested",
            "compaction.failed",
            "model_changed",
            "artifact.completed",
            "something.new",
        ] {
            assert_eq!(estimate(kind, &loud), None, "{kind}");
        }
    }

    #[test]
    fn a_preparing_tool_start_is_not_counted() {
        // The two rows Claude writes for one call, as stored in a live run.
        let preparing = json!({"status": "inProgress", "data": {"id": "toolu_1", "name": "Bash", "phase": "preparing", "input": {}}});
        let running = json!({"status": "inProgress", "data": {"id": "toolu_1", "name": "Bash", "phase": "running", "input": {"command": "ls"}}});
        assert_eq!(estimate("command.started", &preparing), None);
        assert_eq!(estimate("tool.started", &preparing), None);
        assert_eq!(estimate("command.started", &running), tokens(r#"{"command":"ls"}"#.len()));
    }

    #[test]
    fn recognised_kind_with_no_fields_is_null_and_empty_text_is_zero() {
        assert_eq!(estimate("tool.completed", &json!({"data": {"id": "x"}})), None);
        assert_eq!(estimate("tool.started", &json!({"data": {"id": "x"}})), None);
        assert_eq!(estimate("checkpoint", &json!({"schemaVersion": 1})), None);
        assert_eq!(estimate("assistant.message", &json!({"text": ""})), Some(0));
        assert_eq!(estimate("tool.completed", &json!({"text": ""})), Some(0));
    }

    #[test]
    fn typed_summaries_count_their_fields() {
        let checkpoint = json!({
            "schemaVersion": 1,
            "summary": "s".repeat(10),
            "decisions": ["d".repeat(3), "e".repeat(3)],
            "filesTouched": ["src/a.rs"],
            "openWork": [],
            "sourceAgent": "a".repeat(400),
            "firstRetainedEntryId": "b".repeat(400),
            "tokensBefore": 123_456,
        });
        assert_eq!(estimate("checkpoint", &checkpoint), tokens(10 + 6 + 8));
        assert_eq!(estimate("compaction", &checkpoint), tokens(10 + 6 + 8));
        assert_eq!(estimate("branch.summary", &json!({"summary": "x".repeat(9)})), Some(3));
        assert_eq!(
            estimate("handoff.brief", &json!({"summary": "short", "text": "x".repeat(16)})),
            Some(4)
        );
        let worker = json!({
            "status": "completed",
            "summary": "x".repeat(4),
            "decisions": ["y".repeat(4)],
            "risks": [],
            "remainingWork": ["z".repeat(4)],
            "filesChanged": ["w".repeat(4)],
            "childSessionId": "c".repeat(400),
        });
        assert_eq!(estimate("worker.result", &worker), Some(4));
    }

    #[test]
    fn json_input_size_matches_compact_serialization() {
        let value = json!({"nested": {"list": [1, "two", null, {"k": "ü"}]}, "s": "quote \" and \\"});
        assert_eq!(json_bytes(&value), serde_json::to_string(&value).unwrap().len());
    }

    #[test]
    fn method_is_the_prompt_compilers() {
        assert_eq!(METHOD, "bytes_div4_v1");
    }
}
