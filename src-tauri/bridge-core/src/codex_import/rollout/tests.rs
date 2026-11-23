use super::*;
use serde_json::json;
use std::io::Write;

fn record(kind: &str, payload: Value) -> Value {
    json!({ "timestamp": "2026-11-17T10:00:00Z", "ordinal": 0, "type": kind, "payload": payload })
}

fn completed(item: Value) -> Value {
    record("event_msg", json!({ "type": "item_completed", "item": item }))
}

fn meta(id: &str, cwd: &str, source: Value) -> Value {
    record("session_meta", json!({ "id": id, "cwd": cwd, "timestamp": "2026-11-17T09:59:00Z", "originator": "codex_exec", "source": source }))
}

fn write_rollout(dir: &Path, lines: &[Value]) -> std::path::PathBuf {
    let path = dir.join("sessions/2026/11/17/rollout-2026-11-17T10-00-00-t1.jsonl");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = std::fs::File::create(&path).unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
    path
}

fn read(path: &Path, titles: &HashMap<String, String>, archived: bool) -> ParsedRollout {
    read_rollout(&RolloutSource { path, home: path.parent().unwrap(), titles, archived, label: "sessions/rollout.jsonl".into() }).unwrap()
}

/// One of every item Codex shows, in order, as the person saw it.
#[test]
fn every_item_codex_shows_becomes_an_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rollout(dir.path(), &[
        meta("t1", "/work/app", json!("exec")),
        record("turn_context", json!({ "model": "gpt-6.1", "effort": "high" })),
        record("response_item", json!({ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "<environment_context>cwd</environment_context>" }] })),
        completed(json!({ "type": "UserMessage", "id": "u1", "content": [{ "type": "text", "text": "fix the failing test", "text_elements": [] }] })),
        completed(json!({ "type": "Reasoning", "id": "r0", "summary_text": [], "raw_content": [] })),
        completed(json!({ "type": "Reasoning", "id": "r1", "summary_text": ["Looking at the test first"], "raw_content": [] })),
        completed(json!({ "type": "CommandExecution", "id": "c1", "command": ["/bin/zsh", "-lc", "cargo test"], "cwd": "/work/app", "aggregated_output": "1 failed", "exit_code": 101, "status": "completed" })),
        completed(json!({ "type": "CommandExecution", "id": "c2", "command": ["rg", "fn main"], "cwd": "/work/app", "aggregated_output": "src/main.rs", "exit_code": 0, "status": "completed" })),
        completed(json!({ "type": "FileChange", "id": "f1", "changes": { "/work/app/src/lib.rs": { "type": "update", "unified_diff": "-a\n+b", "move_path": null } }, "status": "completed" })),
        completed(json!({ "type": "McpToolCall", "id": "m1", "server": "github", "tool": "get_issue", "arguments": { "number": 7 }, "result": { "content": [{ "type": "text", "text": "Issue 7" }], "isError": false }, "status": "completed" })),
        completed(json!({ "type": "ImageView", "id": "i1", "path": "/work/app/shot.png" })),
        completed(json!({ "type": "Extension", "id": "w1", "kind": "web_search", "query": "rust lifetimes", "results": [{ "title": "The Book", "url": "https://doc.rust-lang.org/book" }] })),
        completed(json!({ "type": "Extension", "id": "w2", "kind": "timing", "durationMs": 4 })),
        completed(json!({ "type": "SubAgentActivity", "id": "s1", "agent_thread_id": "t2", "agent_path": "x", "kind": "started" })),
        record("compacted", json!({ "message": "Fixed the off-by-one; tests pass." })),
        completed(json!({ "type": "AgentMessage", "id": "a1", "content": [{ "type": "text", "text": "Fixed and tested." }], "phase": "final_answer" })),
        record("event_msg", json!({ "type": "token_count" })),
        record("world_state", json!({ "full": true })),
    ]);

    let parsed = read(&path, &HashMap::new(), false);

    let kinds: Vec<BundleEntryKind> = parsed.chat.entries.iter().map(|entry| entry.kind).collect();
    use BundleEntryKind::*;
    assert_eq!(kinds, vec![UserMessage, Reasoning, ToolCall, ToolCall, ToolCall, ToolCall, ToolCall, ToolCall, CompactionBoundary, AssistantMessage]);
    let entries = &parsed.chat.entries;
    assert_eq!(entries[0].text.as_deref(), Some("fix the failing test"), "the injected environment block is not a message");
    assert_eq!(entries[1].text.as_deref(), Some("Looking at the test first"), "an encrypted reasoning item with no summary is left out");
    let tool = |index: usize| entries[index].tool.clone().unwrap();
    assert_eq!((tool(2).name.as_str(), tool(2).input["command"].as_str(), tool(2).status), ("shell", Some("cargo test"), ToolStatus::Failed));
    assert_eq!(tool(2).output.as_deref(), Some("1 failed"));
    assert_eq!((tool(3).input["command"].as_str(), tool(3).status), (Some("rg fn main"), ToolStatus::Completed));
    assert_eq!((tool(4).name.as_str(), tool(4).input["file_path"].as_str()), ("apply_patch", Some("/work/app/src/lib.rs")));
    assert_eq!(tool(4).output.as_deref(), Some("-a\n+b"));
    assert_eq!((tool(5).name.as_str(), tool(5).output.as_deref()), ("mcp__github__get_issue", Some("Issue 7")));
    assert_eq!(tool(6).name, "view_image");
    assert_eq!((tool(7).name.as_str(), tool(7).input["query"].as_str()), ("websearch", Some("rust lifetimes")));
    assert_eq!(entries[8].text.as_deref(), Some("Fixed the off-by-one; tests pass."));
    assert_eq!(entries[9].text.as_deref(), Some("Fixed and tested."));
    assert!(parsed.skipped.is_empty(), "bookkeeping is not a loss: {:?}", parsed.skipped);
    assert_eq!(parsed.typed_messages, 1);

    let chat = &parsed.chat;
    assert_eq!((chat.model.as_deref(), chat.effort.as_deref(), chat.cwd.as_deref()), (Some("gpt-6.1"), Some("high"), Some("/work/app")));
    assert_eq!(chat.native_ref.as_ref().map(|native| native.key()), Some("codex:t1".into()));
    assert_eq!((chat.harness.as_str(), chat.title_source, chat.archived), ("codex", TitleSource::Derived, false));
    assert!(chat.parent_chat_key.is_none() && chat.agent_role.is_none());
}

