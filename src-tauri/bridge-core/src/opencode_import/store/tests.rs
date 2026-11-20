use super::*;
use crate::opencode_import::fixture::{replied, said, Fixture, CANARY};
use serde_json::json;

fn read(fixture: &Fixture, id: &str) -> ParsedSession {
    let conn = open(&fixture.store).unwrap();
    let row = find_session(&conn, id).unwrap().expect("the session is in the store");
    read_session(
        &conn,
        &row,
        &SessionSource {
            store: &fixture.store,
            home: &fixture.home,
            source_ref: format!("{}#{id}", fixture.store.display()),
            label: format!("opencode.db/{id}"),
        },
    )
    .unwrap()
}

fn tool(name: &str, status: &str, input: Value, output: &str, metadata: Value) -> Value {
    json!({ "type": "tool", "id": format!("call-{name}"), "name": name, "state": {
        "status": status, "input": input, "content": [{ "type": "text", "text": output }], "metadata": metadata
    }, "time": { "created": 1 } })
}

fn assistant(content: Vec<Value>, variant: &str) -> Value {
    json!({ "time": { "created": 1 }, "agent": "build", "model": { "id": "glm-5.3", "providerID": "opencode-go", "variant": variant }, "content": content, "finish": "tool-calls" })
}

/// One 2.x session holding every shape the reader knows.
fn every_shape(fixture: &Fixture) {
    fixture.session("ses_all", None, "/work/app", Some("Fix the login"), 1_790_000_000_000);
    let png = "iVBORw0KGgo=";
    fixture.row("ses_all", 1, "system", &json!({ "text": "instructions", "metadata": { "notice": "instructions" } }));
    fixture.row("ses_all", 2, "user", &json!({ "time": {}, "text": "fix the login", "files": [{ "mime": "image/png", "name": "shot.png", "source": { "type": "inline" }, "data": png }] }));
    fixture.row("ses_all", 3, "assistant", &assistant(vec![
        json!({ "type": "reasoning", "text": "Looking at the form." }),
        tool("shell", "completed", json!({ "command": "cargo test" }), "ok", json!({ "exit": 0 })),
        tool("bash", "completed", json!({ "command": "false" }), "boom", json!({ "exit": 1 })),
        tool("read", "completed", json!({ "path": "/work/app/src/login.rs" }), "fn login() {}", json!({})),
        tool("edit", "completed", json!({ "filePath": "/work/app/src/login.rs", "oldString": "a", "newString": "b" }), "Edited", json!({ "diff": "--- a/src/login.rs\n+++ b/src/login.rs\n-a\n+b\n" })),
        tool("write", "completed", json!({ "path": "/work/app/NOTES.md", "content": "notes" }), "Wrote", json!({})),
        tool("patch", "completed", json!({ "patchText": "*** Begin Patch" }), "Patched", json!({})),
        tool("grep", "completed", json!({ "pattern": "login", "path": "/work/app" }), "2 matches", json!({})),
        tool("glob", "completed", json!({ "pattern": "**/*.rs" }), "src/login.rs", json!({})),
        tool("webfetch", "completed", json!({ "url": "https://example.com" }), "page", json!({})),
        tool("websearch", "completed", json!({ "query": "rust login" }), "results", json!({})),
        tool("todowrite", "completed", json!({ "todos": [] }), "1 todo", json!({})),
        tool("bridge_visualize", "completed", json!({ "title": "chart" }), "drawn", json!({})),
        tool("task", "completed", json!({ "description": "Audit the tests", "prompt": "Read the tests", "subagent_type": "general" }), "All good", json!({ "sessionId": "ses_child" })),
        tool("subagent", "completed", json!({ "description": "Scan the docs", "prompt": "Scan", "agent": "explore", "background": true }), "Started", json!({ "sessionID": "ses_bg" })),
        json!({ "type": "tool", "id": "call-err", "name": "edit", "state": { "status": "error", "input": { "filePath": "/x" }, "error": { "type": "tool.execution", "message": "Could not find oldString" } } }),
        json!({ "type": "text", "text": "Fixed the login." }),
        json!({ "type": "hologram", "text": "?" }),
    ], "max"));
    fixture.row("ses_all", 4, "synthetic", &json!({ "metadata": { "source": "subagent", "childID": "ses_bg", "agent": "explore", "state": "completed" }, "text": "The docs are current.", "description": "Scan the docs" }));
    fixture.row("ses_all", 5, "synthetic", &json!({ "metadata": { "source": "shell", "shellID": "sh_1", "state": "completed", "exit": 2 }, "text": "build failed", "description": "npm run build" }));
    fixture.row("ses_all", 6, "synthetic", &json!({ "text": "Continue if you have next steps." }));
    fixture.row("ses_all", 7, "idle", &json!({ "outcome": "succeeded" }));
    fixture.row("ses_all", 8, "compaction", &json!({ "status": "completed", "reason": "auto", "summary": "We fixed the login." }));
    fixture.row("ses_all", 9, "location-switched", &json!({ "location": { "directory": "/work/app" } }));
    fixture.row("ses_all", 10, "teleport", &json!({}));
    fixture.raw_row("ses_all", 11, "user", "{not json");
    fixture.row("ses_all", 12, "user", &said("thanks"));
}

