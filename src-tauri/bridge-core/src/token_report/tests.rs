use super::*;
use serde_json::json;
use std::path::PathBuf;
use TokenClass::{Agent, Bridge, Conversation, Edit, Mcp, OtherTool, Read, Search, Shell, Web};

const RECENT: &str = "2026-06-10T12:00:00Z";
const OLD: &str = "2026-03-01T12:00:00Z";

/// A migrated store with one project and one workspace `w`.
struct World {
    _dir: tempfile::TempDir,
    path: PathBuf,
    db: Connection,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bridge.db");
    let db = crate::store::open(&path).unwrap();
    db.execute_batch(
        "INSERT INTO projects(id,name,path,created_at) VALUES('p','Demo','/tmp/token-report','2026-06-01T00:00:00Z');
         INSERT INTO workspaces(id,project_id,city,title,branch,path,status,created_at)
             VALUES('w','p','Kyoto','Task','main',NULL,'idle','2026-06-01T00:00:00Z');",
    )
    .unwrap();
    World { _dir: dir, path, db }
}

/// One history row to append.
struct Entry {
    kind: &'static str,
    payload: Value,
    estimate: Option<i64>,
    event: Option<String>,
    at: String,
}

fn row(kind: &'static str, payload: Value) -> Entry {
    Entry { kind, payload, estimate: None, event: None, at: RECENT.to_owned() }
}

fn msg(kind: &'static str, tokens: i64) -> Entry {
    row(kind, json!({"text": "hello"})).est(tokens)
}

impl Entry {
    fn est(mut self, tokens: i64) -> Self {
        self.estimate = Some(tokens);
        self
    }

    fn event(mut self, id: &str) -> Self {
        self.event = Some(id.to_owned());
        self
    }

    fn at(mut self, at: &str) -> Self {
        self.at = at.to_owned();
        self
    }
}

impl World {
    fn session(&self, id: &str, kind: &str, workspace: Option<&str>) {
        self.db
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,kind,depth)
                 VALUES(?1,?2,'claude',?3,'idle',?4,0)",
                params![id, workspace, format!("Chat {id}"), kind],
            )
            .unwrap();
    }

    /// A top-level orchestrator chat in the workspace.
    fn chat(&self, id: &str) {
        self.session(id, "orchestrator", Some("w"));
    }

    fn runtime(&self, id: &str, parent: &str) {
        self.db
            .execute(
                "INSERT INTO worker_runtime(session_id,parent_session_id,lifecycle_state,task_family,compatibility_key,updated_at)
                 VALUES(?1,?2,'idle','research','key',?3)",
                params![id, parent, RECENT],
            )
            .unwrap();
    }

    /// A Bridge worker: a child row with a worker runtime.
    fn worker(&self, id: &str, parent: &str) {
        self.session(id, "orchestrator", Some("w"));
        self.child_of(id, parent);
        self.runtime(id, parent);
    }

    fn child_of(&self, id: &str, parent: &str) {
        self.db
            .execute(
                "UPDATE sessions SET parent_session_id=?2, depth=1 WHERE id=?1",
                params![id, parent],
            )
            .unwrap();
    }

    /// One recent message.
    fn talk(&self, id: &str) {
        self.put(id, vec![msg("user.message", 1)]);
    }

    /// Appends rows after the last row of the session, each one the child of
    /// the row before it.
    fn put(&self, session: &str, entries: Vec<Entry>) -> Vec<String> {
        let (mut sequence, mut parent): (i64, Option<String>) = self
            .db
            .query_row(
                "SELECT COALESCE(MAX(sequence),0),
                        (SELECT id FROM session_entries WHERE session_id=?1 ORDER BY sequence DESC LIMIT 1)
                 FROM session_entries WHERE session_id=?1",
                [session],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let mut ids = Vec::new();
        for entry in entries {
            sequence += 1;
            let id = format!("{session}-e{sequence}");
            self.db
                .execute(
                    "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,provider_event_id,token_estimate,created_at)
                     VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        id,
                        session,
                        parent,
                        sequence,
                        entry.kind,
                        entry.payload.to_string(),
                        entry.event,
                        entry.estimate,
                        entry.at
                    ],
                )
                .unwrap();
            parent = Some(id.clone());
            ids.push(id);
        }
        ids
    }

    /// A fork as `sessions::fork_session` writes it: the source branch up to
    /// `point` as sequences 1..K, with new ids and no provider event ids.
    fn fork(&self, id: &str, source: &str, point: &str) {
        self.db
            .execute(
                "INSERT INTO sessions(id,workspace_id,harness,label,status,kind,depth,fork_parent_session_id,fork_parent_entry_id)
                 SELECT ?1,workspace_id,harness,'Fork','idle',kind,depth,?2,?3 FROM sessions WHERE id=?2",
                params![id, source, point],
            )
            .unwrap();
        let mut statement = self
            .db
            .prepare(
                "SELECT kind,payload,token_estimate,created_at FROM session_entries
                 WHERE session_id=?1 AND sequence<=(SELECT sequence FROM session_entries WHERE id=?2)
                 ORDER BY sequence",
            )
            .unwrap();
        let copies: Vec<(String, String, Option<i64>, String)> = statement
            .query_map(params![source, point], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut parent: Option<String> = None;
        for (index, (kind, payload, estimate, at)) in copies.into_iter().enumerate() {
            let copy = format!("{id}-c{}", index + 1);
            self.db
                .execute(
                    "INSERT INTO session_entries(id,session_id,parent_entry_id,sequence,kind,payload,provider_event_id,token_estimate,created_at)
                     VALUES(?1,?2,?3,?4,?5,?6,NULL,?7,?8)",
                    params![copy, id, parent, index as i64 + 1, kind, payload, estimate, at],
                )
                .unwrap();
            parent = Some(copy);
        }
    }

    fn ledger(&self, id: i64, session: &str, used: Option<i64>) {
        self.db
            .execute(
                "INSERT INTO usage_ledger(id,workspace_id,session_id,source,created_at,context_used_tokens)
                 VALUES(?1,'w',?2,'provider.claude',?3,?4)",
                params![id, session, RECENT, used],
            )
            .unwrap();
    }

    fn read(&self, options: &ReportOptions) -> Result<Report, BridgeError> {
        let db = open_read_only(&self.path).unwrap();
        report(&db, options)
    }

    fn run(&self) -> Report {
        self.read(&options()).unwrap()
    }

    fn run_with(&self, change: impl FnOnce(&mut ReportOptions)) -> Report {
        let mut options = options();
        change(&mut options);
        self.read(&options).unwrap()
    }

    /// The report of one session, with no recency.
    fn one(&self, session: &str) -> SessionTokens {
        let mut report = self.run_with(|options| options.session = Some(session.to_owned()));
        assert_eq!(report.sessions.len(), 1);
        report.sessions.remove(0)
    }

    fn one_error(&self, session: &str) -> String {
        let mut options = options();
        options.session = Some(session.to_owned());
        self.read(&options).unwrap_err().to_string()
    }
}

