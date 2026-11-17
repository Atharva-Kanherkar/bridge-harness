//! Codex's half of personal-app control, over `codex app-server` JSON-RPC.
//!
//! Inventory is `mcpServerStatus/list` and a read is `mcpServer/tool/call`;
//! neither starts a model turn. Codex applies no approval policy to a direct
//! call, so authority is decided before a frame is written here: by the
//! router, which only sends allowlisted reads.
//!
//! Requests carry string ids in Bridge's own namespace, so their responses
//! are recognised and consumed before transcript handling: [`translate`] turns
//! one into the same `app_control_result` frame the Claude sidecar writes,
//! and [`super::control::deliver`] does the rest.

use crate::BridgeError;
use serde_json::{json, Map, Value};

const ID_PREFIX: &str = "bridge-app:";

fn invalid(text: &str) -> BridgeError {
    BridgeError::Invalid(text.into())
}

/// The JSON-RPC request for one app-control request, or a refusal.
pub fn request_frame(id: &str, thread_id: &str, request: &Value) -> Result<Value, BridgeError> {
    let server = request["server"]
        .as_str()
        .filter(|server| !server.is_empty() && server.len() <= 100)
        .ok_or_else(|| invalid("invalid preview request"))?;
    match request["kind"].as_str() {
        Some("inventory") => Ok(json!({
            "id": format!("{ID_PREFIX}inventory:{id}"),
            "method": "mcpServerStatus/list",
            "params": {"serverName": server, "threadId": thread_id, "detail": "toolsAndAuthOnly"},
        })),
        // A write reaches here only from the router, after the person
        // approved these exact arguments: Codex applies no approval itself.
        Some(kind @ ("read" | "write")) => {
            let tool = request["tool"]
                .as_str()
                .filter(|tool| !tool.is_empty() && tool.len() <= 64)
                .ok_or_else(|| invalid("invalid preview request"))?;
            let arguments = request["arguments"].clone();
            if !arguments.is_object() || arguments.to_string().len() > 64 * 1024 {
                return Err(invalid("preview arguments must be an object within 64 KiB"));
            }
            Ok(json!({
                "id": format!("{ID_PREFIX}{kind}:{id}"),
                "method": "mcpServer/tool/call",
                "params": {"threadId": thread_id, "server": server, "tool": tool, "arguments": arguments},
            }))
        }
        _ => Err(invalid("invalid preview request")),
    }
}

/// An `app_control_result` frame for a response to one of our requests, or
/// `None` for anything else. Server error text never crosses: it can carry
/// configuration or credentials.
pub fn translate(frame: &Value) -> Option<Value> {
    let rest = frame.get("id")?.as_str()?.strip_prefix(ID_PREFIX)?;
    let (kind, id) = rest.split_once(':')?;
    if frame.get("method").is_some() {
        return None;
    }
    let failed = |error: &str| json!({"type": "app_control_result", "id": id, "ok": false, "error": error});
    if frame.get("error").is_some() {
        return Some(failed("the MCP request failed"));
    }
    let result = frame.get("result")?;
    let value = match kind {
        "inventory" => inventory(result),
        "read" | "write" => read(result),
        _ => None,
    };
    Some(match value {
        Some(value) => json!({"type": "app_control_result", "id": id, "ok": true, "result": value}),
        None => failed("the MCP server returned an unexpected result"),
    })
}