fn tools(parsed: &ParsedSession) -> Vec<&BundleTool> {
    parsed.chat.entries.iter().filter_map(|entry| entry.tool.as_ref()).collect()
}

#[test]
fn a_2x_session_comes_over_as_opencode_showed_it() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    every_shape(&fixture);

    let parsed = read(&fixture, "ses_all");
    let chat = &parsed.chat;

    assert_eq!(chat.key, "opencode:ses_all");
    assert_eq!((chat.title.as_deref(), chat.title_source.clone()), (Some("Fix the login"), TitleSource::Native));
    assert_eq!((chat.cwd.as_deref(), chat.model.as_deref(), chat.effort.as_deref()), (Some("/work/app"), Some("opencode-go/glm-5.3"), Some("max")));
    assert_eq!(chat.native_ref.as_ref().map(|native| native.session_id.as_str()), Some("ses_all"));
    assert_eq!(parsed.typed_messages, 2, "the two messages typed; system rows and nudges are not");

    let user = &chat.entries[0];
    assert_eq!((user.kind.clone(), user.text.as_deref()), (BundleEntryKind::UserMessage, Some("fix the login")));
    assert_eq!(user.attachments.len(), 1);
    assert_eq!((user.attachments[0].media_type.as_str(), user.attachments[0].data_base64.as_deref()), ("image/png", Some("iVBORw0KGgo=")));
    assert_eq!((chat.entries[1].kind.clone(), chat.entries[1].text.as_deref()), (BundleEntryKind::Reasoning, Some("Looking at the form.")));
    let reply = chat.entries.iter().find(|entry| entry.kind == BundleEntryKind::AssistantMessage).unwrap();
    assert_eq!(reply.text.as_deref(), Some("Fixed the login."));
    let compaction = chat.entries.iter().find(|entry| entry.kind == BundleEntryKind::CompactionBoundary).unwrap();
    assert_eq!(compaction.text.as_deref(), Some("We fixed the login."));
    assert_eq!(chat.entries.last().unwrap().text.as_deref(), Some("thanks"));

    let skipped: Vec<(&str, u32)> = parsed.skipped.iter().map(|(reason, count)| (reason.as_str(), *count)).collect();
    assert_eq!(skipped, vec![("content:hologram", 1), ("row:teleport", 1), ("unreadable_row", 1)], "bookkeeping is not a loss");
}