#[test]
fn codex_titles_archive_and_subagents_carry_over() {
    let dir = tempfile::tempdir().unwrap();
    let parent = json!({ "subagent": { "thread_spawn": { "parent_thread_id": "t0", "depth": 1 } } });
    let path = write_rollout(dir.path(), &[
        meta("t1", "/work/app", parent),
        completed(json!({ "type": "AgentMessage", "id": "a1", "content": [{ "type": "text", "text": "done" }] })),
    ]);
    let titles = HashMap::from([("t1".to_owned(), "Fix the login".to_owned())]);

    let parsed = read(&path, &titles, true);

    assert_eq!((parsed.chat.title.as_deref(), parsed.chat.title_source), (Some("Fix the login"), TitleSource::Native));
    assert!(parsed.chat.archived);
    assert_eq!(parsed.chat.parent_chat_key.as_deref(), Some("codex:t0"));
    assert_eq!(parsed.chat.parent_native_key.as_deref(), Some("codex:t0"), "found in the store when the parent was read in another batch");
    assert_eq!(parsed.chat.agent_role, Some(AgentRole::Subagent));
    assert_eq!(parsed.typed_messages, 0);
}

#[test]
fn what_bridge_cannot_carry_is_counted() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rollout(dir.path(), &[
        meta("t1", "/work/app", json!("exec")),
        record("mystery", json!({})),
        completed(json!({ "type": "Hologram", "id": "h1" })),
        record("event_msg", json!({ "type": "brand_new_event" })),
        json!("not an object"),
    ]);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("{not json\n");
    std::fs::write(&path, text).unwrap();

    let parsed = read(&path, &HashMap::new(), false);

    let skipped: Vec<(&str, u32)> = parsed.skipped.iter().map(|(reason, count)| (reason.as_str(), *count)).collect();
    assert_eq!(skipped, vec![("event:brand_new_event", 1), ("item:Hologram", 1), ("malformed_line", 1), ("record:mystery", 1), ("untyped_record", 1)]);
}

#[test]
fn the_first_record_is_read_on_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rollout(dir.path(), &[meta("t9", "/private/var/folders/x/T/probe", json!({ "subagent": { "thread_spawn": { "parent_thread_id": "t8" } } }))]);
    let meta = read_meta(&path).unwrap();
    assert_eq!((meta.thread_id.as_deref(), meta.cwd.as_deref(), meta.parent_thread_id.as_deref()), (Some("t9"), Some("/private/var/folders/x/T/probe"), Some("t8")));
}

#[test]
fn a_codex_command_shows_as_the_script_it_ran() {
    assert_eq!(command_line(&json!(["/bin/zsh", "-lc", "cargo test -p core"])), "cargo test -p core");
    assert_eq!(command_line(&json!(["bash", "-c", "ls"])), "ls");
    assert_eq!(command_line(&json!(["rg", "-n", "fn main"])), "rg -n fn main");
    assert_eq!(command_line(&json!("git status")), "git status");
}

#[test]
fn titles_come_from_the_session_index_newest_last() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("session_index.jsonl"),
        "{\"id\":\"t1\",\"thread_name\":\"Old\",\"updated_at\":\"a\"}\n{\"id\":\"t1\",\"thread_name\":\"New\",\"updated_at\":\"b\"}\nnot json\n",
    )
    .unwrap();
    assert_eq!(read_titles(dir.path()).get("t1").map(String::as_str), Some("New"));
}