fn options() -> ReportOptions {
    ReportOptions { cohort: Cohort::Orchestrator, since: day("2026-05-01"), top: 10, session: None }
}

fn day(value: &str) -> NaiveDate {
    parse_since(value).unwrap()
}

fn ids(report: &Report) -> Vec<&str> {
    report.sessions.iter().map(|session| session.session_id.as_str()).collect()
}

fn class(tokens: &SessionTokens, class: TokenClass) -> i64 {
    tokens.classes[&class]
}

fn claude_start(id: &str, name: &str, input: Value, phase: &str) -> Value {
    json!({
        "itemId": id,
        "status": "inProgress",
        "title": name,
        "data": {"type": "tool_use", "id": id, "name": name, "input": input, "phase": phase},
    })
}

fn claude_result(id: &str, output: &str) -> Value {
    json!({
        "itemId": id,
        "status": "completed",
        "text": output,
        "data": {"type": "tool_result", "tool_use_id": id, "content": output},
    })
}

fn opencode_part(id: &str, tool: &str, command: Option<&str>) -> Value {
    let input = match command {
        Some(command) => json!({"command": command}),
        None => json!({"filePath": "src/a.rs"}),
    };
    json!({
        "itemId": id,
        "title": tool,
        "data": {"type": "tool", "callID": id, "tool": tool, "state": {"status": "completed", "input": input, "output": "out"}},
    })
}

fn codex_item(id: &str, item_type: &str, command: Option<Value>) -> Value {
    let mut data = json!({"type": item_type, "id": id});
    if let Some(command) = command {
        data["command"] = command;
    }
    json!({"itemId": id, "data": data})
}

/// A tool record as `import_bundle::lower_tool` writes it.
fn import_tool(id: &str, name: &str) -> Value {
    json!({
        "sourceId": "src",
        "role": "assistant",
        "kind": "tool.completed",
        "itemId": id,
        "title": name,
        "status": "completed",
        "data": {"type": "tool_use", "name": name, "input": {"pattern": "x"}, "aggregatedOutput": "out"},
    })
}

fn shell(text: &str) -> TokenClass {
    classify_shell(&json!(text))
}

// Classes

#[test]
fn claude_tool_names_map_to_classes() {
    let cases = [
        ("Read", json!({"file_path": "a.rs"}), Read),
        ("NotebookRead", json!({"notebook_path": "a.ipynb"}), Read),
        ("Grep", json!({"pattern": "fn"}), Search),
        ("Glob", json!({"pattern": "*.rs"}), Search),
        ("LS", json!({"path": "."}), Search),
        ("Bash", json!({"command": "rg -n fn src"}), Search),
        ("Bash", json!({"command": "sed -n 1,20p src/a.rs"}), Read),
        ("Bash", json!({"command": "cargo test"}), Shell),
        ("Edit", json!({"file_path": "a.rs"}), Edit),
        ("Write", json!({"file_path": "a.rs"}), Edit),
        ("MultiEdit", json!({"file_path": "a.rs"}), Edit),
        ("NotebookEdit", json!({"notebook_path": "a.ipynb"}), Edit),
        ("WebFetch", json!({"url": "https://example.com"}), Web),
        ("WebSearch", json!({"query": "x"}), Web),
        ("Task", json!({"prompt": "x"}), Agent),
        ("Agent", json!({"prompt": "x"}), Agent),
        ("mcp__bridge__read_note", json!({}), Mcp),
        ("TodoWrite", json!({}), OtherTool),
    ];
    for (name, input, expected) in cases {
        let start = claude_start("toolu_1", name, input, "running");
        assert_eq!(classify_tool("tool.started", Some(&start), None), expected, "{name}");
        // A Claude result has no name, so it takes the class of its start.
        let result = claude_result("toolu_1", "out");
        assert_eq!(classify_tool("tool.completed", Some(&start), Some(&result)), expected, "{name}");
    }
}

#[test]
fn opencode_part_tools_map_to_classes() {
    let cases = [
        ("read", None, Read),
        ("grep", None, Search),
        ("glob", None, Search),
        ("list", None, Search),
        ("bash", Some("cat src/a.rs"), Read),
        ("bash", Some("npm test"), Shell),
        ("shell", Some("git log -3"), Read),
        ("edit", None, Edit),
        ("write", None, Edit),
        ("patch", None, Edit),
        ("multiedit", None, Edit),
        ("apply_patch", None, Edit),
        ("webfetch", None, Web),
        ("websearch", None, Web),
        ("task", None, Agent),
        ("bridge_read_note", None, Mcp),
        ("todowrite", None, OtherTool),
    ];
    for (tool, command, expected) in cases {
        let part = opencode_part("prt_1", tool, command);
        assert_eq!(classify_tool("tool.completed", None, Some(&part)), expected, "{tool}");
        assert_eq!(classify_tool("tool.started", Some(&part), None), expected, "{tool}");
    }
}

#[test]
fn codex_item_types_map_to_classes() {
    let cases = [
        ("commandExecution", Some(json!("rg -n fn src")), Search),
        ("commandExecution", Some(json!("/bin/zsh -lc 'sed -n 5p a.rs'")), Read),
        ("commandExecution", Some(json!(["bash", "-lc", "cat a.rs"])), Read),
        ("commandExecution", Some(json!("make build")), Shell),
        ("commandExecution", None, Shell),
        ("mcpToolCall", None, Mcp),
        ("webSearch", None, Web),
        ("collabAgentToolCall", None, Agent),
        ("fileChange", None, Edit),
        ("dynamicToolCall", None, OtherTool),
    ];
    for (item_type, command, expected) in cases {
        let item = codex_item("item_1", item_type, command);
        assert_eq!(classify_tool("tool.completed", None, Some(&item)), expected, "{item_type}");
        assert_eq!(classify_tool("tool.started", Some(&item), None), expected, "{item_type}");
    }
}

#[test]
fn text_kinds_are_conversation_and_injected_kinds_are_bridge() {
    let named = claude_start("toolu_1", "Read", json!({}), "running");
    for kind in CONVERSATION_KINDS {
        assert_eq!(classify_tool(kind, Some(&named), None), Conversation, "{kind}");
    }
    for kind in BRIDGE_KINDS {
        assert_eq!(classify_tool(kind, Some(&named), None), Bridge, "{kind}");
    }
    assert_eq!(classify_tool("file_change.completed", None, None), Edit);

    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            msg("user.message", 5),
            msg("assistant.message", 7),
            row("reasoning.completed", json!({"text": "t"})).est(3),
            row("reasoning", json!({"text": "t"})).est(2),
            row("session.context", json!({"text": "c"})).est(11),
            row("delegation.result", json!({"text": "d"})).est(13),
            row("worker.result", json!({"summary": "s"})).est(17),
            row("context.compacted", json!({"data": {"summary": "s"}})).est(19),
            row("checkpoint", json!({"summary": "s"})).est(1),
            row("compaction", json!({"summary": "s"})).est(1),
            row("branch.summary", json!({"summary": "s"})).est(1),
            row("handoff.brief", json!({"text": "s"})).est(1),
            // The report reads only the estimated kinds.
            row("turn.completed", json!({"text": "x"})).est(1000),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Conversation), 17);
    assert_eq!(class(&tokens, Bridge), 64);
    assert_eq!(tokens.total, 81);
}

