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

/// The JSON-RPC request for one app-control request, or a refusal. `cwd` is
/// the folder of the chat, for the reads that take no thread id.
pub fn request_frame(id: &str, thread_id: &str, cwd: &str, request: &Value) -> Result<Value, BridgeError> {
    // Every server's name and status, for correcting a misspelled name.
    if request["kind"] == "servers" {
        return Ok(json!({
            "id": format!("{ID_PREFIX}servers:{id}"),
            "method": "mcpServerStatus/list",
            "params": {"threadId": thread_id, "detail": "toolsAndAuthOnly"},
        }));
    }
    // The tool picker of the chat reads its skills and its config. Both are
    // reads: Bridge never sends a skills or config write.
    let cwd = Some(cwd).filter(|cwd| !cwd.is_empty());
    if request["kind"] == "skills" {
        return Ok(json!({
            "id": format!("{ID_PREFIX}skills:{id}"),
            "method": "skills/list",
            "params": {"cwds": cwd.into_iter().collect::<Vec<_>>()},
        }));
    }
    if request["kind"] == "config" {
        return Ok(json!({
            "id": format!("{ID_PREFIX}config:{id}"),
            "method": "config/read",
            "params": {"cwd": cwd, "includeLayers": true},
        }));
    }
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
        "servers" => servers(result),
        "skills" => skills(result),
        "config" => config(result),
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
fn servers(result: &Value) -> Option<Value> {
    let listed: Vec<Value> = result
        .get("data")?
        .as_array()?
        .iter()
        .take(200)
        .filter_map(|server| {
            let name = server.get("name")?.as_str()?;
            let connected = match server.get("runtimeStatus").and_then(Value::as_str) {
                Some(status) => status == "connected",
                None => server.get("tools").and_then(Value::as_object).is_some_and(|tools| !tools.is_empty()),
            };
            // A bundled server (Codex's connectors) names its tools
            // `<connector>.<tool>`: its connectors let "notion" find it.
            let mut connectors: Vec<&str> = server
                .get("tools")
                .and_then(Value::as_object)
                .map(|tools| tools.keys().filter_map(|key| key.split_once('.').map(|(prefix, _)| prefix)).collect())
                .unwrap_or_default();
            connectors.sort_unstable();
            connectors.dedup();
            connectors.truncate(50);
            // Not yet connected keeps Codex's own word (`starting`), so the
            // router can wait for a fresh runtime's slower servers.
            let status = if connected { "connected" } else { server.get("runtimeStatus").and_then(Value::as_str).unwrap_or("unavailable") };
            // The figure leaves the reply in place of the tool schemas.
            let tokens = crate::tool_selection::codex_tools_tokens(server);
            Some(json!({"name": name, "status": status, "connectors": connectors, "tokens": tokens}))
        })
        .collect();
    Some(json!({"servers": listed}))
}

/// `{skills:[{name, enabled, tokens}]}` from a `skills/list` reply. A name
/// that a stored choice cannot hold is dropped, and no description crosses.
fn skills(result: &Value) -> Option<Value> {
    let mut seen = std::collections::BTreeSet::new();
    let listed: Vec<Value> = result
        .get("data")?
        .as_array()?
        .iter()
        .filter_map(|entry| entry.get("skills").and_then(Value::as_array))
        .flatten()
        .filter_map(|skill| {
            let name = skill.get("name")?.as_str()?;
            if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) || !seen.insert(name) {
                return None;
            }
            let description = skill.get("description").and_then(Value::as_str).unwrap_or_default();
            let enabled = skill.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let tokens = crate::tool_selection::codex_skill_tokens(name, description);
            Some(json!({"name": name, "enabled": enabled, "tokens": tokens}))
        })
        .take(512)
        .collect();
    let value = json!({"skills": listed});
    (value.to_string().len() <= 256 * 1024).then_some(value)
}

