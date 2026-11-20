//! The `bridge` MCP server: newline-delimited JSON-RPC 2.0 over stdio.
//!
//! Runs as `bridged --bridge-mcp-visualize`, spawned by the harness that
//! attached it. Stateless apart from the repeat guard, which lives exactly as
//! long as the harness session's connection. A malformed line gets a parse
//! error and the server keeps serving; stdin EOF ends it.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use super::guard::RepeatGuard;
use super::notes_tools::NotesLink;
use super::{lint, spec, tool};
use crate::tool_limit;

/// Protocol revisions this server speaks. It answers with the client's
/// requested revision when it knows it, and its newest otherwise.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The view the MCP Apps host renders. Built by `bun run build:visual` from
/// `src/mcp-apps/visual/` and checked in, so the daemon binary carries it.
pub const VIEW_HTML: &str = include_str!("../../../../src/mcp-apps/visual/generated/visual.html");

#[derive(Default)]
pub struct VisualServer {
    guard: RepeatGuard,
    /// The chat's notes, when the server was launched for one.
    notes: Option<NotesLink>,
}

fn reply(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn failure(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn block_summary(block: &Value) -> String {
    let form = block["form"].as_str().unwrap_or("?");
    let count = match block["family"].as_str() {
        Some("chart") => {
            let rows = spec::chart_rows(block).len();
            format!("{rows} row{}", if rows == 1 { "" } else { "s" })
        }
        Some("diagram") => {
            let nodes = block
                .pointer("/graph/nodes")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("{nodes} nodes")
        }
        _ => {
            let items = ["items", "rows", "options", "pros"]
                .iter()
                .find_map(|key| {
                    block
                        .pointer(&format!("/content/{key}"))
                        .and_then(Value::as_array)
                })
                .map_or(1, Vec::len);
            format!("{items} item{}", if items == 1 { "" } else { "s" })
        }
    };
    format!("{form}, {count}")
}

fn refusal(errors: &[spec::SpecError]) -> Value {
    let mut text = String::from(
        "The visual was not drawn. Fix every item below and call visualize once more:\n",
    );
    for error in errors.iter().take(spec::MAX_REPORTED) {
        let path = if error.path.is_empty() {
            "spec"
        } else {
            error.path.as_str()
        };
        text.push_str(&format!("- {path}: {}\n", error.message));
    }
    if errors.len() > spec::MAX_REPORTED {
        text.push_str(&format!(
            "- and {} more like these\n",
            errors.len() - spec::MAX_REPORTED
        ));
    }
    json!({"content": [{"type": "text", "text": text.trim_end()}], "isError": true})
}

/// Whether `name` is a chat tool this server forwards to the core, which
/// checks the chat's token and access on every call.
pub fn forwards_to_core(name: &str) -> bool {
    name.starts_with("notes_")
        || name.starts_with("memory_")
        || name.starts_with("outgoing_")
        || name.starts_with("settings_")
        || name.starts_with(crate::history_tools::PREFIX)
        || name == crate::notes::tool::NOTIFY_USER
        || name == "apps_draft"
        || name == "apps_open"
}

impl VisualServer {
    pub fn with_notes(notes: Option<NotesLink>) -> Self {
        Self { guard: RepeatGuard::default(), notes }
    }

    /// Run one `visualize` call: validate, lint, guard, then summarise.
    pub fn visualize(&mut self, arguments: &Value) -> Value {
        let errors = spec::validate(arguments);
        if !errors.is_empty() {
            return refusal(&errors);
        }
        let errors = lint::lint(arguments);
        if !errors.is_empty() {
            return refusal(&errors);
        }
        if let Err(error) = self.guard.check(arguments) {
            return refusal(&[error]);
        }
        self.guard.remember(arguments);
        let title = arguments["title"].as_str().unwrap_or("");
        let blocks: Vec<String> = arguments["blocks"]
            .as_array()
            .map(|blocks| blocks.iter().map(block_summary).collect())
            .unwrap_or_default();
        let text = format!(
            "Shown to the user inline: \"{title}\" ({}). Do not restate its values in prose; say what matters and why.",
            blocks.join("; ")
        );
        json!({"content": [{"type": "text", "text": text}], "isError": false})
    }

    /// What the model reads about this server: visualize, then a paragraph
    /// for each group of chat tools the core lists for this chat.
    fn instructions(&self) -> String {
        let visualize = "Use visualize to draw a chart, diagram or sourced findings inline when a visual carries the answer better than prose. Most turns need none.";
        match self.notes.as_ref().map(NotesLink::instructions).filter(|text| !text.is_empty()) {
            Some(chat_tools) => format!("{visualize} {chat_tools}"),
            None => visualize.to_owned(),
        }
    }

    /// Handle one JSON-RPC message. `None` for notifications.
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(failure(
                &Value::Null,
                -32600,
                "expected a JSON-RPC request object",
            ));
        };
        let method = object.get("method").and_then(Value::as_str).unwrap_or("");
        let Some(id) = object.get("id") else {
            // Notifications (initialized, cancelled, …) need no answer.
            return None;
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => {
                let requested = params["protocolVersion"].as_str().unwrap_or("");
                let version = PROTOCOL_VERSIONS
                    .iter()
                    .find(|known| **known == requested)
                    .copied()
                    .unwrap_or(PROTOCOL_VERSIONS[0]);
                reply(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {
                            "tools": {"listChanged": false},
                            "resources": {"listChanged": false},
                            "extensions": {"io.modelcontextprotocol/ui": {"mimeTypes": [tool::VIEW_MIME]}}
                        },
                        "serverInfo": {"name": tool::SERVER_NAME, "title": "Bridge", "version": env!("CARGO_PKG_VERSION")},
                        "instructions": self.instructions()
                    }),
                )
            }
            "ping" => reply(id, json!({})),
            "tools/list" => {
                let mut tools = vec![tool::definition()];
                if let Some(notes) = &self.notes {
                    tools.extend(notes.definitions());
                }
                reply(id, json!({"tools": tools}))
            }
            // Each reply passes the limiter here, the one point that every
            // harness reads through. The core and its loopback route return the
            // full result.
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("");
                if let (Some(notes), true) = (&self.notes, forwards_to_core(name)) {
                    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    reply(id, tool_limit::bound_reply(name, &arguments, notes.call(name, &arguments)))
                } else if name != tool::TOOL_NAME {
                    failure(id, -32602, &format!("unknown tool \"{name}\""))
                } else {
                    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
                    let result = self.visualize(&arguments);
                    reply(id, tool_limit::bound_reply(name, &arguments, result))
                }
            }
            "resources/list" => reply(
                id,
                json!({"resources": [{
                    "uri": tool::VIEW_URI,
                    "name": "visual",
                    "title": "Bridge visual",
                    "mimeType": tool::VIEW_MIME
                }]}),
            ),
            "resources/templates/list" => reply(id, json!({"resourceTemplates": []})),
            "resources/read" => {
                if params["uri"].as_str() == Some(tool::VIEW_URI) {
                    reply(
                        id,
                        json!({"contents": [{
                            "uri": tool::VIEW_URI,
                            "mimeType": tool::VIEW_MIME,
                            "text": VIEW_HTML,
                            "_meta": {"ui": {"prefersBorder": false, "csp": {}}}
                        }]}),
                    )
                } else {
                    failure(id, -32002, "resource not found")
                }
            }
            "prompts/list" => reply(id, json!({"prompts": []})),
            _ => failure(id, -32601, &format!("method not found: {method}")),
        })
    }

    /// Serve until EOF.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(&line) {
                Ok(message) => self.handle(&message),
                Err(_) => Some(failure(&Value::Null, -32700, "parse error")),
            };
            if let Some(response) = response {
                serde_json::to_writer(&mut output, &response)?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp_apps::notes_tools;

    fn request(id: u64, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    fn bar(values: Value) -> Value {
        json!({
            "version": 1, "title": "Spend",
            "sources": [{"id": "s", "kind": "user", "ref": "the user's message"}],
            "blocks": [{"family": "chart", "form": "bar", "sourceIds": ["s"], "vegaLite": {
                "mark": "bar", "data": {"values": values},
                "encoding": {"x": {"field": "k", "type": "nominal"}, "y": {"field": "v", "type": "quantitative"}}
            }}]
        })
    }

    fn three() -> Value {
        json!([{"k": "a", "v": 1}, {"k": "b", "v": 2}, {"k": "c", "v": 3}])
    }

    struct FakeCore {
        access: Option<&'static str>,
        calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }

    impl notes_tools::NotesTransport for FakeCore {
        fn get(&self, path: &str) -> Result<Value, (u16, String)> {
            assert_eq!(path, "tools");
            match self.access {
                Some("write") => Ok(json!({"access": "write", "tools": ["notes_search", "notes_read", "notes_create", "notes_edit", "notes_append", "notes_set", "notes_draw"]})),
                Some(_) => Ok(json!({"access": "read", "tools": ["notes_search", "notes_read"]})),
                None => Err((0, "Bridge is not reachable".into())),
            }
        }
        fn post(&self, path: &str, body: &Value) -> Result<Value, (u16, String)> {
            self.calls.lock().unwrap().push((path.to_owned(), body.clone()));
            if self.access == Some("read") && path != "notes_search" && path != "notes_read" {
                return Err((403, "this chat can read notes but not change them".into()));
            }
            Ok(json!({"created": {"id": "p1", "title": body["title"]}}))
        }
    }

    fn with_core(access: Option<&'static str>) -> (VisualServer, std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let link = NotesLink::new(Box::new(FakeCore { access, calls: calls.clone() }));
        (VisualServer::with_notes(Some(link)), calls)
    }

    fn tool_names(server: &mut VisualServer) -> Vec<String> {
        let response = server.handle(&request(1, "tools/list", json!({}))).unwrap();
        response["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_owned()).collect()
    }

    struct MemoryCore {
        calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }

    impl notes_tools::NotesTransport for MemoryCore {
        fn get(&self, _path: &str) -> Result<Value, (u16, String)> {
            Ok(json!({"access": "write", "tools": ["notes_search", "notes_read", "memory_search", "memory_propose"]}))
        }
        fn post(&self, path: &str, body: &Value) -> Result<Value, (u16, String)> {
            self.calls.lock().unwrap().push((path.to_owned(), body.clone()));
            Ok(json!({"written": true, "proposed": {"id": "m1", "status": "proposed"}}))
        }
    }

    struct DraftCore {
        calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    }

    impl notes_tools::NotesTransport for DraftCore {
        fn get(&self, _path: &str) -> Result<Value, (u16, String)> {
            Ok(json!({"access": "write", "tools": ["notes_search", "notes_read", "outgoing_draft"]}))
        }
        fn post(&self, path: &str, body: &Value) -> Result<Value, (u16, String)> {
            self.calls.lock().unwrap().push((path.to_owned(), body.clone()));
            Ok(json!({"id": "o1", "state": "draft", "rendered": body["body"]}))
        }
    }

    #[test]
    fn outgoing_draft_is_listed_from_the_core_and_forwarded_to_it() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut server = VisualServer::with_notes(Some(NotesLink::new(Box::new(DraftCore { calls: calls.clone() }))));
        assert_eq!(tool_names(&mut server), vec!["visualize", "notes_search", "notes_read", "outgoing_draft"]);
        let arguments = json!({"to": {"clipboard": "plain"}, "body": "hi", "reason": "r"});
        let drafted = server.handle(&request(2, "tools/call", json!({"name": "outgoing_draft", "arguments": arguments}))).unwrap();
        assert_eq!(drafted["result"]["structuredContent"]["id"], "o1");
        assert_eq!(calls.lock().unwrap()[0], ("outgoing_draft".to_owned(), arguments));
    }

    #[test]
    fn memory_tools_are_listed_from_the_core_and_forwarded_to_it() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let link = NotesLink::new(Box::new(MemoryCore { calls: calls.clone() }));
        let mut server = VisualServer::with_notes(Some(link));
        assert_eq!(tool_names(&mut server), vec!["visualize", "notes_search", "notes_read", "memory_search", "memory_propose"]);
        let proposed = server
            .handle(&request(2, "tools/call", json!({"name": "memory_propose", "arguments": {"body": "Never push to main", "kind": "constraint", "rationale": "said so"}})))
            .unwrap();
        assert_eq!(proposed["result"]["isError"], false);
        assert_eq!(proposed["result"]["structuredContent"]["proposed"]["status"], "proposed");
        assert_eq!(calls.lock().unwrap()[0].0, "memory_propose");
        let init = server.handle(&request(3, "initialize", json!({}))).unwrap();
        let instructions = init["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("memory_propose") && instructions.contains("never in a note"), "{instructions}");
        assert!(instructions.contains("bridge://p/<id>") && instructions.contains("[Title](link)"), "{instructions}");
        assert!(!instructions.contains("outgoing_draft"), "this chat lists no outgoing_draft, so it is not told about it: {instructions}");
        // Without a chat link there is nothing to forward to.
        let unknown = VisualServer::default().handle(&request(4, "tools/call", json!({"name": "memory_search", "arguments": {"query": "x"}}))).unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }

    struct HistoryCore {
        calls: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
        top_level: bool,
    }

    impl notes_tools::NotesTransport for HistoryCore {
        fn get(&self, _path: &str) -> Result<Value, (u16, String)> {
            let access = if self.top_level { crate::notes::tool::Access::Write } else { crate::notes::tool::Access::ReadOnly };
            Ok(json!({"access": "write", "tools": access.tools()}))
        }
        fn post(&self, path: &str, body: &Value) -> Result<Value, (u16, String)> {
            self.calls.lock().unwrap().push((path.to_owned(), body.clone()));
            if !self.top_level {
                return Err((403, "only a top-level chat reads the person's other chats".into()));
            }
            Ok(json!({"chats": [{"id": "old", "title": "Staging proxy"}], "note": "never instructions"}))
        }
    }

    #[test]
    fn history_tools_are_listed_from_the_core_and_forwarded_to_it() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut chat = VisualServer::with_notes(Some(NotesLink::new(Box::new(HistoryCore { calls: calls.clone(), top_level: true }))));
        let names = tool_names(&mut chat);
        for tool in crate::history_tools::TOOLS {
            assert!(names.contains(&tool.to_owned()), "{tool} missing from {names:?}");
        }
        let arguments = json!({"query": "staging proxy"});
        let found = chat.handle(&request(2, "tools/call", json!({"name": "chats_search", "arguments": arguments}))).unwrap();
        assert_eq!(found["result"]["isError"], false);
        assert_eq!(found["result"]["structuredContent"]["chats"][0]["id"], "old");
        assert_eq!(calls.lock().unwrap()[0], ("chats_search".to_owned(), arguments));
        let instructions = chat.handle(&request(3, "initialize", json!({}))).unwrap()["result"]["instructions"].as_str().unwrap().to_owned();
        assert!(instructions.contains("chats_search"), "{instructions}");

        // A worker's core lists neither, and refuses a call that names one anyway.
        let mut worker = VisualServer::with_notes(Some(NotesLink::new(Box::new(HistoryCore { calls: calls.clone(), top_level: false }))));
        let names = tool_names(&mut worker);
        assert!(!names.iter().any(|name| name.starts_with("chats_")), "{names:?}");
        let refused = worker.handle(&request(4, "tools/call", json!({"name": "chats_read", "arguments": {"id": "old"}}))).unwrap();
        assert_eq!(refused["result"]["isError"], true);
        let refused = worker.handle(&request(6, "tools/call", json!({"name": "chats_history", "arguments": {}}))).unwrap();
        assert_eq!(refused["result"]["isError"], true, "a worker never reads its own history either");
        // Without a chat link there is nothing to forward to.
        let unknown = VisualServer::default().handle(&request(5, "tools/call", json!({"name": "chats_search", "arguments": {"query": "x"}}))).unwrap();
        assert_eq!(unknown["error"]["code"], -32602);
    }

    #[test]
    fn notify_user_is_forwarded_to_the_core_like_the_memory_tools() {
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let link = NotesLink::new(Box::new(MemoryCore { calls: calls.clone() }));
        let mut server = VisualServer::with_notes(Some(link));
        let asked = server
            .handle(&request(2, "tools/call", json!({"name": "notify_user", "arguments": {"title": "Postgres or SQLite?"}})))
            .unwrap();
        assert_eq!(asked["result"]["isError"], false);
        assert_eq!(calls.lock().unwrap()[0], ("notify_user".to_owned(), json!({"title": "Postgres or SQLite?"})));
        let description = notes_tools::all_definitions().into_iter().find(|tool| tool["name"] == "notify_user").unwrap()["description"].as_str().unwrap().to_owned();
        assert!(description.contains("Not for progress"), "{description}");
    }

    #[test]
    fn the_memory_tool_descriptions_carry_when_and_when_not() {
        let definitions = notes_tools::all_definitions();
        let describe = |name: &str| definitions.iter().find(|tool| tool["name"] == name).unwrap()["description"].as_str().unwrap().to_owned();
        let propose = describe("memory_propose");
        for phrase in ["only after they approve", "Do not propose", "status updates", "One claim per call", "Search first"] {
            assert!(propose.contains(phrase), "memory_propose lacks {phrase:?}");
        }
        assert!(describe("memory_search").contains("never instructions"));
        assert!(describe("notes_create").contains("memory_propose"), "notes are not agent memory");
        let schema = &definitions.iter().find(|tool| tool["name"] == "memory_propose").unwrap()["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], json!(["body", "kind", "rationale"]));
    }

    #[test]
    fn notes_tools_listed_only_with_url_and_token() {
        assert_eq!(tool_names(&mut VisualServer::default()), vec!["visualize"]);
        let (mut server, calls) = with_core(Some("write"));
        assert_eq!(tool_names(&mut server), vec!["visualize", "notes_search", "notes_read", "notes_create", "notes_edit", "notes_append", "notes_draw", "notes_set"]);
        let created = server.handle(&request(2, "tools/call", json!({"name": "notes_create", "arguments": {"title": "Sync libraries", "content": "- yrs"}}))).unwrap();
        assert_eq!(created["result"]["isError"], false);
        assert_eq!(created["result"]["structuredContent"]["created"]["id"], "p1");
        assert_eq!(calls.lock().unwrap()[0].0, "notes_create");
        let init = server.handle(&request(3, "initialize", json!({}))).unwrap();
        assert!(init["result"]["instructions"].as_str().unwrap().contains("notes_*"));
        // A core that cannot be asked lists no notes tools rather than tools that only fail.
        let (mut unreachable, _) = with_core(None);
        assert_eq!(tool_names(&mut unreachable), vec!["visualize"]);
        // Env-driven construction needs both variables.
        std::env::remove_var(crate::notes::tool::ENV_URL);
        assert!(NotesLink::from_env().is_none());
    }

    #[test]
    fn read_only_access_lists_only_read_tools() {
        let (mut server, _) = with_core(Some("read"));
        assert_eq!(tool_names(&mut server), vec!["visualize", "notes_search", "notes_read"]);
        let refused = server.handle(&request(2, "tools/call", json!({"name": "notes_create", "arguments": {"title": "x"}}))).unwrap();
        assert_eq!(refused["result"]["isError"], true);
        assert!(refused["result"]["content"][0]["text"].as_str().unwrap().contains("not change"));
    }

    #[test]
    fn initialize_echoes_a_supported_protocol_version_and_declares_tools_and_resources() {
        let mut server = VisualServer::default();
        let response = server
            .handle(&request(
                1,
                "initialize",
                json!({"protocolVersion": "2025-06-18"}),
            ))
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"]["resources"].is_object());
        let response = server
            .handle(&request(
                2,
                "initialize",
                json!({"protocolVersion": "1999-01-01"}),
            ))
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    #[test]
    fn tools_list_has_visualize_with_ui_meta_and_read_only_annotations() {
        let mut server = VisualServer::default();
        let response = server.handle(&request(1, "tools/list", json!({}))).unwrap();
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["name"], "visualize");
        assert_eq!(tools[0]["_meta"]["ui"]["resourceUri"], "ui://bridge/visual");
        assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    }

    #[test]
    fn tools_call_valid_returns_a_summary() {
        let mut server = VisualServer::default();
        let response = server
            .handle(&request(
                1,
                "tools/call",
                json!({"name": "visualize", "arguments": bar(three())}),
            ))
            .unwrap();
        let result = &response["result"];
        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"Spend\" (bar, 3 rows)"), "{text}");
        assert!(text.contains("Do not restate"), "{text}");
        assert!(
            result.get("structuredContent").is_none(),
            "the spec is already in the input; do not echo it back"
        );
    }

    #[test]
    fn tools_call_invalid_returns_is_error_listing_every_path() {
        let mut server = VisualServer::default();
        let mut spec = bar(three());
        spec["version"] = json!(3);
        spec["blocks"][0]["colors"] = json!({"a": "#fff"});
        let response = server
            .handle(&request(
                1,
                "tools/call",
                json!({"name": "visualize", "arguments": spec}),
            ))
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("- version:"), "{text}");
        assert!(text.contains("- blocks[0].colors.a:"), "{text}");
    }

    #[test]
    fn a_lint_failure_and_a_repeat_are_refusals_too() {
        let mut server = VisualServer::default();
        let call = |server: &mut VisualServer, spec: Value| {
            server
                .handle(&request(
                    1,
                    "tools/call",
                    json!({"name": "visualize", "arguments": spec}),
                ))
                .unwrap()["result"]
                .clone()
        };
        assert_eq!(
            call(&mut server, bar(json!([{"k": "a", "v": 1}])))["isError"],
            true
        );
        assert_eq!(call(&mut server, bar(three()))["isError"], false);
        let again = call(&mut server, bar(three()));
        assert_eq!(again["isError"], true);
        assert!(again["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already seen"));
    }

    #[test]
    fn tools_call_unknown_tool_is_a_jsonrpc_error() {
        let mut server = VisualServer::default();
        let response = server
            .handle(&request(1, "tools/call", json!({"name": "paint"})))
            .unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn resources_list_has_ui_bridge_visual() {
        let mut server = VisualServer::default();
        let response = server
            .handle(&request(1, "resources/list", json!({})))
            .unwrap();
        assert_eq!(
            response["result"]["resources"][0]["uri"],
            "ui://bridge/visual"
        );
        assert_eq!(
            response["result"]["resources"][0]["mimeType"],
            "text/html;profile=mcp-app"
        );
    }

    #[test]
    fn resources_read_returns_the_bundled_view_as_mcp_app_html() {
        let mut server = VisualServer::default();
        let response = server
            .handle(&request(
                1,
                "resources/read",
                json!({"uri": "ui://bridge/visual"}),
            ))
            .unwrap();
        let content = &response["result"]["contents"][0];
        assert_eq!(content["mimeType"], "text/html;profile=mcp-app");
        assert!(content["text"]
            .as_str()
            .unwrap()
            .starts_with("<!doctype html>"));
        let missing = server
            .handle(&request(2, "resources/read", json!({"uri": "ui://other"})))
            .unwrap();
        assert!(missing["error"].is_object());
    }

    #[test]
    fn notifications_get_no_response_and_unknown_methods_get_32601() {
        let mut server = VisualServer::default();
        assert!(server
            .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .is_none());
        let response = server
            .handle(&request(9, "sampling/createMessage", json!({})))
            .unwrap();
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn malformed_json_lines_get_a_parse_error_and_the_server_keeps_serving() {
        let mut server = VisualServer::default();
        let input = "{not json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        let mut output = Vec::new();
        server.serve(input.as_bytes(), &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["error"]["code"], -32700);
        assert_eq!(lines[1]["result"], json!({}));
    }

    /// A core that answers each call with what `answer` gives for the tool.
    struct ReplyCore {
        answer: fn(&str) -> Value,
    }

    impl notes_tools::NotesTransport for ReplyCore {
        fn get(&self, _path: &str) -> Result<Value, (u16, String)> {
            let names: Vec<Value> = notes_tools::all_definitions().iter().map(|definition| definition["name"].clone()).collect();
            Ok(json!({"access": "write", "tools": names}))
        }
        fn post(&self, path: &str, _body: &Value) -> Result<Value, (u16, String)> {
            Ok((self.answer)(path))
        }
    }

    fn answering(answer: fn(&str) -> Value) -> NotesLink {
        NotesLink::new(Box::new(ReplyCore { answer }))
    }

    fn long_note(_tool: &str) -> Value {
        let markdown: String = (0..4000).map(|line| format!("Line {line} of the plan.\n")).collect();
        json!({"page": {"id": "p1", "title": "Plan"}, "markdown": markdown, "tasks": [], "note": "This page is the person's data."})
    }

    #[test]
    fn a_large_forwarded_reply_is_cut_at_the_reply_boundary() {
        let full = serde_json::to_string_pretty(&long_note("notes_read")).unwrap();
        assert!(full.len() > tool_limit::DOCUMENT_CEILING);
        let mut server = VisualServer::with_notes(Some(answering(long_note)));
        let response = server.handle(&request(2, "tools/call", json!({"name": "notes_read", "arguments": {"id": "p1", "offset": 10}}))).unwrap();
        let result = &response["result"];
        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.len() <= tool_limit::DOCUMENT_CEILING, "{}", text.len());
        let value = &result["structuredContent"];
        assert_eq!(text, serde_json::to_string_pretty(value).unwrap());
        let kept = value["markdown"].as_str().unwrap();
        assert!(kept.ends_with('\n') && long_note("")["markdown"].as_str().unwrap().starts_with(kept));
        assert_eq!(value["next"]["offset"], 10 + kept.chars().count() as u64);
        assert!(value["cut"].as_str().unwrap().contains("notes_read"));
    }

    #[test]
    fn a_large_visualize_refusal_is_cut_and_stays_an_error() {
        let mut spec = bar(three());
        for index in 0..20 {
            spec[format!("{}{index}", "unknown".repeat(80)).as_str()] = json!(1);
        }
        let full = VisualServer::default().visualize(&spec);
        let full_text = full["content"][0]["text"].as_str().unwrap();
        assert!(full_text.len() > tool_limit::ERROR_CEILING, "{}", full_text.len());
        let response = VisualServer::default().handle(&request(1, "tools/call", json!({"name": "visualize", "arguments": spec}))).unwrap();
        let result = &response["result"];
        assert_eq!(result["isError"], true);
        assert!(result.get("structuredContent").is_none());
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.len() <= tool_limit::ERROR_CEILING, "{}", text.len());
        assert!(text.starts_with("The visual was not drawn."), "{text}");
        assert!(text.ends_with(&format!("The full text has {} bytes.", full_text.len())), "{text}");
    }

    #[test]
    fn a_small_reply_of_each_tool_keeps_its_bytes() {
        fn small(tool: &str) -> Value {
            json!({"tool": tool, "pages": [{"id": "p1", "title": "Plan \"A\"", "preview": "é漢😀\n\u{1}"}], "records": [], "markdown": "# Plan\n", "total": 1})
        }
        let arguments = json!({"id": "p1"});
        for definition in notes_tools::all_definitions() {
            let name = definition["name"].as_str().unwrap();
            let expected = answering(small).call(name, &arguments);
            let response = VisualServer::with_notes(Some(answering(small))).handle(&request(1, "tools/call", json!({"name": name, "arguments": arguments}))).unwrap();
            assert_eq!(serde_json::to_string(&response["result"]).unwrap(), serde_json::to_string(&expected).unwrap(), "{name}");
        }
        let expected = VisualServer::default().visualize(&bar(three()));
        let response = VisualServer::default().handle(&request(1, "tools/call", json!({"name": "visualize", "arguments": bar(three())}))).unwrap();
        assert_eq!(serde_json::to_string(&response["result"]).unwrap(), serde_json::to_string(&expected).unwrap());
        // A small error keeps its bytes too.
        let (_, calls) = with_core(Some("read"));
        let refused = NotesLink::new(Box::new(FakeCore { access: Some("read"), calls })).call("notes_create", &json!({"title": "x"}));
        let (mut server, _) = with_core(Some("read"));
        let response = server.handle(&request(2, "tools/call", json!({"name": "notes_create", "arguments": {"title": "x"}}))).unwrap();
        assert_eq!(serde_json::to_string(&response["result"]).unwrap(), serde_json::to_string(&refused).unwrap());
    }

    #[test]
    fn a_large_note_pages_through_the_helper_and_the_route() {
        use crate::notes::tool::{serve_for, token_for, verify, AUTH_HEADER, URL_PREFIX};
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(crate::runtime::BridgeCore::for_tests(dir.path()));
        core.db.lock().unwrap().execute("INSERT INTO sessions(id,harness,label,status,kind,title) VALUES('chat','claude','Chat','ready','direct','Plan')", []).unwrap();
        let content: String = (0..3000).map(|line| format!("Line {line} of the plan: é漢😀 \"quoted\" back\\slash\ttab.\n")).collect();
        let page = crate::notes::service::CreatePage { title: Some("Plan".into()), ..Default::default() };
        let id = core.notes.create(page, crate::notes::service::PERSON).unwrap().id;
        // One append is one insert. A body in `create` is a diff of each
        // character, which is slow for this size in a debug build.
        let append = bridge_protocol::messages::NoteAppend { content, position: "end".into() };
        core.notes.edit(crate::notes::service::EditPage { id: id.clone(), append: Some(append), ..Default::default() }, crate::notes::service::PERSON).unwrap();
        let (status, body) = serve_for(&core, "chat", "notes_read", json!({"id": id}).to_string().as_bytes());
        assert_eq!(status, 200, "{body}");
        let full = serde_json::from_str::<Value>(&body).unwrap()["markdown"].as_str().unwrap().to_owned();
        assert!(full.len() > 3 * tool_limit::DOCUMENT_CEILING, "{}", full.len());

        // The loopback route of `respond`, against this test's core: `install`
        // holds one core for the whole process.
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let (serving, routed) = (server.clone(), core.clone());
        std::thread::spawn(move || {
            for mut request in serving.incoming_requests() {
                let route = request.url().strip_prefix(URL_PREFIX).unwrap_or_default().to_owned();
                let token = request.headers().iter().find(|header| header.field.equiv(AUTH_HEADER)).map(|header| header.value.as_str().to_owned()).unwrap_or_default();
                let mut body = Vec::new();
                request.as_reader().read_to_end(&mut body).unwrap();
                let (session, tool) = route.split_once('/').unwrap_or_default();
                let (status, text) = if verify(session, &token) { serve_for(&routed, session, tool, &body) } else { (401, json!({"error": "not authorized"}).to_string()) };
                request.respond(tiny_http::Response::from_string(text).with_status_code(status)).unwrap();
            }
        });

        let link = NotesLink::over_http(format!("http://127.0.0.1:{port}{URL_PREFIX}chat"), token_for("chat")).unwrap();
        let mut helper = VisualServer::with_notes(Some(link));
        assert!(tool_names(&mut helper).contains(&"notes_read".to_owned()));
        let mut arguments = json!({"id": id});
        let mut joined = String::new();
        let mut replies = 0;
        loop {
            replies += 1;
            assert!(replies < 50, "never ends");
            let response = helper.handle(&request(replies, "tools/call", json!({"name": "notes_read", "arguments": arguments}))).unwrap();
            let result = &response["result"];
            assert_eq!(result["isError"], false, "{result}");
            let text = result["content"][0]["text"].as_str().unwrap();
            assert!(text.len() <= tool_limit::DOCUMENT_CEILING, "reply {replies}: {} bytes", text.len());
            let value = &result["structuredContent"];
            assert_eq!(text, serde_json::to_string_pretty(value).unwrap());
            joined.push_str(value["markdown"].as_str().unwrap());
            match value.get("next") {
                Some(next) => {
                    assert!(value["cut"].as_str().unwrap().contains("notes_read"), "{value}");
                    arguments["offset"] = next["offset"].clone();
                }
                None => break,
            }
        }
        assert!(replies > 3, "{replies}");
        assert_eq!(joined, full);
    }
}