#[test]
fn web_tools_are_web_and_outside_the_reading_share() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            msg("user.message", 0),
            row("tool.started", claude_start("toolu_1", "Read", json!({"file_path": "a.rs"}), "running")).est(10),
            row("tool.completed", claude_result("toolu_1", "x")).est(30),
            row("tool.started", claude_start("toolu_2", "WebFetch", json!({"url": "https://example.com"}), "running")).est(10),
            row("tool.completed", claude_result("toolu_2", "x")).est(50),
            row("command.started", claude_start("toolu_3", "Bash", json!({"command": "cargo test"}), "running")).est(5),
            row("command.completed", claude_result("toolu_3", "ok")).est(20),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!((class(&tokens, Read), class(&tokens, Web), class(&tokens, Shell)), (40, 60, 25));
    assert_eq!(tokens.total, 125);
    assert_eq!(tokens.reading_share, 0.32);
    assert_eq!(tokens.shell_share, 0.2);
}

#[test]
fn title_never_decides_a_class() {
    let titled_read = json!({"itemId": "t1", "title": "Read", "text": "out", "data": {}});
    assert_eq!(classify_tool("tool.completed", None, Some(&titled_read)), OtherTool);
    let titled_command = json!({"itemId": "t2", "title": "rg fn src", "data": {"type": "tool_result", "tool_use_id": "t2"}});
    assert_eq!(classify_tool("command.completed", None, Some(&titled_command)), OtherTool);
    // OpenCode puts free text in the title. The tool decides.
    let mut part = opencode_part("prt_1", "bash", Some("cargo build"));
    part["title"] = json!("cat src/a.rs");
    assert_eq!(classify_tool("command.started", Some(&part), None), Shell);

    let w = world();
    w.chat("chat-a");
    w.put("chat-a", vec![row("tool.completed", titled_read).est(9), row("command.completed", titled_command).est(4)]);
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, OtherTool), 13);
    assert_eq!(class(&tokens, Read) + class(&tokens, Search), 0);
}

// Keys and groups

#[test]
fn a_completion_takes_the_class_of_its_start_by_payload_item_id() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("tool.started", claude_start("toolu_1", "Read", json!({"file_path": "a.rs"}), "running")).est(10).event("ev-start"),
            // Another provider event id, the same item id.
            row("tool.completed", claude_result("toolu_1", "fn main() {}")).est(30).event("ev-done"),
            // The provider event id of this row is the item id of the read,
            // but the keys of two sources never merge.
            row("tool.completed", claude_result("toolu_2", "x")).est(7).event("toolu_1"),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Read), 40);
    assert_eq!(class(&tokens, OtherTool), 7);
    assert_eq!(tokens.total, 47);
}

#[test]
fn a_forked_start_and_completion_pair_without_provider_event_ids() {
    // A fork copies these rows with the payload and no provider event id.
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("tool.started", claude_start("toolu_1", "Grep", json!({}), "preparing")),
            row("tool.started", claude_start("toolu_1", "Grep", json!({"pattern": "fn"}), "running")).est(10),
            row("tool.completed", claude_result("toolu_1", "a.rs:1")).est(30),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Search), 40);
    assert_eq!(class(&tokens, OtherTool), 0);
    assert_eq!(tokens.unestimated, 0);
}

#[test]
fn an_imported_completion_pairs_by_payload_item_id_not_source_id() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            // Two records of one call, with two source ids: one item.
            row("tool.completed", import_tool("call_1", "Read")).est(20).event("src-use"),
            row("tool.completed", import_tool("call_1", "Read")).est(50).event("src-result"),
            row("tool.completed", import_tool("call_2", "Grep")).est(9).event("call_1"),
            // An import with no tool name pairs with its start by item id.
            row("tool.started", claude_start("call_3", "Read", json!({"file_path": "b.rs"}), "running")).est(4).event("live-3"),
            row("tool.completed", import_tool("call_3", "tool")).est(6).event("src-3"),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Read), 60);
    assert_eq!(class(&tokens, Search), 9);
    assert_eq!(class(&tokens, OtherTool), 0);
}

#[test]
fn fallback_keys_are_scoped_by_session_and_source() {
    let w = world();
    w.chat("chat-a");
    w.chat("chat-b");
    let output = || json!({"text": "out"});
    w.put(
        "chat-a",
        vec![
            msg("user.message", 1),
            row("tool.completed", output()).est(10).event("p1"),
            row("tool.completed", output()).est(30).event("p1"),
            row("tool.completed", json!({"itemId": "p1", "text": "out"})).est(40),
            row("tool.completed", output()).est(5),
            row("tool.completed", output()).est(7),
            // This provider event id is the id of the entry with 5 tokens.
            row("tool.completed", output()).est(9).event("chat-a-e5"),
        ],
    );
    w.put("chat-b", vec![msg("user.message", 1), row("tool.completed", output()).est(30).event("p1")]);
    let report = w.run();
    assert_eq!(ids(&report), ["chat-a", "chat-b"]);
    assert_eq!(class(&report.sessions[0], OtherTool), 30 + 40 + 5 + 7 + 9);
    assert_eq!(class(&report.sessions[1], OtherTool), 30);
}

#[test]
fn each_item_counts_once_per_kind_with_its_largest_estimate() {
    let w = world();
    w.chat("chat-a");
    // OpenCode writes a start for each update of a tool part.
    let part = || opencode_part("prt_1", "read", None);
    w.put(
        "chat-a",
        vec![
            row("tool.started", part()).est(3),
            row("tool.started", part()).est(8),
            row("tool.started", part()).est(5),
            row("tool.completed", part()).est(20),
            row("tool.completed", part()).est(12),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Read), 28);
    assert_eq!(tokens.total, 28);
}

#[test]
fn a_null_estimate_falls_back_to_entry_tokens() {
    let w = world();
    w.chat("chat-a");
    let text = json!({"text": "x".repeat(40)});
    let input = claude_start("toolu_1", "Read", json!({"file_path": "src/lib.rs"}), "running");
    let output = claude_result("toolu_1", &"y".repeat(400));
    let expected = entry_tokens::estimate("tool.started", &input).unwrap()
        + entry_tokens::estimate("tool.completed", &output).unwrap();
    w.put("chat-a", vec![row("user.message", text), row("tool.started", input), row("tool.completed", output)]);
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Conversation), 10);
    assert_eq!(class(&tokens, Read), expected);
    assert!(expected > 100);
    assert_eq!(tokens.unestimated, 0);
}