/// `{name, status, tools[{name, readOnly, destructive}]}` from a status list,
/// the shape the router and the sidecar's inventory share.
fn inventory(result: &Value) -> Option<Value> {
    let servers = result.get("data")?.as_array()?;
    // `serverName` already narrowed the list; anything else is refused rather
    // than guessed at, as an ambiguous match could be another account.
    let [server] = servers.as_slice() else {
        return None;
    };
    let name = server.get("name")?.as_str()?;
    let tools = server.get("tools")?.as_object()?;
    let connected = match server.get("runtimeStatus").and_then(Value::as_str) {
        Some(status) => status == "connected",
        None => !tools.is_empty(),
    };
    let status = if connected {
        "connected".to_owned()
    } else {
        server
            .get("runtimeStatus")
            .and_then(Value::as_str)
            .unwrap_or("unavailable")
            .to_owned()
    };
    let mut names: Vec<(&String, &Value)> = tools.iter().collect();
    names.sort_by(|a, b| a.0.cmp(b.0));
    let tools: Vec<Value> = names
        .into_iter()
        .map(|(key, tool)| {
            let annotations = tool.get("annotations").and_then(Value::as_object);
            let hint = |names: &[&str]| -> Value {
                annotations
                    .and_then(|map: &Map<String, Value>| names.iter().find_map(|name| map.get(*name)))
                    .filter(|value| value.is_boolean())
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            json!({
                "name": tool.get("name").and_then(Value::as_str).unwrap_or(key),
                "readOnly": hint(&["readOnlyHint", "readOnly"]),
                "destructive": hint(&["destructiveHint", "destructive"]),
            })
        })
        .collect();
    let value = json!({"name": name, "status": status, "tools": tools});
    (value.to_string().len() <= 256 * 1024).then_some(value)
}

/// A tool result, kept to the MCP result shape.
fn read(result: &Value) -> Option<Value> {
    let object = result.as_object()?;
    object.get("content")?.as_array()?;
    let mut kept = Map::new();
    for key in ["content", "structuredContent", "isError"] {
        if let Some(value) = object.get(key) {
            kept.insert(key.to_owned(), value.clone());
        }
    }
    let value = Value::Object(kept);
    (value.to_string().len() <= 256 * 1024).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_carry_the_exact_server_tool_and_arguments() {
        let read = request_frame("r1", "thread", &json!({"kind":"read","server":"slack","tool":"slack_search_users","arguments":{"query":"Ada"}})).unwrap();
        assert_eq!(read["method"], "mcpServer/tool/call");
        assert_eq!(read["id"], "bridge-app:read:r1");
        assert_eq!(read["params"], json!({"threadId":"thread","server":"slack","tool":"slack_search_users","arguments":{"query":"Ada"}}));
        let inventory = request_frame("i1", "thread", &json!({"kind":"inventory","server":"slack"})).unwrap();
        assert_eq!(inventory["method"], "mcpServerStatus/list");
        assert_eq!(inventory["params"], json!({"serverName":"slack","threadId":"thread","detail":"toolsAndAuthOnly"}));
        let write = request_frame("w1", "thread", &json!({"kind":"write","server":"slack","tool":"slack_send_message","arguments":{"text":"hi"}})).unwrap();
        assert_eq!(write["id"], "bridge-app:write:w1");
        assert_eq!(write["params"], json!({"threadId":"thread","server":"slack","tool":"slack_send_message","arguments":{"text":"hi"}}));
        assert_eq!(translate(&json!({"id":"bridge-app:write:w1","result":{"content":[{"type":"text","text":"sent"}]}})).unwrap()["result"]["content"][0]["text"], "sent");
        for bad in [
            json!({"kind":"delete","server":"slack","tool":"x","arguments":{}}),
            json!({"kind":"read","server":"","tool":"x","arguments":{}}),
            json!({"kind":"read","server":"slack","tool":"x","arguments":[]}),
            json!({"kind":"read","server":"slack","tool":"x","arguments":{"t":"x".repeat(65536)}}),
        ] {
            assert!(request_frame("x", "thread", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn inventory_normalizes_mcp_annotations() {
        let frame = json!({"id":"bridge-app:inventory:i1","result":{"data":[{"name":"slack","authStatus":"oAuth","runtimeStatus":"connected","resources":[],"resourceTemplates":[],"tools":{
            "slack_send_message":{"name":"slack_send_message","inputSchema":{},"annotations":{"destructiveHint":true}},
            "slack_search_users":{"name":"slack_search_users","inputSchema":{},"annotations":{"readOnlyHint":true}},
            "slack_get_channel":{"name":"slack_get_channel","inputSchema":{}}
        }}],"nextCursor":null}});
        let reply = translate(&frame).unwrap();
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["id"], "i1");
        assert_eq!(reply["result"], json!({"name":"slack","status":"connected","tools":[
            {"name":"slack_get_channel","readOnly":null,"destructive":null},
            {"name":"slack_search_users","readOnly":true,"destructive":null},
            {"name":"slack_send_message","readOnly":null,"destructive":true}
        ]}));
        let mut starting = frame.clone();
        starting["result"]["data"][0]["runtimeStatus"] = json!("authenticationRequired");
        assert_eq!(translate(&starting).unwrap()["result"]["status"], "authenticationRequired");
        let mut two = frame.clone();
        let copy = two["result"]["data"][0].clone();
        two["result"]["data"].as_array_mut().unwrap().push(copy);
        assert_eq!(translate(&two).unwrap()["ok"], false, "two matches could be two accounts");
    }

    #[test]
    fn only_bridge_responses_are_consumed_and_errors_never_relay_text() {
        assert!(translate(&json!({"id":7,"result":{}})).is_none());
        assert!(translate(&json!({"id":"other:read:x","result":{}})).is_none());
        assert!(translate(&json!({"id":"bridge-app:read:x","method":"item/started","params":{}})).is_none());
        let failed = translate(&json!({"id":"bridge-app:read:r1","error":{"code":-32603,"message":"token sk-secret expired"}})).unwrap();
        assert_eq!(failed["ok"], false);
        assert!(!failed.to_string().contains("sk-secret"));
        let read = translate(&json!({"id":"bridge-app:read:r1","result":{"content":[{"type":"text","text":"Ada"}],"_meta":{"x":1}}})).unwrap();
        assert_eq!(read["result"], json!({"content":[{"type":"text","text":"Ada"}]}));
        let big = translate(&json!({"id":"bridge-app:read:r1","result":{"content":[{"type":"text","text":"x".repeat(262144)}]}})).unwrap();
        assert_eq!(big["ok"], false);
        assert_eq!(translate(&json!({"id":"bridge-app:read:r1","result":{"nope":1}})).unwrap()["ok"], false);
    }
}
