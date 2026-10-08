//! The `bridge` MCP server: newline-delimited JSON-RPC 2.0 over stdio.
//!
//! Runs as `bridged --bridge-mcp-visualize`, spawned by the harness that
//! attached it. Stateless apart from the repeat guard, which lives exactly as
//! long as the harness session's connection. A malformed line gets a parse
//! error and the server keeps serving; stdin EOF ends it.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use super::guard::RepeatGuard;
use super::{lint, spec, tool};

/// Protocol revisions this server speaks. It answers with the client's
/// requested revision when it knows it, and its newest otherwise.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// The view the MCP Apps host renders. Built by `bun run build:visual` from
/// `src/mcp-apps/visual/` and checked in, so the daemon binary carries it.
pub const VIEW_HTML: &str = include_str!("../../../../src/mcp-apps/visual/generated/visual.html");

#[derive(Debug, Default)]
pub struct VisualServer {
    guard: RepeatGuard,
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
            let nodes = block.pointer("/graph/nodes").and_then(Value::as_array).map_or(0, Vec::len);
            format!("{nodes} nodes")
        }
        _ => {
            let items = ["items", "rows", "options", "pros"]
                .iter()
                .find_map(|key| block.pointer(&format!("/content/{key}")).and_then(Value::as_array))
                .map_or(1, Vec::len);
            format!("{items} item{}", if items == 1 { "" } else { "s" })
        }
    };
    format!("{form}, {count}")
}

fn refusal(errors: &[spec::SpecError]) -> Value {
    let mut text = String::from("The visual was not drawn. Fix every item below and call visualize once more:\n");
    for error in errors.iter().take(spec::MAX_REPORTED) {
        let path = if error.path.is_empty() { "spec" } else { error.path.as_str() };
        text.push_str(&format!("- {path}: {}\n", error.message));
    }
    if errors.len() > spec::MAX_REPORTED {
        text.push_str(&format!("- and {} more like these\n", errors.len() - spec::MAX_REPORTED));
    }
    json!({"content": [{"type": "text", "text": text.trim_end()}], "isError": true})
}

impl VisualServer {
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

    /// Handle one JSON-RPC message. `None` for notifications.
    pub fn handle(&mut self, message: &Value) -> Option<Value> {
        let Some(object) = message.as_object() else {
            return Some(failure(&Value::Null, -32600, "expected a JSON-RPC request object"));
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
                        "instructions": "Use visualize to draw a chart, diagram or sourced findings inline when a visual carries the answer better than prose. Most turns need none."
                    }),
                )
            }
            "ping" => reply(id, json!({})),
            "tools/list" => reply(id, json!({"tools": [tool::definition()]})),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or("");
                if name != tool::TOOL_NAME {
                    failure(id, -32602, &format!("unknown tool \"{name}\""))
                } else {
                    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
                    reply(id, self.visualize(&arguments))
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

    #[test]
    fn initialize_echoes_a_supported_protocol_version_and_declares_tools_and_resources() {
        let mut server = VisualServer::default();
        let response = server.handle(&request(1, "initialize", json!({"protocolVersion": "2025-06-18"}))).unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert!(response["result"]["capabilities"]["resources"].is_object());
        let response = server.handle(&request(2, "initialize", json!({"protocolVersion": "1999-01-01"}))).unwrap();
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
            .handle(&request(1, "tools/call", json!({"name": "visualize", "arguments": bar(three())})))
            .unwrap();
        let result = &response["result"];
        assert_eq!(result["isError"], false);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"Spend\" (bar, 3 rows)"), "{text}");
        assert!(text.contains("Do not restate"), "{text}");
        assert!(result.get("structuredContent").is_none(), "the spec is already in the input; do not echo it back");
    }

    #[test]
    fn tools_call_invalid_returns_is_error_listing_every_path() {
        let mut server = VisualServer::default();
        let mut spec = bar(three());
        spec["version"] = json!(3);
        spec["blocks"][0]["colors"] = json!({"a": "#fff"});
        let response = server
            .handle(&request(1, "tools/call", json!({"name": "visualize", "arguments": spec})))
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
            server.handle(&request(1, "tools/call", json!({"name": "visualize", "arguments": spec}))).unwrap()["result"].clone()
        };
        assert_eq!(call(&mut server, bar(json!([{"k": "a", "v": 1}])))["isError"], true);
        assert_eq!(call(&mut server, bar(three()))["isError"], false);
        let again = call(&mut server, bar(three()));
        assert_eq!(again["isError"], true);
        assert!(again["content"][0]["text"].as_str().unwrap().contains("already seen"));
    }

    #[test]
    fn tools_call_unknown_tool_is_a_jsonrpc_error() {
        let mut server = VisualServer::default();
        let response = server.handle(&request(1, "tools/call", json!({"name": "paint"}))).unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn resources_list_has_ui_bridge_visual() {
        let mut server = VisualServer::default();
        let response = server.handle(&request(1, "resources/list", json!({}))).unwrap();
        assert_eq!(response["result"]["resources"][0]["uri"], "ui://bridge/visual");
        assert_eq!(response["result"]["resources"][0]["mimeType"], "text/html;profile=mcp-app");
    }

    #[test]
    fn resources_read_returns_the_bundled_view_as_mcp_app_html() {
        let mut server = VisualServer::default();
        let response = server.handle(&request(1, "resources/read", json!({"uri": "ui://bridge/visual"}))).unwrap();
        let content = &response["result"]["contents"][0];
        assert_eq!(content["mimeType"], "text/html;profile=mcp-app");
        assert!(content["text"].as_str().unwrap().starts_with("<!doctype html>"));
        let missing = server.handle(&request(2, "resources/read", json!({"uri": "ui://other"}))).unwrap();
        assert!(missing["error"].is_object());
    }

    #[test]
    fn notifications_get_no_response_and_unknown_methods_get_32601() {
        let mut server = VisualServer::default();
        assert!(server.handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let response = server.handle(&request(9, "sampling/createMessage", json!({}))).unwrap();
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
}