#[test]
fn a_preparing_start_with_a_full_start_is_estimated() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("command.started", claude_start("toolu_1", "Bash", json!({}), "preparing")),
            row("command.started", claude_start("toolu_1", "Bash", json!({"command": "rg fn src"}), "running")).est(12),
            row("command.completed", claude_result("toolu_1", "src/a.rs")).est(30),
            // A preparing start alone has no input yet. It is not unestimated.
            row("tool.started", claude_start("toolu_2", "Read", json!({}), "preparing")),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Search), 42);
    assert_eq!(tokens.total, 42);
    assert_eq!(tokens.unestimated, 0);
}

#[test]
fn an_item_with_no_estimate_counts_once_as_unestimated() {
    let w = world();
    w.chat("chat-a");
    let bare = |id: &str| json!({"itemId": id, "data": {"id": id}});
    w.put(
        "chat-a",
        vec![
            row("tool.completed", bare("x")),
            row("tool.completed", bare("x")),
            row("tool.completed", bare("y")).est(4),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(tokens.unestimated, 1);
    assert_eq!(tokens.total, 4);
}

// Completion-only records

#[test]
fn a_completion_only_opencode_part_takes_its_class_from_its_tool() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("tool.completed", opencode_part("prt_1", "grep", None)).est(25),
            row("command.completed", opencode_part("prt_2", "bash", Some("cat src/a.rs"))).est(15),
            row("tool.completed", opencode_part("prt_3", "todowrite", None)).est(3),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!((class(&tokens, Search), class(&tokens, Read), class(&tokens, OtherTool)), (25, 15, 3));
}

#[test]
fn a_completion_only_codex_item_takes_its_class_from_its_type() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("command.completed", codex_item("item_1", "commandExecution", Some(json!("rg -n token src")))).est(11),
            row("tool.completed", codex_item("item_2", "webSearch", None)).est(13),
            row("tool.completed", codex_item("item_3", "mcpToolCall", None)).est(17),
            row("tool.completed", codex_item("item_4", "collabAgentToolCall", None)).est(19),
            row("file_change.completed", codex_item("item_5", "fileChange", None)).est(23),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Search), 11);
    assert_eq!(class(&tokens, Web), 13);
    assert_eq!(class(&tokens, Mcp), 17);
    assert_eq!(class(&tokens, Agent), 19);
    assert_eq!(class(&tokens, Edit), 23);
}

#[test]
fn a_completion_only_import_takes_its_class_from_data_name() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("tool.completed", import_tool("call_1", "Read")).est(31).event("src-1"),
            row("tool.completed", import_tool("call_2", "Grep")).est(37).event("src-2"),
            row("tool.completed", import_tool("call_3", "tool")).est(2).event("src-3"),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!((class(&tokens, Read), class(&tokens, Search), class(&tokens, OtherTool)), (31, 37, 2));
}

#[test]
fn a_completion_only_claude_bash_result_uses_its_command() {
    let w = world();
    w.chat("chat-a");
    let mut bash = claude_result("toolu_1", "fn main() {}");
    bash["data"]["command"] = json!("cat src/main.rs");
    // Monitor also has a command, but it is not a Bash completion.
    let mut monitor = claude_result("toolu_2", "fn main() {}");
    monitor["data"]["command"] = json!("cat src/main.rs");
    w.put("chat-a", vec![row("command.completed", bash).est(21), row("tool.completed", monitor).est(5)]);
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, Read), 21);
    assert_eq!(class(&tokens, OtherTool), 5);
}

#[test]
fn a_completion_with_no_start_and_no_tool_fields_is_other_tool() {
    let w = world();
    w.chat("chat-a");
    w.put(
        "chat-a",
        vec![
            row("tool.completed", claude_result("toolu_1", "out")).est(8),
            row("command.completed", json!({"itemId": "t2", "text": "out", "data": {}})).est(2),
            row("tool.completed", json!({"text": "out"})).est(1),
        ],
    );
    let tokens = w.one("chat-a");
    assert_eq!(class(&tokens, OtherTool), 11);
    assert_eq!(tokens.total, 11);
}

// Shell rule

#[test]
fn simple_reads_are_read() {
    for command in [
        "cat src/main.rs",
        "/usr/bin/cat a.rs",
        "head -n 20 a.rs",
        "tail -5 log.txt",
        "nl a.rs",
        "wc -l a.rs b.rs",
        "sed -n 10p a.rs",
        "sed -n '1,40p' src/a.rs",
        "git show HEAD:src/a.rs",
        "git diff main",
        "git log -5 --oneline",
        "git blame a.rs",
    ] {
        assert_eq!(shell(command), Read, "{command}");
    }
}

#[test]
fn simple_searches_are_search() {
    for command in [
        "rg -n foo src",
        "grep -rn foo .",
        "ls -la",
        "ls",
        "fd main.rs",
        "fd -e rs token",
        "find . -name '*.rs'",
        "git grep foo",
        "git ls-files",
    ] {
        assert_eq!(shell(command), Search, "{command}");
    }
}

#[test]
fn known_wrappers_are_unwrapped_once() {
    assert_eq!(shell("bash -lc 'rg foo src'"), Search);
    assert_eq!(shell("/bin/zsh -lc 'cat a.rs'"), Read);
    assert_eq!(shell("sh -c ls"), Search);
    assert_eq!(shell("bash -cl 'git log'"), Read);
    assert_eq!(shell("/usr/local/bin/bash -c 'head a.rs'"), Read);
    // One time only.
    assert_eq!(shell("bash -lc \"bash -lc 'cat a.rs'\""), Shell);
    // Another flag, another shell or another word count is not a wrapper.
    assert_eq!(shell("bash -x 'cat a.rs'"), Shell);
    assert_eq!(shell("bash -lc 'cat a.rs' extra"), Shell);
    assert_eq!(shell("fish -c 'cat a.rs'"), Shell);
    // The script of a wrapper goes through the operator check.
    assert_eq!(shell("bash -lc 'cat a.rs | head'"), Shell);
}

#[test]
fn argv_commands_follow_the_same_rule() {
    assert_eq!(classify_shell(&json!(["cat", "a.rs"])), Read);
    assert_eq!(classify_shell(&json!(["/bin/zsh", "-lc", "rg foo src"])), Search);
    assert_eq!(classify_shell(&json!(["git", "log", "-3"])), Read);
    assert_eq!(classify_shell(&json!(["bash", "-lc", "cat a.rs | head"])), Shell);
    assert_eq!(classify_shell(&json!(["bash", "-lc", "bash -c 'cat a.rs'"])), Shell);
    assert_eq!(classify_shell(&json!(["rg", "--pre", "x", "foo"])), Shell);
    assert_eq!(classify_shell(&json!(["find", ".", "-delete"])), Shell);
    assert_eq!(classify_shell(&json!(["A=1", "cat", "a.rs"])), Shell);
    assert_eq!(classify_shell(&json!(["cargo", "test"])), Shell);
    assert_eq!(classify_shell(&json!([])), Shell);
    assert_eq!(classify_shell(&json!(["cat", 1])), Shell);
    assert_eq!(classify_shell(&json!(42)), Shell);
    assert_eq!(classify_shell(&Value::Null), Shell);
}

