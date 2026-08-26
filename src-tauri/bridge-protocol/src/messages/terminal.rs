//! The terminal domain: the PTYs attached to a workspace. A workspace holds
//! several shells — a dev server, a watcher, a prompt — each addressed by a
//! client-chosen terminal id. Terminal bytes flow back as transient
//! notifications, never as durable events.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenTerminalParams {
    pub workspace_id: String,
    /// Client-chosen shell identity within the workspace. Opening an id that
    /// is already running is a no-op, which is what makes reattach safe.
    pub terminal_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteTerminalParams {
    pub workspace_id: String,
    pub terminal_id: String,
    /// Raw bytes to write to the PTY, exactly as typed.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResizeTerminalParams {
    pub workspace_id: String,
    pub terminal_id: String,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloseTerminalParams {
    pub workspace_id: String,
    pub terminal_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListTerminalsParams {
    pub workspace_id: String,
}

/// The shells still running for a workspace, so a reopened window reattaches
/// instead of respawning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListTerminalsResult {
    pub terminal_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn terminal_params_round_trip() {
        let open = OpenTerminalParams { workspace_id: "w-1".into(), terminal_id: "t1".into() };
        assert_eq!(
            serde_json::to_value(&open).unwrap(),
            json!({"workspaceId": "w-1", "terminalId": "t1"})
        );
        assert_eq!(round_trip(&open), open);

        let write = WriteTerminalParams { workspace_id: "w-1".into(), terminal_id: "t1".into(), data: "ls -la\n".into() };
        assert_eq!(
            serde_json::to_value(&write).unwrap(),
            json!({"workspaceId": "w-1", "terminalId": "t1", "data": "ls -la\n"})
        );
        assert_eq!(round_trip(&write), write);

        let resize = ResizeTerminalParams { workspace_id: "w-1".into(), terminal_id: "t1".into(), rows: 48, cols: 160 };
        assert_eq!(
            serde_json::to_value(&resize).unwrap(),
            json!({"workspaceId": "w-1", "terminalId": "t1", "rows": 48, "cols": 160})
        );
        assert_eq!(round_trip(&resize), resize);

        let close = CloseTerminalParams { workspace_id: "w-1".into(), terminal_id: "t1".into() };
        assert_eq!(
            serde_json::to_value(&close).unwrap(),
            json!({"workspaceId": "w-1", "terminalId": "t1"})
        );
        assert_eq!(round_trip(&close), close);

        let list = ListTerminalsParams { workspace_id: "w-1".into() };
        assert_eq!(serde_json::to_value(&list).unwrap(), json!({"workspaceId": "w-1"}));
        assert_eq!(round_trip(&list), list);

        let listed = ListTerminalsResult { terminal_ids: vec!["t1".into(), "t2".into()] };
        assert_eq!(
            serde_json::to_value(&listed).unwrap(),
            json!({"terminalIds": ["t1", "t2"]})
        );
        assert_eq!(round_trip(&listed), listed);
    }

    #[test]
    fn terminal_params_reject_incomplete_payloads() {
        assert!(serde_json::from_value::<OpenTerminalParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<OpenTerminalParams>(json!({"workspaceId": "w"})).is_err(),
            "a shell needs an identity"
        );
        assert!(serde_json::from_value::<WriteTerminalParams>(json!({"workspaceId": "w"})).is_err());
        assert!(
            serde_json::from_value::<WriteTerminalParams>(
                json!({"workspaceId": "w", "data": "x"})
            )
            .is_err(),
            "writes address one shell"
        );
        assert!(
            serde_json::from_value::<CloseTerminalParams>(json!({"workspaceId": "w"})).is_err(),
            "close addresses one shell"
        );
        assert!(
            serde_json::from_value::<ResizeTerminalParams>(
                json!({"workspaceId": "w", "terminalId": "t", "rows": 48})
            )
            .is_err(),
            "cols is required"
        );
        assert!(
            serde_json::from_value::<ResizeTerminalParams>(
                json!({"workspaceId": "w", "terminalId": "t", "rows": 48, "cols": -1})
            )
            .is_err(),
            "terminal dimensions are unsigned"
        );
        assert!(
            serde_json::from_value::<WriteTerminalParams>(
                json!({"workspace_id": "w", "data": "x"})
            )
            .is_err(),
            "wire names are camelCase"
        );
    }
}