#[test]
fn each_2x_tool_is_the_row_bridge_draws_for_it_live() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    every_shape(&fixture);

    let parsed = read(&fixture, "ses_all");
    let tools = tools(&parsed);

    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, vec![
        "bash", "bash", "read", "edit", "write", "apply_patch", "grep", "glob", "webfetch", "websearch", "todowrite",
        "bridge_visualize", "task", "task", "edit", "task", "bash",
    ]);
    assert_eq!((tools[0].input["command"].as_str(), tools[0].output.as_deref(), tools[0].status), (Some("cargo test"), Some("ok"), ToolStatus::Completed));
    assert_eq!(tools[1].status, ToolStatus::Failed, "a non-zero exit failed");
    assert_eq!(tools[2].input["file_path"].as_str(), Some("/work/app/src/login.rs"));
    assert!(tools[3].output.as_deref().unwrap().starts_with("--- a/src/login.rs"), "an edit carries its diff");
    assert_eq!(tools[3].input["file_path"].as_str(), Some("/work/app/src/login.rs"));
    assert_eq!(tools[4].input["file_path"].as_str(), Some("/work/app/NOTES.md"));
    assert_eq!((tools[12].input["description"].as_str(), tools[12].input["subagent_type"].as_str()), (Some("Audit the tests"), Some("general")));
    assert_eq!(tools[13].input["subagent_type"].as_str(), Some("explore"), "a subagent call names its agent");
}

#[test]
fn errors_and_background_results_keep_their_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    every_shape(&fixture);

    let parsed = read(&fixture, "ses_all");
    let tools = tools(&parsed);

    assert_eq!((tools[14].status, tools[14].output.as_deref()), (ToolStatus::Failed, Some("Could not find oldString")), "an errored tool failed with why");
    assert_eq!((tools[15].output.as_deref(), tools[15].status), (Some("The docs are current."), ToolStatus::Completed), "a background subagent's result");
    assert_eq!((tools[16].input["command"].as_str(), tools[16].status), (Some("npm run build"), ToolStatus::Failed), "a background command's result");
}

#[test]
fn a_subagent_session_names_its_parent_and_an_untitled_one_derives_a_title() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    fixture.session("ses_child", Some("ses_parent"), "/work/app", Some("Child session - 2026-11-01T10:00:00.000Z"), 2);
    fixture.row("ses_child", 1, "user", &said("check the cookie"));
    fixture.row("ses_child", 2, "assistant", &replied("The cookie is fine."));
    fixture.archive("ses_child");

    let chat = read(&fixture, "ses_child").chat;

    assert_eq!(chat.parent_native_key.as_deref(), Some("opencode:ses_parent"));
    assert_eq!(chat.parent_chat_key.as_deref(), Some("opencode:ses_parent"));
    assert_eq!(chat.agent_role, Some(AgentRole::Subagent));
    assert_eq!((chat.title, chat.title_source), (None, TitleSource::Derived), "OpenCode's placeholder is not a title");
    assert!(chat.archived);
    assert_eq!(chat.effort, None, "the default variant is no effort");
}