#[test]
fn any_shell_operator_is_shell_even_in_quotes() {
    for command in [
        "rg 'a|b' src",
        "cat a && cat b",
        "cat a; ls",
        "cat a > b",
        "cat input>output",
        "cat < a",
        "echo $(cat a)",
        "cat $HOME/a",
        "cat `ls`",
        "cat a\nls",
        "grep 'x(y)' a",
        "ls &",
        "grep \"a;b\" src",
    ] {
        assert_eq!(shell(command), Shell, "{command:?}");
    }
}

#[test]
fn writes_and_exec_flags_are_shell() {
    for command in [
        "sed -i s/a/b/ a.rs",
        "sed -n s/a/b/p a.rs",
        "sed -n 1,5p",
        "sed -n 1,5p -i a.rs",
        "sed 1,5p a.rs",
        "git diff --output=patch.diff",
        "git log --output out.txt",
        "git push",
        "git checkout main",
        "git",
        "rg --pre ./script foo",
        "rg --pre-glob '*.gz' foo",
        "fd -x rm",
        "fd -X rm",
        "fd --exec rm",
        "fd --exec-batch rm",
        "fd -Hx rm",
        "find . -name x -exec rm {} +",
        "find . -execdir ls {} +",
        "find . -ok rm {} +",
        "find . -okdir rm {} +",
        "find . -delete",
        "find . -fprint out",
        "find . -fprintf out %p",
        "find . -fls out",
    ] {
        assert_eq!(shell(command), Shell, "{command}");
    }
}

#[test]
fn unknown_programs_and_unsplittable_commands_are_shell() {
    for command in [
        "cargo test",
        "npm run build",
        "echo hi",
        "rg 'unterminated",
        "",
        "   ",
        "FOO=1 cat a.rs",
        "python3 script.py",
    ] {
        assert_eq!(shell(command), Shell, "{command:?}");
    }
}

// Cohort and recency

#[test]
fn the_orchestrator_cohort_keeps_top_level_workspace_orchestrators_only() {
    let w = world();
    w.chat("chat-a");
    w.session("no-workspace", "orchestrator", None);
    w.worker("worker-1", "chat-a");
    w.chat("runtime-only");
    w.runtime("runtime-only", "chat-a");
    w.chat("deep");
    w.db.execute("UPDATE sessions SET depth=1 WHERE id='deep'", []).unwrap();
    w.session("sub", crate::harness_subagents::KIND, Some("w"));
    w.child_of("sub", "chat-a");
    w.session("direct-a", "direct", Some("w"));
    for id in ["chat-a", "no-workspace", "worker-1", "runtime-only", "deep", "sub", "direct-a"] {
        w.talk(id);
    }
    let report = w.run();
    assert_eq!(ids(&report), ["chat-a"]);
    assert_eq!(report.cohort, Cohort::Orchestrator);
    assert_eq!(report.sessions[0].chat_kind, "orchestrator");
}

#[test]
fn the_direct_cohort_keeps_top_level_direct_chats_only() {
    let w = world();
    w.session("direct-a", "direct", None);
    w.session("direct-w", "direct", Some("w"));
    w.session("direct-child", "direct", None);
    w.child_of("direct-child", "direct-a");
    w.session("direct-worker", "direct", None);
    w.runtime("direct-worker", "direct-a");
    w.chat("chat-a");
    for id in ["direct-a", "direct-w", "direct-child", "direct-worker", "chat-a"] {
        w.talk(id);
    }
    let report = w.run_with(|options| options.cohort = Cohort::Direct);
    assert_eq!(ids(&report), ["direct-a", "direct-w"]);
    assert_eq!(report.cohort, Cohort::Direct);
}

#[test]
fn imported_workflow_and_hidden_sessions_are_in_no_cohort() {
    let w = world();
    let hidden = crate::chat_search::index::HIDDEN_KINDS_SQL
        .split(',')
        .map(|kind| kind.trim().trim_matches('\'').to_owned());
    let kinds: Vec<String> = ["imported", crate::workflow_run::WORKFLOW_SESSION_KIND, crate::harness_subagents::KIND]
        .into_iter()
        .map(str::to_owned)
        .chain(hidden)
        .collect();
    assert!(kinds.len() > 5, "{kinds:?}");
    for kind in &kinds {
        let id = format!("{kind}-1");
        w.session(&id, kind, Some("w"));
        w.talk(&id);
    }
    assert!(w.run().sessions.is_empty());
    assert!(w.run_with(|options| options.cohort = Cohort::Direct).sessions.is_empty());
    for kind in &kinds {
        let error = w.one_error(&format!("{kind}-1"));
        assert!(error.contains("not a top-level orchestrator chat or direct chat"), "{error}");
    }
}

#[test]
fn recency_uses_the_last_own_message_not_the_last_entry() {
    let w = world();
    for id in ["a", "b", "c"] {
        w.chat(id);
    }
    // A recent entry that is not a message is not activity.
    w.put("a", vec![msg("user.message", 1).at(OLD), row("session.context", json!({"text": "c"})).est(1)]);
    // The last message by sequence decides, not the latest time.
    w.put("b", vec![msg("user.message", 1), msg("assistant.message", 1).at(OLD)]);
    w.put(
        "c",
        vec![
            msg("user.message", 1).at(OLD),
            msg("assistant.message", 1),
            row("tool.completed", json!({"text": "x"})).est(1).at(OLD),
        ],
    );
    let report = w.run();
    assert_eq!(ids(&report), ["c"]);
    assert_eq!(report.sessions[0].last_activity_at.as_deref(), Some(RECENT));
}

#[test]
fn since_changes_the_window() {
    let w = world();
    for (id, at) in [
        ("april", "2026-04-15T08:00:00Z"),
        ("june", RECENT),
        ("edge", "2026-04-15T00:00:00Z"),
        ("before", "2026-04-14T23:59:59Z"),
        ("offset", "2026-04-15T01:00:00+02:00"),
    ] {
        w.chat(id);
        w.put(id, vec![msg("user.message", 1).at(at)]);
    }
    assert_eq!(ids(&w.run()), ["june"]);
    let report = w.run_with(|options| options.since = day("2026-04-15"));
    assert_eq!(ids(&report), ["april", "edge", "june"]);
    assert_eq!(report.since, "2026-04-15");
    let offset = w.one("offset");
    assert_eq!(offset.last_activity_at.as_deref(), Some("2026-04-14T23:00:00Z"));

    assert_eq!(default_since(day("2026-11-20")), day("2026-10-21"));
    assert_eq!(default_since(day("2026-03-15")), day("2026-02-13"));
    for bad in ["2026-5-1", "2026-13-01", "2026-02-30", "20260501", "2026/05/01", "", "2026-05-01T00"] {
        assert!(parse_since(bad).is_err(), "{bad}");
    }
}

