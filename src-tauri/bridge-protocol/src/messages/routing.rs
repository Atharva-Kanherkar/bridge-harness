//! The routing domain: how much authority the learned router has over harness
//! and model selection, and how to roll a policy back.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How much authority the router has. Mirrors
/// `bridge_core::learning_router::RouterMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouterMode {
    Disabled,
    Shadow,
    Autonomous,
}

/// Per-workspace routing preferences. Mirrors
/// `bridge_core::learning_router::RouterPreferences`, including its refusal of
/// unknown fields — a misspelled preference must not silently do nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterPreferences {
    pub mode: RouterMode,
    /// Minimum expected pass rate in basis points; the server rejects values
    /// above 10000 (100%).
    #[schemars(range(min = 0, max = 10_000))]
    pub minimum_pass_bps: u16,
    #[serde(default)]
    pub pinned_harness: Option<String>,
    #[serde(default)]
    pub pinned_model: Option<String>,
    #[serde(default)]
    pub excluded_harnesses: Vec<String>,
    #[serde(default)]
    pub excluded_models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetRouterPreferencesParams {
    pub workspace_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRouterPreferencesParams {
    pub workspace_id: String,
    pub preferences: RouterPreferences,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RollbackRoutingPolicyParams {
    pub workspace_id: String,
    /// The policy version to make active again.
    pub target_version: i64,
    /// Why the rollback happened; recorded with the policy change.
    pub explanation: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn router_preferences_round_trip_with_camel_case_wire_names() {
        let update = UpdateRouterPreferencesParams {
            workspace_id: "w-1".into(),
            preferences: RouterPreferences {
                mode: RouterMode::Autonomous,
                minimum_pass_bps: 6_500,
                pinned_harness: Some("codex".into()),
                pinned_model: None,
                excluded_harnesses: Vec::new(),
                excluded_models: vec!["haiku".into()],
            },
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(wire["workspaceId"], json!("w-1"));
        assert_eq!(wire["preferences"]["mode"], json!("autonomous"));
        assert_eq!(wire["preferences"]["minimumPassBps"], json!(6_500));
        assert_eq!(wire["preferences"]["pinnedHarness"], json!("codex"));
        assert_eq!(wire["preferences"]["excludedModels"], json!(["haiku"]));
        assert_eq!(round_trip(&update), update);

        let get = GetRouterPreferencesParams { workspace_id: "w-1".into() };
        assert_eq!(serde_json::to_value(&get).unwrap(), json!({"workspaceId": "w-1"}));
        assert_eq!(round_trip(&get), get);
    }

    #[test]
    fn optional_preferences_default_when_absent() {
        let preferences: RouterPreferences =
            serde_json::from_value(json!({"mode": "shadow", "minimumPassBps": 6_500})).unwrap();
        assert_eq!(preferences.pinned_harness, None);
        assert!(preferences.excluded_harnesses.is_empty());
    }

    #[test]
    fn rollback_params_round_trip() {
        let rollback = RollbackRoutingPolicyParams {
            workspace_id: "w".into(),
            target_version: 7,
            explanation: "canary regressed".into(),
        };
        assert_eq!(
            serde_json::to_value(&rollback).unwrap(),
            json!({"workspaceId": "w", "targetVersion": 7, "explanation": "canary regressed"})
        );
        assert_eq!(round_trip(&rollback), rollback);
    }

    #[test]
    fn routing_params_reject_incomplete_and_misspelled_payloads() {
        assert!(serde_json::from_value::<GetRouterPreferencesParams>(json!({})).is_err());
        assert!(serde_json::from_value::<UpdateRouterPreferencesParams>(
            json!({"workspaceId": "w"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<RouterPreferences>(json!({"mode": "shadow"})).is_err(),
            "minimumPassBps is required"
        );
        assert!(
            serde_json::from_value::<RouterPreferences>(
                json!({"mode": "shadow", "minimum_pass_bps": 1})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<RouterPreferences>(json!({
                "mode": "shadow", "minimumPassBps": 1, "pinnedHarnesses": ["codex"],
            }))
            .is_err(),
            "a misspelled preference must be rejected, not ignored"
        );
        assert!(
            serde_json::from_value::<RouterPreferences>(
                json!({"mode": "supervised", "minimumPassBps": 1})
            )
            .is_err(),
            "unknown router modes must be rejected"
        );
        assert!(serde_json::from_value::<RollbackRoutingPolicyParams>(json!({"targetVersion": 1}))
            .is_err());
    }
}
