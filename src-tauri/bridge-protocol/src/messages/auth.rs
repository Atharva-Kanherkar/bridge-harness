//! The provider sign-in domain: starting a vendor's own login flow inside
//! Bridge's terminal infrastructure so the health probe can observe when it
//! completes. There is no field to type a token into anywhere in this
//! domain — the vendor process owns its own credential handoff, and Bridge
//! only launches it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartProviderLoginParams {
    pub provider: String,
}

/// Identifies the terminal pane hosting the vendor's login flow — the same
/// `workspaceId`/`terminalId` pair the existing terminal domain methods
/// (`terminal/write_terminal`, `terminal/resize_terminal`,
/// `terminal/close_terminal`) address, so the UI hosts the flow without a
/// second PTY surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartProviderLoginResult {
    pub workspace_id: String,
    pub terminal_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn start_provider_login_round_trips_with_camel_case_wire_names() {
        let params = StartProviderLoginParams { provider: "claude".into() };
        assert_eq!(serde_json::to_value(&params).unwrap(), json!({"provider": "claude"}));
        assert_eq!(round_trip(&params), params);

        let result = StartProviderLoginResult {
            workspace_id: "provider-login".into(),
            terminal_id: "claude".into(),
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({"workspaceId": "provider-login", "terminalId": "claude"})
        );
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn start_provider_login_params_reject_incomplete_payloads() {
        assert!(serde_json::from_value::<StartProviderLoginParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<StartProviderLoginParams>(json!({"provider_id": "claude"}))
                .is_err(),
            "wire names are camelCase"
        );
    }
}