#[test]
fn an_unreadable_timestamp_is_not_recent_and_is_counted() {
    let w = world();
    for id in ["bad", "good", "blank"] {
        w.chat(id);
    }
    w.put("bad", vec![msg("user.message", 1).at("last tuesday")]);
    // Only the last message counts, so an older bad value is not counted.
    w.put("good", vec![msg("user.message", 1).at("garbage"), msg("assistant.message", 1)]);
    w.put("blank", vec![msg("user.message", 1).at("")]);
    let report = w.run();
    assert_eq!(ids(&report), ["good"]);
    assert_eq!(report.unreadable_timestamps, 2);
}

#[test]
fn the_session_option_rejects_a_worker() {
    let w = world();
    w.chat("chat-a");
    w.put("chat-a", vec![msg("user.message", 4).at(OLD)]);
    w.worker("worker-1", "chat-a");
    w.talk("worker-1");
    w.session("sub", crate::harness_subagents::KIND, Some("w"));
    w.child_of("sub", "chat-a");
    for id in ["worker-1", "sub", "missing"] {
        let error = w.one_error(id);
        assert!(error.contains(id), "{error}");
    }
    // The option skips recency for a chat in a cohort.
    let report = w.run_with(|options| options.session = Some("chat-a".to_owned()));
    assert_eq!(ids(&report), ["chat-a"]);
    assert_eq!((report.eligible, report.sessions[0].rank, report.sessions[0].total), (1, 1, 4));
    // The cohort comes from the kind of the session.
    w.session("direct-a", "direct", None);
    let report = w.run_with(|options| options.session = Some("direct-a".to_owned()));
    assert_eq!(report.cohort, Cohort::Direct);
    assert_eq!(report.sessions[0].last_activity_at, None);
}

// Forks

#[test]
fn a_fork_excludes_its_copied_prefix_from_tokens_and_activity() {
    let w = world();
    w.chat("src");
    let entries = w.put(
        "src",
        vec![
            msg("user.message", 10),
            msg("assistant.message", 20),
            row("tool.started", claude_start("toolu_1", "Read", json!({"file_path": "a.rs"}), "running")).est(5).event("toolu_1"),
            row("tool.completed", claude_result("toolu_1", "x")).est(100).event("toolu_1"),
        ],
    );
    w.fork("fork-a", "src", &entries[3]);
    w.put("fork-a", vec![msg("user.message", 3), msg("assistant.message", 4)]);
    // A fork with no own message: its copied messages are not activity.
    w.fork("fork-quiet", "src", &entries[3]);
    let report = w.run();
    assert_eq!(ids(&report), ["src", "fork-a"]);
    assert_eq!(report.sessions[0].total, 135);
    let fork = &report.sessions[1];
    assert_eq!((fork.total, class(fork, Conversation), class(fork, Read)), (7, 7, 0));
    assert_eq!((fork.fork_prefix_entries, fork.fork_prefix_unknown), (4, false));
    let quiet = w.one("fork-quiet");
    assert_eq!((quiet.total, quiet.fork_prefix_entries), (0, 4));
    assert_eq!(quiet.last_activity_at, None);
}

#[test]
fn a_fork_of_a_fork_excludes_only_its_own_prefix() {
    let w = world();
    w.chat("src");
    let source = w.put("src", vec![msg("user.message", 10), msg("assistant.message", 20)]);
    w.fork("f1", "src", &source[1]);
    let first = w.put("f1", vec![msg("user.message", 3), msg("assistant.message", 4)]);
    w.fork("f2", "f1", &first[1]);
    w.put("f2", vec![msg("user.message", 1)]);
    let report = w.run();
    let totals: Vec<(&str, i64, i64)> = report
        .sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session.total, session.fork_prefix_entries))
        .collect();
    assert_eq!(totals, [("src", 30, 0), ("f1", 7, 2), ("f2", 1, 4)]);
}

#[test]
fn a_fork_with_an_absent_source_entry_counts_every_entry_and_says_so() {
    let w = world();
    w.chat("src");
    let source = w.put("src", vec![msg("user.message", 10), msg("assistant.message", 20)]);
    w.fork("f1", "src", &source[1]);
    w.put("f1", vec![msg("user.message", 3)]);
    w.db.execute("UPDATE sessions SET fork_parent_entry_id='gone' WHERE id='f1'", []).unwrap();
    w.fork("f2", "src", &source[1]);
    w.put("f2", vec![msg("user.message", 4)]);
    w.db.execute("UPDATE sessions SET fork_parent_session_id='deleted' WHERE id='f2'", []).unwrap();
    let report = w.run();
    assert_eq!(ids(&report), ["f2", "f1", "src"]);
    for (fork, total) in [(&report.sessions[0], 34), (&report.sessions[1], 33)] {
        assert_eq!(fork.total, total);
        assert_eq!((fork.fork_prefix_entries, fork.fork_prefix_unknown), (0, true));
    }
    let markdown = render_markdown(&report);
    assert_eq!(markdown.matches("fork-prefix-unknown").count(), 2, "{markdown}");
}

// Rank

#[test]
fn the_top_ten_comes_from_exact_totals_of_every_eligible_session() {
    let w = world();
    for n in 1..=11 {
        let id = format!("c{n:02}");
        w.chat(&id);
        w.put(&id, vec![msg("user.message", 100 + n)]);
    }
    // Many snapshots of one item: a sum of rows gives 301, the exact total
    // is 11.
    w.chat("snap");
    let mut snapshots = vec![msg("user.message", 1)];
    snapshots.extend((0..30).map(|_| row("tool.started", opencode_part("prt_1", "read", None)).est(10)));
    w.put("snap", snapshots);
    let rough: i64 = w
        .db
        .query_row("SELECT SUM(token_estimate) FROM session_entries WHERE session_id='snap'", [], |row| row.get(0))
        .unwrap();
    assert!(rough > 111, "the sum of rows ranks snap first");

    let report = w.run();
    assert_eq!(report.eligible, 12);
    let expected: Vec<String> = (2..=11).rev().map(|n| format!("c{n:02}")).collect();
    assert_eq!(ids(&report), expected);
    let ranks: Vec<usize> = report.sessions.iter().map(|session| session.rank).collect();
    assert_eq!(ranks, (1..=10).collect::<Vec<_>>());
    assert_eq!(w.one("snap").total, 11);
}

