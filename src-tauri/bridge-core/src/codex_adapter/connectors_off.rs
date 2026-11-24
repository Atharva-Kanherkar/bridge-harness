//! A Codex launch with every connector turned off.
//!
//! Codex has no per-tool authority (see `briefing_policy`), so a workflow
//! step on Codex cannot be held to a few granted connector tools. It can be
//! held to none. The step launches with:
//!
//! * every MCP server in the config Codex reports for the step's folder
//!   switched off, so a project's own `.codex/config.toml` servers count too;
//! * Codex apps off;
//! * plugins, remote plugins and skill-installed MCP servers off
//!   ([`CODEX_CONNECTOR_FEATURES`]). Remote plugins are account-level, are
//!   named in no config entry, and connect seconds after the thread starts,
//!   so only their feature switch reaches them.
//!
//! Two checks fail the launch closed before the step can work:
//!
//! * the launched process's own config must show every server off and each
//!   of those features off ([`isolation_holds`]);
//! * once the thread starts, no MCP server may report a tool
//!   ([`reachable_tools`]).

use serde_json::Value;

use crate::tool_selection::{codex_key_is_safe, CODEX_CONNECTOR_FEATURES};

/// The MCP servers a Codex config holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub servers: Vec<String>,
}

fn table<'a>(config: &'a serde_json::Map<String, Value>, name: &str) -> Vec<(&'a String, &'a Value)> {
    config.get(name).and_then(Value::as_object).map(|entries| entries.iter().collect()).unwrap_or_default()
}

/// Every MCP server in a `config/read` result. A key Bridge cannot write
/// safely refuses the launch: it could not be turned off.
pub fn inventory(result: &Value) -> Result<Inventory, String> {
    let config = result.get("config").and_then(Value::as_object).ok_or("Codex did not report its config")?;
    let mut inventory = Inventory::default();
    for (key, _) in table(config, "mcp_servers") {
        if !codex_key_is_safe(key) {
            return Err(format!("Codex has an MCP server named `{key}`, which Bridge cannot turn off safely"));
        }
        inventory.servers.push(key.clone());
    }
    inventory.servers.sort();
    Ok(inventory)
}

/// Does the launched process's own config show everything off? Any server
/// it holds must say `enabled = false`, whether or not the first read listed
/// it, and apps and each connector feature must be off.
pub fn isolation_holds(result: &Value) -> Result<(), String> {
    let config = result.get("config").and_then(Value::as_object).ok_or("Codex did not report its config")?;
    for (key, entry) in table(config, "mcp_servers") {
        if entry.get("enabled").and_then(Value::as_bool) != Some(false) {
            return Err(format!("Codex's MCP server `{key}` is still on"));
        }
    }
    let features = config.get("features");
    for feature in std::iter::once("apps").chain(CODEX_CONNECTOR_FEATURES) {
        if features.and_then(|features| features.get(feature)).and_then(Value::as_bool) != Some(false) {
            return Err(format!("Codex's `{feature}` feature is still on"));
        }
    }
    Ok(())
}

