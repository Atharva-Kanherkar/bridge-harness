//! A Codex launch with every connector turned off.
//!
//! Codex has no per-tool authority (see `briefing_policy`), so a workflow
//! step on Codex cannot be held to a few granted connector tools. It can be
//! held to none: the step launches with every MCP server and every plugin
//! switched off, and with Codex apps off. The list comes from the config
//! Codex itself reports for the step's folder, so servers from a project's
//! own `.codex/config.toml` are included.
//!
//! Two checks fail the launch closed before the step can work:
//!
//! * the launched process's own config must show every server and plugin
//!   `enabled = false`, and apps off ([`isolation_holds`]);
//! * once the thread starts, no MCP server may report a tool
//!   ([`reachable_tools`]). That catches a source the config does not name.

use serde_json::Value;

/// The connectors a Codex config holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    pub servers: Vec<String>,
    pub plugins: Vec<String>,
}

/// Only these characters reach a `-c` key path: a dot would split it and a
/// quote would end it.
fn key_is_safe(key: &str) -> bool {
    (1..=128).contains(&key.len()) && key.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'@'))
}

fn table<'a>(config: &'a serde_json::Map<String, Value>, name: &str) -> Vec<(&'a String, &'a Value)> {
    config.get(name).and_then(Value::as_object).map(|entries| entries.iter().collect()).unwrap_or_default()
}

/// Every MCP server and plugin in a `config/read` result. A key Bridge cannot
/// write safely refuses the launch: it could not be turned off.
pub fn inventory(result: &Value) -> Result<Inventory, String> {
    let config = result.get("config").and_then(Value::as_object).ok_or("Codex did not report its config")?;
    let mut inventory = Inventory::default();
    for (key, _) in table(config, "mcp_servers") {
        if !crate::tool_selection::codex_key_is_safe(key) {
            return Err(format!("Codex has an MCP server named `{key}`, which Bridge cannot turn off safely"));
        }
        inventory.servers.push(key.clone());
    }
    for (key, _) in table(config, "plugins") {
        if !key_is_safe(key) {
            return Err(format!("Codex has a plugin named `{key}`, which Bridge cannot turn off safely"));
        }
        inventory.plugins.push(key.clone());
    }
    inventory.servers.sort();
    inventory.plugins.sort();
    Ok(inventory)
}

/// Does the launched process's own config show everything off? Any server
/// or plugin it holds must say `enabled = false`, whether or not the first
/// read listed it, and apps must be off.
pub fn isolation_holds(result: &Value) -> Result<(), String> {
    let config = result.get("config").and_then(Value::as_object).ok_or("Codex did not report its config")?;
    for (kind, name) in [("MCP server", "mcp_servers"), ("plugin", "plugins")] {
        for (key, entry) in table(config, name) {
            if entry.get("enabled").and_then(Value::as_bool) != Some(false) {
                return Err(format!("Codex's {kind} `{key}` is still on"));
            }
        }
    }
    if config.get("features").and_then(|features| features.get("apps")).and_then(Value::as_bool) != Some(false) {
        return Err("Codex apps are still on".into());
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
    use serde_json::json;

    fn config(value: Value) -> Value {
        json!({ "config": value, "layers": [] })
    }

    #[test]
    fn every_server_and_plugin_is_listed_and_turned_off() {
        let read = config(json!({
            "mcp_servers": { "railway": { "enabled": true }, "computer-use": { "enabled": false } },
            "plugins": { "slack@openai-curated": { "enabled": true } },
            "features": { "apps": true }
        }));
        let found = inventory(&read).unwrap();
        assert_eq!(found.servers, ["computer-use", "railway"]);
        assert_eq!(found.plugins, ["slack@openai-curated"]);
        let launch = crate::tool_selection::ToolLaunch {
            servers_off: found.servers,
            plugins_off: found.plugins,
            ..crate::tool_selection::ToolLaunch::connectors_off("codex")
        };
        assert_eq!(
            crate::tool_selection::codex_overrides(&launch),
            [
                "mcp_servers.computer-use.enabled=false",
                "mcp_servers.railway.enabled=false",
                "plugins.slack@openai-curated.enabled=false",
                "features.apps=false",
            ]
        );
        assert_eq!(
            crate::tool_selection::codex_overrides(&crate::tool_selection::ToolLaunch::connectors_off("codex")),
            ["features.apps=false"],
            "apps are always turned off"
        );
    }

    #[test]
    fn a_key_bridge_cannot_write_refuses_the_launch() {
        for (table, key) in [("mcp_servers", "my.server"), ("mcp_servers", "a\"b"), ("mcp_servers", "a@b"), ("plugins", "x y@z"), ("plugins", "p.q"), ("mcp_servers", "")] {
            let read = config(json!({ table: { key: { "enabled": true } } }));
            let error = inventory(&read).unwrap_err();
            assert!(error.contains("cannot turn off safely"), "{table} {key}: {error}");
        }
        assert!(inventory(&json!({ "error": "nope" })).is_err(), "no config is no inventory");
    }

    #[test]
    fn isolation_holds_only_when_everything_says_off() {
        let off = json!({ "mcp_servers": { "a": { "enabled": false } }, "plugins": { "p@m": { "enabled": false } }, "features": { "apps": false } });
        assert_eq!(isolation_holds(&config(off.clone())), Ok(()));
        let cases = [
            ("/mcp_servers/a/enabled", json!(true), "MCP server `a`"),
            ("/plugins/p@m/enabled", json!(true), "plugin `p@m`"),
            ("/features/apps", json!(true), "apps"),
        ];
        for (pointer, value, needle) in cases {
            let mut changed = off.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(isolation_holds(&config(changed)).unwrap_err().contains(needle), "{pointer}");
        }
        // A server the first read never saw, and one with no `enabled` at all.
        let mut extra = off.clone();
        extra["mcp_servers"]["late"] = json!({ "command": "x" });
        assert!(isolation_holds(&config(extra)).unwrap_err().contains("`late`"));
        let mut no_apps = off;
        no_apps["features"] = json!({});
        assert!(isolation_holds(&config(no_apps)).is_err(), "apps unset is on by default");
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
