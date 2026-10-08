//! `bridge.visualize`: Bridge's own MCP Apps server.
//!
//! Chat agents get one tool, `visualize`, that takes a `VisualSpec` (data,
//! never code) and draws it inline in the transcript through the MCP Apps
//! view `ui://bridge/visual`. This module is the server half: the form
//! catalog, the validator, the form-fit lint, the repeat guard, the tool
//! definition the model reads, the stdio JSON-RPC loop, and the rules for
//! which sessions get it. The view and the host frame live in
//! `src/mcp-apps/`.
//!
//! The server runs as a helper mode of the bundled `bridged` binary, the
//! same way the keychain helper does, so a release build reaches it without
//! shipping another executable.

pub mod catalog;
pub mod guard;
pub mod lint;
pub mod server;
pub mod spec;
pub mod tool;

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::delegation::WriteMode;
use crate::worker_sandbox::ReadOnlySandbox;
use crate::briefing_policy::BriefingRuntimePolicy;

/// The `bridged` flag that turns it into this stdio server.
pub const HELPER_FLAG: &str = "--bridge-mcp-visualize";
/// Overrides where the helper binary is found (tests, unusual installs).
pub const BIN_ENV: &str = "BRIDGE_VISUALIZE_MCP_BIN";

/// Serialise with object keys sorted, whatever map type serde_json was built
/// with, so equal data always hashes and compares equal.
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let fields: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{}:{}", Value::String(key.clone()), canonical_json(&map[key])))
                .collect();
            format!("{{{}}}", fields.join(","))
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// Only an interactive chat draws visuals. Workers report to an orchestrator,
/// not a reader; read-only workers and briefings run under strict scopes that
/// a new tool must not widen.
pub fn attaches_to(
    write_mode: Option<WriteMode>,
    read_only_sandbox: Option<&ReadOnlySandbox>,
    briefing: Option<&BriefingRuntimePolicy>,
) -> bool {
    write_mode.is_none() && read_only_sandbox.is_none() && briefing.is_none()
}

/// The `bridged` binary that serves the tool: an explicit override, this
/// process when it is `bridged` (daemon host), or the `bridged` bundled next
/// to it (embedded host). `None` leaves the tool off rather than failing the
/// session.
pub fn helper_binary() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os(BIN_ENV) {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    let current = std::env::current_exe().ok()?;
    if current.file_stem().and_then(|stem| stem.to_str()) == Some("bridged") {
        return Some(current);
    }
    let sibling = current.with_file_name("bridged");
    sibling.is_file().then_some(sibling)
}

/// How a harness launches the server, or `None` when it cannot be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerLaunch {
    pub command: PathBuf,
    pub args: Vec<String>,
}

impl ServerLaunch {
    pub fn resolve() -> Option<Self> {
        helper_binary().map(|command| Self {
            command,
            args: vec![HELPER_FLAG.to_owned()],
        })
    }

    /// Claude Agent SDK `mcpServers` entry.
    pub fn claude_entry(&self) -> Value {
        json!({
            "type": "stdio",
            "command": self.command.to_string_lossy(),
            "args": self.args,
        })
    }

    /// Codex `-c` overrides. Values are TOML: the path is a basic string, the
    /// args an array of them. Scoped to `mcp_servers.bridge` so the user's
    /// own servers are untouched.
    pub fn codex_overrides(&self) -> Vec<String> {
        let toml_string = |text: &str| {
            let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        };
        let args: Vec<String> = self.args.iter().map(|arg| toml_string(arg)).collect();
        let name = tool::SERVER_NAME;
        vec![
            format!("mcp_servers.{name}.command={}", toml_string(&self.command.to_string_lossy())),
            format!("mcp_servers.{name}.args=[{}]", args.join(",")),
        ]
    }

    /// OpenCode `mcp` entry, merged into `OPENCODE_CONFIG_CONTENT`.
    pub fn opencode_entry(&self) -> Value {
        let mut command = vec![self.command.to_string_lossy().into_owned()];
        command.extend(self.args.iter().cloned());
        json!({"type": "local", "command": command, "enabled": true})
    }
}

/// Merge the `bridge` server into an existing `OPENCODE_CONFIG_CONTENT`
/// value, keeping whatever the user put there.
pub fn merge_opencode_config(existing: Option<&str>, launch: &ServerLaunch) -> String {
    let mut config = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if !config["mcp"].is_object() {
        config["mcp"] = json!({});
    }
    config["mcp"][tool::SERVER_NAME] = launch.opencode_entry();
    config.to_string()
}

/// Entry point for `bridged --bridge-mcp-visualize`.
pub fn serve_stdio() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server::VisualServer::default().serve(stdin.lock(), stdout.lock())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> ServerLaunch {
        ServerLaunch {
            command: PathBuf::from("/Applications/Bridge \"Beta\".app/Contents/MacOS/bridged"),
            args: vec![HELPER_FLAG.to_owned()],
        }
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_depth() {
        let a = json!({"b": 1, "a": {"d": [1, {"z": 0, "y": 1}], "c": null}});
        assert_eq!(canonical_json(&a), r#"{"a":{"c":null,"d":[1,{"y":1,"z":0}]},"b":1}"#);
    }

    #[test]
    fn only_interactive_chats_attach() {
        assert!(attaches_to(None, None, None));
        assert!(!attaches_to(Some(WriteMode::Isolated), None, None));
        assert!(!attaches_to(Some(WriteMode::ReadOnly), None, None));
    }

    #[test]
    fn codex_overrides_are_toml_scoped_to_the_bridge_entry() {
        assert_eq!(
            launch().codex_overrides(),
            vec![
                r#"mcp_servers.bridge.command="/Applications/Bridge \"Beta\".app/Contents/MacOS/bridged""#.to_owned(),
                r#"mcp_servers.bridge.args=["--bridge-mcp-visualize"]"#.to_owned(),
            ]
        );
    }

    #[test]
    fn opencode_config_keeps_the_users_content() {
        let merged: Value = serde_json::from_str(&merge_opencode_config(
            Some(r#"{"model":"x","mcp":{"mine":{"type":"remote","url":"https://a"}}}"#),
            &launch(),
        ))
        .unwrap();
        assert_eq!(merged["model"], "x");
        assert_eq!(merged["mcp"]["mine"]["url"], "https://a");
        assert_eq!(merged["mcp"]["bridge"]["type"], "local");
        assert_eq!(merged["mcp"]["bridge"]["command"][1], HELPER_FLAG);
        let fresh: Value = serde_json::from_str(&merge_opencode_config(Some("not json"), &launch())).unwrap();
        assert_eq!(fresh["mcp"]["bridge"]["enabled"], true);
    }

    #[test]
    fn an_explicit_binary_that_does_not_exist_leaves_the_tool_off() {
        std::env::set_var(BIN_ENV, "/nowhere/bridged");
        assert_eq!(helper_binary(), None);
        std::env::remove_var(BIN_ENV);
    }
}