/// The servers of an `mcpServerStatus/list` result that still offer a tool.
/// A reply Bridge cannot read, or one with more pages, counts as unsafe.
pub fn reachable_tools(result: &Value) -> Result<Vec<String>, String> {
    let servers = result.get("data").and_then(Value::as_array).ok_or("Codex did not list its MCP servers")?;
    if result.get("nextCursor").is_some_and(|cursor| !cursor.is_null()) {
        return Err("Codex listed more MCP servers than Bridge read".into());
    }
    let mut reachable = Vec::new();
    for server in servers {
        let name = server.get("name").and_then(Value::as_str).unwrap_or("an unnamed server");
        let tools = match server.get("tools") {
            None | Some(Value::Null) => 0,
            Some(Value::Object(tools)) => tools.len(),
            Some(Value::Array(tools)) => tools.len(),
            Some(_) => return Err(format!("Codex reported the tools of `{name}` in a form Bridge cannot read")),
        };
        if tools > 0 {
            reachable.push(name.to_owned());
        }
    }
    Ok(reachable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_selection::{codex_overrides, ToolLaunch};
    use serde_json::json;

    fn config(value: Value) -> Value {
        json!({ "config": value, "layers": [] })
    }

    fn all_off() -> Value {
        json!({
            "mcp_servers": { "a": { "enabled": false } },
            "plugins": { "slack@openai-curated": { "enabled": true } },
            "features": { "apps": false, "plugins": false, "remote_plugin": false, "skill_mcp_dependency_install": false }
        })
    }

    #[test]
    fn every_server_is_listed_and_turned_off_with_the_connector_features() {
        let read = config(json!({
            "mcp_servers": { "railway": { "enabled": true }, "computer-use": { "enabled": false } },
            "plugins": { "slack@openai-curated": { "enabled": true } },
            "features": { "apps": true }
        }));
        let found = inventory(&read).unwrap();
        assert_eq!(found.servers, ["computer-use", "railway"]);
        let launch = ToolLaunch { servers_off: found.servers, ..ToolLaunch::connectors_off("codex") };
        assert_eq!(
            codex_overrides(&launch),
            [
                "mcp_servers.computer-use.enabled=false",
                "mcp_servers.railway.enabled=false",
                "features.apps=false",
                "features.plugins=false",
                "features.remote_plugin=false",
                "features.skill_mcp_dependency_install=false",
            ]
        );
        assert_eq!(
            codex_overrides(&ToolLaunch::connectors_off("codex")),
            ["features.apps=false", "features.plugins=false", "features.remote_plugin=false", "features.skill_mcp_dependency_install=false"],
            "apps and the connector features are always turned off"
        );
        assert!(codex_overrides(&ToolLaunch::connectors_off("claude")).is_empty(), "Codex only");
    }

    #[test]
    fn a_server_bridge_cannot_write_refuses_the_launch() {
        for key in ["my.server", "a\"b", "a@b", ""] {
            let read = config(json!({ "mcp_servers": { key: { "enabled": true } } }));
            let error = inventory(&read).unwrap_err();
            assert!(error.contains("cannot turn off safely"), "{key}: {error}");
        }
        assert!(inventory(&json!({ "error": "nope" })).is_err(), "no config is no inventory");
    }

    #[test]
    fn isolation_holds_only_when_every_server_and_feature_says_off() {
        assert_eq!(isolation_holds(&config(all_off())), Ok(()), "a plugin entry does not matter once plugins are off");
        let cases = [
            ("/mcp_servers/a/enabled", json!(true), "MCP server `a`"),
            ("/features/apps", json!(true), "`apps`"),
            ("/features/plugins", json!(true), "`plugins`"),
            ("/features/remote_plugin", json!(true), "`remote_plugin`"),
            ("/features/skill_mcp_dependency_install", json!(true), "`skill_mcp_dependency_install`"),
        ];
        for (pointer, value, needle) in cases {
            let mut changed = all_off();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(isolation_holds(&config(changed)).unwrap_err().contains(needle), "{pointer}");
        }
        // A server the first read never saw, and a feature left unset (on by default).
        let mut extra = all_off();
        extra["mcp_servers"]["late"] = json!({ "command": "x" });
        assert!(isolation_holds(&config(extra)).unwrap_err().contains("`late`"));
        for feature in ["apps", "plugins", "remote_plugin", "skill_mcp_dependency_install"] {
            let mut unset = all_off();
            unset["features"].as_object_mut().unwrap().remove(feature);
            assert!(isolation_holds(&config(unset)).is_err(), "{feature} unset");
        }
    }

    #[test]
    fn a_server_that_still_offers_a_tool_is_reachable() {
        let status = json!({ "data": [
            { "name": "railway", "tools": {} },
            { "name": "codex_apps", "tools": { "a": {}, "b": {} } },
            { "name": "quiet", "tools": null },
            { "name": "listed", "tools": [{ "name": "t" }] }
        ], "nextCursor": null });
        assert_eq!(reachable_tools(&status).unwrap(), ["codex_apps", "listed"]);
        assert!(reachable_tools(&json!({ "data": [], "nextCursor": "page2" })).is_err());
        assert!(reachable_tools(&json!({ "nope": true })).is_err());
        assert!(reachable_tools(&json!({ "data": [{ "name": "odd", "tools": 3 }] })).is_err());
        assert_eq!(reachable_tools(&json!({ "data": [] })).unwrap(), Vec::<String>::new());
    }
}
