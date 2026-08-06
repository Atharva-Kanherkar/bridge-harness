//! The terminal domain: the PTY attached to a workspace. Terminal bytes flow
//! back as transient notifications, never as durable events.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpenTerminalParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WriteTerminalParams {
    pub workspace_id: String,
    /// Raw bytes to write to the PTY, exactly as typed.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResizeTerminalParams {
    pub workspace_id: String,
    pub rows: u16,
    pub cols: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn terminal_params_round_trip() {
        let open = OpenTerminalParams { workspace_id: "w-1".into() };
        assert_eq!(serde_json::to_value(&open).unwrap(), json!({"workspaceId": "w-1"}));
        assert_eq!(round_trip(&open), open);

        let write = WriteTerminalParams { workspace_id: "w-1".into(), data: "ls -la\n".into() };
        assert_eq!(
            serde_json::to_value(&write).unwrap(),
            json!({"workspaceId": "w-1", "data": "ls -la\n"})
        );
        assert_eq!(round_trip(&write), write);

        let resize = ResizeTerminalParams { workspace_id: "w-1".into(), rows: 48, cols: 160 };
        assert_eq!(
            serde_json::to_value(&resize).unwrap(),
            json!({"workspaceId": "w-1", "rows": 48, "cols": 160})
        );
        assert_eq!(round_trip(&resize), resize);
    }

    #[test]
    fn terminal_params_reject_incomplete_payloads() {
        assert!(serde_json::from_value::<OpenTerminalParams>(json!({})).is_err());
        assert!(serde_json::from_value::<WriteTerminalParams>(json!({"workspaceId": "w"})).is_err());
        assert!(
            serde_json::from_value::<ResizeTerminalParams>(
                json!({"workspaceId": "w", "rows": 48})
            )
            .is_err(),
            "cols is required"
        );
        assert!(
            serde_json::from_value::<ResizeTerminalParams>(
                json!({"workspaceId": "w", "rows": 48, "cols": -1})
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