#[test]
fn a_rank_tie_breaks_by_session_id() {
    let w = world();
    for (id, tokens) in [("b", 50), ("a", 50), ("d", 60), ("c", 50)] {
        w.chat(id);
        w.put(id, vec![msg("user.message", tokens)]);
    }
    let report = w.run();
    assert_eq!(ids(&report), ["d", "a", "b", "c"]);
    let ranks: Vec<usize> = report.sessions.iter().map(|session| session.rank).collect();
    assert_eq!(ranks, [1, 2, 3, 4]);
}

#[test]
fn an_empty_recent_session_ranks_last_with_zero_shares() {
    let w = world();
    w.chat("empty");
    w.put("empty", vec![row("user.message", json!({"text": ""}))]);
    w.chat("full");
    w.put("full", vec![msg("user.message", 10)]);
    let report = w.run();
    assert_eq!(ids(&report), ["full", "empty"]);
    let empty = &report.sessions[1];
    assert_eq!((empty.total, empty.unestimated), (0, 0));
    assert_eq!((empty.reading_share, empty.shell_share), (0.0, 0.0));
    assert!(render_markdown(&report).contains("| 2 | empty | claude | 0 |"));
}

// Schema and the old database

#[test]
fn a_schema_59_database_reads_with_exact_fallbacks() {
    let w = world();
    w.chat("chat-a");
    w.put("chat-a", vec![msg("user.message", 10), row("tool.completed", import_tool("call_1", "Read")).est(5)]);
    w.ledger(1, "chat-a", Some(900));
    w.db.execute_batch(
        "ALTER TABLE sessions DROP COLUMN fork_parent_session_id;
         ALTER TABLE sessions DROP COLUMN fork_parent_entry_id;
         ALTER TABLE sessions DROP COLUMN fork_worktree_policy;
         ALTER TABLE sessions DROP COLUMN context_usage_after_id;
         ALTER TABLE usage_ledger DROP COLUMN context_used_tokens;
         DELETE FROM schema_version WHERE version>59;",
    )
    .unwrap();
    let report = w.run();
    assert_eq!(report.schema_version, 59);
    assert_eq!(report.fallbacks, [FALLBACK_NO_FORKS, FALLBACK_NO_EPOCH, FALLBACK_NO_WINDOW]);
    let tokens = &report.sessions[0];
    assert_eq!((tokens.total, class(tokens, Read)), (15, 5));
    assert_eq!((tokens.fork_prefix_entries, tokens.fork_prefix_unknown), (0, false));
    assert_eq!((tokens.last_context_used, tokens.max_context_used), (None, None));
    let markdown = render_markdown(&report);
    for fallback in report.fallbacks {
        assert!(markdown.contains(&format!("\nFallback: {fallback}\n")), "{markdown}");
    }
}

#[test]
fn a_database_without_a_required_column_is_a_plain_error() {
    let w = world();
    w.chat("chat-a");
    w.talk("chat-a");
    w.db.execute_batch("ALTER TABLE session_entries DROP COLUMN token_estimate;").unwrap();
    let error = w.read(&options()).unwrap_err().to_string();
    assert_eq!(
        error,
        "The database has no column session_entries.token_estimate. Open it one time with a \
         current Bridge build, then take a new backup."
    );
}

#[test]
fn a_newer_schema_version_is_a_plain_error() {
    let w = world();
    // A newer build could also change a table. The version check comes first.
    w.db.execute_batch(
        "INSERT INTO schema_version(version,applied_at) SELECT MAX(version)+1,'2026-06-01T00:00:00Z' FROM schema_version;
         ALTER TABLE session_entries DROP COLUMN token_estimate;",
    )
    .unwrap();
    let error = w.read(&options()).unwrap_err().to_string();
    assert!(error.starts_with("This database uses schema version"), "{error}");
    assert!(error.ends_with("Open it with a newer Bridge version."), "{error}");
}

// Transaction and writes

#[test]
fn the_report_connection_rejects_writes() {
    let w = world();
    let db = open_read_only(&w.path).unwrap();
    let query_only: i64 = db.query_row("PRAGMA query_only", [], |row| row.get(0)).unwrap();
    assert_eq!(query_only, 1);
    let insert = "INSERT INTO projects(id,name,path,created_at) VALUES('x','X','/x','2026-06-01T00:00:00Z')";
    assert!(db.execute(insert, []).is_err());
    assert!(db.execute_batch("CREATE TABLE scratch(a)").is_err());
    // The file is also open read-only, so a write fails without query_only.
    db.execute_batch("PRAGMA query_only=OFF").unwrap();
    let error = db.execute(insert, []).unwrap_err().to_string();
    assert!(error.contains("readonly"), "{error}");
    let projects: i64 = w.db.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0)).unwrap();
    assert_eq!(projects, 1);
    // The report does not make a database that is not there.
    let absent = w._dir.path().join("absent.db");
    assert!(open_read_only(&absent).is_err());
    assert!(!absent.exists());
}

#[test]
fn one_read_transaction_hides_a_concurrent_write() {
    let w = world();
    w.chat("chat-a");
    w.put("chat-a", vec![msg("user.message", 10)]);
    let reader = open_read_only(&w.path).unwrap();
    let mut wrote = false;
    let seen = build(&reader, &options(), &mut || {
        w.put("chat-a", vec![msg("assistant.message", 1000)]);
        w.chat("chat-b");
        w.put("chat-b", vec![msg("user.message", 5)]);
        wrote = true;
    })
    .unwrap();
    assert!(wrote);
    assert_eq!(ids(&seen), ["chat-a"]);
    assert_eq!(seen.sessions[0].total, 10);
    let after = report(&reader, &options()).unwrap();
    assert_eq!(ids(&after), ["chat-a", "chat-b"]);
    assert_eq!(after.sessions[0].total, 1010);
}

// Window columns

#[test]
fn window_columns_use_the_current_provider_epoch() {
    let w = world();
    for id in ["chat-a", "chat-b"] {
        w.chat(id);
        w.talk(id);
    }
    for (id, used) in [(1, Some(5000)), (2, Some(9000)), (3, None), (4, Some(300)), (6, Some(200)), (5, Some(700))] {
        w.ledger(id, "chat-a", used);
    }
    w.ledger(7, "chat-b", Some(800));
    w.db.execute_batch(
        "UPDATE sessions SET context_usage_after_id=2 WHERE id='chat-a';
         UPDATE sessions SET context_usage_after_id=7 WHERE id='chat-b';",
    )
    .unwrap();
    let report = w.run();
    assert!(report.fallbacks.is_empty());
    let windows: Vec<(&str, Option<i64>, Option<i64>)> = report
        .sessions
        .iter()
        .map(|session| (session.session_id.as_str(), session.last_context_used, session.max_context_used))
        .collect();
    assert_eq!(windows, [("chat-a", Some(200), Some(700)), ("chat-b", None, None)]);
}