/// True when the `result` of a `config/read` reply has a `skills` key of the
/// user: a layer other than the `-c` values of the launch holds the key, so
/// the own `skills.config` value of Bridge does not count. A layer config
/// that is not an object counts as a key. Without layers the effective config
/// decides, which can only count too much. `None` for a result with no
/// `config` object.
pub fn user_skills_key(result: &Value) -> Option<bool> {
    let effective = result.get("config")?.as_object()?;
    Some(match result.get("layers").and_then(Value::as_array) {
        Some(layers) => layers.iter().any(|layer| {
            layer.pointer("/name/type").and_then(Value::as_str) != Some("sessionFlags")
                && layer.get("config").and_then(Value::as_object).is_none_or(|config| config.contains_key("skills"))
        }),
        None => effective.contains_key("skills"),
    })
}

/// `{userSkillsKey, plugins}` from a `config/read` reply. Only these facts
/// cross: the config can hold credentials. [`user_skills_key`] gives the
/// first fact.
fn config(result: &Value) -> Option<Value> {
    let user_skills_key = user_skills_key(result)?;
    let effective = result.get("config")?.as_object()?;
    let mut plugins: Vec<&str> = effective
        .get("plugins")
        .and_then(Value::as_object)
        .map(|plugins| plugins.keys().map(String::as_str).filter(|name| name.len() <= 128).collect())
        .unwrap_or_default();
    plugins.sort_unstable();
    plugins.truncate(200);
    Some(json!({"userSkillsKey": user_skills_key, "plugins": plugins}))
}

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
        let read = request_frame("r1", "thread", "/w", &json!({"kind":"read","server":"slack","tool":"slack_search_users","arguments":{"query":"Ada"}})).unwrap();
        assert_eq!(read["method"], "mcpServer/tool/call");
        assert_eq!(read["id"], "bridge-app:read:r1");
        assert_eq!(read["params"], json!({"threadId":"thread","server":"slack","tool":"slack_search_users","arguments":{"query":"Ada"}}));
        let inventory = request_frame("i1", "thread", "/w", &json!({"kind":"inventory","server":"slack"})).unwrap();
        assert_eq!(inventory["method"], "mcpServerStatus/list");
        assert_eq!(inventory["params"], json!({"serverName":"slack","threadId":"thread","detail":"toolsAndAuthOnly"}));
        let write = request_frame("w1", "thread", "/w", &json!({"kind":"write","server":"slack","tool":"slack_send_message","arguments":{"text":"hi"}})).unwrap();
        assert_eq!(write["id"], "bridge-app:write:w1");
        assert_eq!(write["params"], json!({"threadId":"thread","server":"slack","tool":"slack_send_message","arguments":{"text":"hi"}}));
        assert_eq!(translate(&json!({"id":"bridge-app:write:w1","result":{"content":[{"type":"text","text":"sent"}]}})).unwrap()["result"]["content"][0]["text"], "sent");
        for bad in [
            json!({"kind":"delete","server":"slack","tool":"x","arguments":{}}),
            json!({"kind":"read","server":"","tool":"x","arguments":{}}),
            json!({"kind":"read","server":"slack","tool":"x","arguments":[]}),
            json!({"kind":"read","server":"slack","tool":"x","arguments":{"t":"x".repeat(65536)}}),
        ] {
            assert!(request_frame("x", "thread", "/w", &bad).is_err(), "{bad}");
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

    #[test]
    fn the_status_reply_keeps_a_token_estimate_for_each_server() {
        let github = json!({"search": {"name": "search", "description": "Search the code of a repository", "inputSchema": {"type": "object"}}});
        let apps = json!({"gmail.send": {"name": "gmail.send"}, "drive.list": {"name": "drive.list"}});
        let frame = json!({"id": "bridge-app:servers:s1", "result": {"data": [
            {"name": "github", "runtimeStatus": "connected", "tools": github},
            {"name": "sentry", "runtimeStatus": "connected", "tools": {}},
            {"name": "codex_apps", "runtimeStatus": "connected", "tools": apps},
            {"name": "starting", "runtimeStatus": "starting"}
        ]}});
        let reply = translate(&frame).unwrap();
        assert_eq!((reply["ok"].clone(), reply["id"].clone()), (json!(true), json!("s1")));
        let figure = |tools: &Value| json!(tools.to_string().len() / 4);
        assert_eq!(reply["result"], json!({"servers": [
            {"name": "github", "status": "connected", "connectors": [], "tokens": figure(&github)},
            {"name": "sentry", "status": "connected", "connectors": [], "tokens": null},
            {"name": "codex_apps", "status": "connected", "connectors": ["drive", "gmail"], "tokens": figure(&apps)},
            {"name": "starting", "status": "starting", "connectors": [], "tokens": null}
        ]}));
        assert!(reply["result"]["servers"][0]["tokens"].as_u64().is_some_and(|tokens| tokens > 0));
        // The figure replaces the schemas: no tool text crosses.
        assert!(!reply.to_string().contains("Search the code"));
    }

    #[test]
    fn skills_and_config_requests_are_reads_on_the_chat_thread() {
        let skills = request_frame("k1", "thread", "/work/repo", &json!({"kind": "skills"})).unwrap();
        assert_eq!(skills, json!({"id": "bridge-app:skills:k1", "method": "skills/list", "params": {"cwds": ["/work/repo"]}}));
        let config = request_frame("c1", "thread", "/work/repo", &json!({"kind": "config"})).unwrap();
        assert_eq!(config, json!({"id": "bridge-app:config:c1", "method": "config/read", "params": {"cwd": "/work/repo", "includeLayers": true}}));
        // Extra fields cannot turn either request into a write.
        for kind in ["skills", "config"] {
            let frame = request_frame("x", "thread", "/w", &json!({"kind": kind, "method": "skills/config/write", "edits": [{"name": "a"}]})).unwrap();
            assert!(matches!(frame["method"].as_str(), Some("skills/list" | "config/read")), "{frame}");
            assert!(!frame.to_string().contains("write"), "{frame}");
        }

        // Each reply is consumed as an app-control result, so it never reaches the transcript.
        let listed = translate(&json!({"id": "bridge-app:skills:k1", "result": {"data": [
            {"cwd": "/work/repo", "errors": [], "skills": [
                {"name": "review", "description": "Review a diff", "enabled": true, "path": "/h/.codex/skills/review/SKILL.md", "scope": "user"},
                {"name": "deploy", "description": "", "enabled": false, "path": "/p", "scope": "repo"},
                {"name": "review", "description": "a copy", "enabled": true},
                {"name": "bad\nname", "description": ""},
                {"name": "", "description": ""}
            ]}
        ]}})).unwrap();
        assert_eq!((listed["type"].clone(), listed["id"].clone(), listed["ok"].clone()), (json!("app_control_result"), json!("k1"), json!(true)));
        assert_eq!(listed["result"], json!({"skills": [
            {"name": "review", "enabled": true, "tokens": ("review".len() + "Review a diff".len()) / 4},
            {"name": "deploy", "enabled": false, "tokens": 1}
        ]}));

        let layer = |kind: &str, config: Value| json!({"name": {"type": kind}, "version": "v1", "config": config});
        let read = |layers: Option<Value>| {
            let mut result = json!({"config": {"model": "gpt", "skills": {"config": []}, "mcp_servers": {"x": {"env": {"TOKEN": "sk-secret"}}}, "plugins": {"github@openai-curated": {}, "visualize@openai-bundled": {}}}, "origins": {}});
            if let Some(layers) = layers {
                result["layers"] = layers;
            }
            translate(&json!({"id": "bridge-app:config:c1", "result": result})).unwrap()
        };
        // The own `-c` skills value of Bridge is in the session flags layer: it is not a key of the user.
        let ours = read(Some(json!([layer("sessionFlags", json!({"skills": {"config": []}})), layer("user", json!({"model": "gpt"}))])));
        assert_eq!(ours["result"], json!({"userSkillsKey": false, "plugins": ["github@openai-curated", "visualize@openai-bundled"]}));
        assert!(!ours.to_string().contains("sk-secret"));
        let theirs = read(Some(json!([layer("sessionFlags", json!({})), layer("project", json!({"skills": {}}))])));
        assert_eq!(theirs["result"]["userSkillsKey"], true);
        let unreadable = read(Some(json!([{"name": {"type": "user"}, "version": "v1"}])));
        assert_eq!(unreadable["result"]["userSkillsKey"], true, "a layer without a config counts as a key");
        assert_eq!(read(None)["result"]["userSkillsKey"], true, "without layers the effective config decides");
        assert_eq!(translate(&json!({"id": "bridge-app:config:c1", "result": {"origins": {}}})).unwrap()["ok"], false);
    }
}