#[test]
fn a_1x_session_reads_from_its_messages_and_parts() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::v1_only(dir.path());
    fixture.v1_session("ses_old", None, "/work/api", "Add a route", 10);
    fixture.v1_message("ses_old", "msg_1", 10, &json!({ "role": "user", "time": { "created": 10 } }), &[
        json!({ "type": "text", "text": "add a route" }),
        json!({ "type": "text", "text": "<file>contents</file>", "synthetic": true }),
        json!({ "type": "file", "mime": "image/png", "filename": "a.png", "url": "data:image/png;base64,iVBORw0KGgo=" }),
    ]);
    fixture.v1_message("ses_old", "msg_2", 11, &json!({ "role": "assistant", "modelID": "big-pickle", "providerID": "opencode", "variant": "high", "time": { "created": 11 } }), &[
        json!({ "type": "step-start" }),
        json!({ "type": "reasoning", "text": "Routes live in api.rs." }),
        json!({ "type": "tool", "callID": "call_1", "tool": "bash", "state": { "status": "completed", "input": { "command": "ls" }, "output": "api.rs", "metadata": { "exit": 0 } } }),
        json!({ "type": "tool", "callID": "call_2", "tool": "edit", "state": { "status": "error", "input": { "filePath": "/work/api/api.rs" }, "error": "no match" } }),
        json!({ "type": "tool", "callID": "call_3", "tool": "read", "state": { "status": "running", "input": { "filePath": "/work/api/api.rs" } } }),
        json!({ "type": "text", "text": "Added the route." }),
        json!({ "type": "step-finish", "reason": "stop" }),
        json!({ "type": "patch", "files": [] }),
        json!({ "type": "mirage" }),
    ]);
    fixture.v1_message("ses_old", "msg_3", 12, &json!({ "role": "user", "time": { "created": 12 } }), &[json!({ "type": "compaction", "auto": true })]);
    fixture.v1_message("ses_old", "msg_4", 13, &json!({ "role": "assistant", "summary": true, "modelID": "big-pickle", "time": { "created": 13 } }), &[json!({ "type": "text", "text": "We added a route." })]);

    let parsed = read(&fixture, "ses_old");
    let chat = &parsed.chat;
    let kinds: Vec<BundleEntryKind> = chat.entries.iter().map(|entry| entry.kind.clone()).collect();

    assert_eq!(kinds, vec![
        BundleEntryKind::UserMessage,
        BundleEntryKind::Reasoning,
        BundleEntryKind::ToolCall,
        BundleEntryKind::ToolCall,
        BundleEntryKind::ToolCall,
        BundleEntryKind::AssistantMessage,
        BundleEntryKind::CompactionBoundary,
    ]);
    assert_eq!(chat.entries[0].text.as_deref(), Some("add a route"), "OpenCode's own words are not the person's");
    assert_eq!(chat.entries[0].attachments[0].data_base64.as_deref(), Some("iVBORw0KGgo="));
    let tools = tools(&parsed);
    assert_eq!((tools[0].name.as_str(), tools[0].output.as_deref(), tools[0].status), ("bash", Some("api.rs"), ToolStatus::Completed));
    assert_eq!((tools[1].status, tools[1].output.as_deref()), (ToolStatus::Failed, Some("no match")));
    assert_eq!(tools[2].status, ToolStatus::Unknown, "a call cut off mid-run has no result");
    assert_eq!(chat.entries[6].text.as_deref(), Some("We added a route."), "the summary is the compaction's text");
    assert_eq!((chat.model.as_deref(), chat.effort.as_deref()), (Some("opencode/big-pickle"), Some("high")), "the model as Bridge names it");
    assert_eq!(parsed.typed_messages, 1);
    assert_eq!(parsed.skipped.get("part:mirage"), Some(&1));
}

#[test]
fn a_session_in_both_generations_is_listed_once_from_2x() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    fixture.v1_session("ses_both", None, "/work/app", "Old copy", 1);
    fixture.session("ses_both", None, "/work/app", Some("Upgraded"), 1);
    fixture.v1_session("ses_only_v1", None, "/work/api", "Only in 1.x", 2);
    fixture.row("ses_both", 1, "user", &said("hello"));

    let conn = open(&fixture.store).unwrap();
    let rows = list_sessions(&conn).unwrap();

    let listed: Vec<(&str, Generation)> = rows.iter().map(|row| (row.id.as_str(), row.generation)).collect();
    assert_eq!(listed, vec![("ses_both", Generation::V2), ("ses_only_v1", Generation::V1)]);
    assert!(rows[0].bytes > 0, "a session's size comes from its messages");
    assert_eq!(find_session(&conn, "ses_both").unwrap().unwrap().title.as_deref(), Some("Upgraded"));
    assert_eq!(list_session_heads(&conn).unwrap().len(), 2);
}

#[test]
fn nothing_read_ever_carries_the_credential_tables() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    every_shape(&fixture);
    fixture.v1_session("ses_old", None, "/work/api", "Old", 3);

    let conn = open(&fixture.store).unwrap();
    let rows = list_sessions(&conn).unwrap();
    let heads = list_session_heads(&conn).unwrap();
    let chats: Vec<BundleChat> = rows.iter().map(|row| read(&fixture, &row.id).chat).collect();

    let everything = format!("{rows:?}{heads:?}{}", serde_json::to_string(&chats).unwrap());
    assert!(!everything.contains(CANARY));
}

#[test]
fn the_store_opens_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(dir.path());
    let conn = open(&fixture.store).unwrap();
    let write = conn.execute("DELETE FROM session_v2", []);
    assert!(write.is_err(), "a read-only connection cannot write");
}