#[test]
fn window_columns_are_null_without_the_ledger_column() {
    let w = world();
    w.chat("chat-a");
    w.talk("chat-a");
    w.ledger(1, "chat-a", Some(4000));
    w.db.execute_batch("ALTER TABLE usage_ledger DROP COLUMN context_used_tokens;").unwrap();
    let report = w.run();
    assert_eq!(report.fallbacks, [FALLBACK_NO_WINDOW]);
    let tokens = &report.sessions[0];
    assert_eq!((tokens.last_context_used, tokens.max_context_used), (None, None));
}

// Output

fn fixed_report() -> Report {
    let zero: BTreeMap<TokenClass, i64> = TokenClass::ALL.iter().map(|class| (*class, 0)).collect();
    let mut classes = zero.clone();
    classes.extend([(Read, 40), (Search, 20), (Shell, 25), (Conversation, 15)]);
    let first = SessionTokens {
        rank: 1,
        session_id: "chat-a".into(),
        harness: "claude".into(),
        chat_kind: "orchestrator".into(),
        total: 100,
        classes,
        reading_share: 0.6,
        shell_share: 0.25,
        unestimated: 2,
        fork_prefix_entries: 3,
        fork_prefix_unknown: false,
        last_activity_at: Some(RECENT.into()),
        last_context_used: Some(1200),
        max_context_used: Some(3400),
    };
    let second = SessionTokens {
        rank: 2,
        session_id: "fork-b".into(),
        harness: "codex".into(),
        chat_kind: "orchestrator".into(),
        total: 0,
        classes: zero,
        reading_share: 0.0,
        shell_share: 0.0,
        unestimated: 0,
        fork_prefix_entries: 0,
        fork_prefix_unknown: true,
        last_activity_at: None,
        last_context_used: None,
        max_context_used: None,
    };
    Report {
        schema_version: 85,
        cohort: Cohort::Orchestrator,
        since: "2026-05-01".into(),
        eligible: 3,
        fallbacks: vec![FALLBACK_NO_WINDOW],
        unreadable_timestamps: 1,
        sessions: vec![first, second],
    }
}

#[test]
fn markdown_output_matches_a_fixed_table() {
    let expected = "\
Token report: schema 85, cohort orchestrator, since 2026-05-01, eligible 3, shown 2, unreadableTimestamps 1
Fallback: no usage_ledger.context_used_tokens: lastContextUsed and maxContextUsed are null

| rank | session | harness | total | read | search | shell | edit | web | mcp | agent | other_tool | conversation | bridge | readingShare | shellShare | unestimated | lastContextUsed | maxContextUsed | lastActivityAt | notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | chat-a | claude | 100 | 40 | 20 | 25 | 0 | 0 | 0 | 0 | 0 | 15 | 0 | 60.0% | 25.0% | 2 | 1200 | 3400 | 2026-06-10T12:00:00Z | - |
| 2 | fork-b | codex | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0% | 0.0% | 0 | - | - | - | fork-prefix-unknown |
";
    assert_eq!(render_markdown(&fixed_report()), expected);
}

#[test]
fn json_output_has_the_documented_fields() {
    let w = world();
    w.chat("chat-a");
    w.put("chat-a", vec![msg("user.message", 10)]);
    w.ledger(1, "chat-a", Some(1500));
    let report = w.run();
    let value: Value = serde_json::from_str(&render_json(&report).unwrap()).unwrap();
    let keys = |value: &Value| value.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys(&value),
        ["schemaVersion", "cohort", "since", "eligible", "fallbacks", "unreadableTimestamps", "sessions"]
    );
    let session = &value["sessions"][0];
    assert_eq!(
        keys(session),
        [
            "rank",
            "sessionId",
            "harness",
            "chatKind",
            "total",
            "classes",
            "readingShare",
            "shellShare",
            "unestimated",
            "forkPrefixEntries",
            "forkPrefixUnknown",
            "lastActivityAt",
            "lastContextUsed",
            "maxContextUsed",
        ]
    );
    assert_eq!(
        keys(&session["classes"]),
        ["read", "search", "shell", "edit", "web", "mcp", "agent", "other_tool", "conversation", "bridge"]
    );
    assert_eq!(value["cohort"], "orchestrator");
    assert_eq!(value["since"], "2026-05-01");
    assert_eq!(session["sessionId"], "chat-a");
    assert_eq!(session["classes"]["conversation"], 10);
    assert_eq!(session["lastActivityAt"], RECENT);
    assert_eq!(session["lastContextUsed"], 1500);
    assert_eq!(serde_json::to_value(fixed_report()).unwrap()["sessions"][1]["lastActivityAt"], Value::Null);
}

#[test]
fn no_output_contains_payload_text() {
    let w = world();
    w.db.execute_batch(
        "UPDATE projects SET name='MARK-PROJECT';
         UPDATE workspaces SET title='MARK-WORKSPACE', city='MARK-CITY', branch='MARK-BRANCH';",
    )
    .unwrap();
    w.chat("chat-a");
    w.session("chat-d", "direct", None);
    w.worker("worker-1", "chat-a");
    w.db.execute("UPDATE sessions SET label='MARK-LABEL', title='MARK-TITLE'", []).unwrap();
    let mut bash = claude_start("toolu_1", "Bash", json!({"command": "cat MARK-TOOL-INPUT"}), "running");
    bash["data"]["command"] = json!("cat MARK-COMMAND");
    let mut result = claude_result("toolu_1", "MARK-TOOL-OUTPUT");
    result["data"]["command"] = json!("cat MARK-COMMAND");
    result["data"]["aggregatedOutput"] = json!("MARK-AGGREGATED");
    w.put(
        "chat-a",
        vec![
            row("user.message", json!({"text": "MARK-USER-TEXT"})).event("MARK-EVENT"),
            row("reasoning.completed", json!({"text": "MARK-REASONING"})),
            row("command.started", bash),
            row("command.completed", result),
            row("tool.completed", import_tool("MARK-CALL", "MARK-TOOL-NAME")),
            row("session.context", json!({"text": "MARK-CONTEXT"})),
            row("worker.result", json!({"summary": "MARK-WORKER-SUMMARY"})),
            row("assistant.message", json!({"text": "MARK-ASSISTANT", "title": "MARK-ENTRY-TITLE"})),
        ],
    );
    w.put("chat-d", vec![row("user.message", json!({"text": "MARK-DIRECT"})).at("MARK-CREATED")]);
    let reports = [
        w.run(),
        w.run_with(|options| options.cohort = Cohort::Direct),
        w.run_with(|options| options.session = Some("chat-a".to_owned())),
        w.run_with(|options| options.session = Some("chat-d".to_owned())),
    ];
    assert!(reports[0].sessions[0].total > 0);
    assert_eq!(reports[3].sessions[0].session_id, "chat-d");
    for report in &reports {
        for output in [render_markdown(report), render_json(report).unwrap()] {
            assert!(!output.contains("MARK"), "{output}");
        }
    }
    assert!(!w.one_error("worker-1").contains("MARK"));
}
