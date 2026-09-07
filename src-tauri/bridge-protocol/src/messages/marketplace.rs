//! The marketplace domain: plugins installed into the harnesses' own plugin
//! systems.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whose plugin system a plugin lives in. Mirrors
/// `bridge_core::marketplace::MarketplaceProvider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MarketplaceProvider {
    Codex,
    Claude,
}

/// What to do to a plugin. Mirrors
/// `bridge_core::marketplace::MarketplaceAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MarketplaceAction {
    Install,
    Enable,
    Disable,
    Update,
    Uninstall,
    Authenticate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketplaceActionParams {
    pub provider: MarketplaceProvider,
    pub plugin_id: String,
    /// The marketplace the plugin comes from; omitted uses the provider's
    /// default marketplace.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marketplace: Option<String>,
    pub action: MarketplaceAction,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn marketplace_actions_round_trip() {
        let action = MarketplaceActionParams {
            provider: MarketplaceProvider::Claude,
            plugin_id: "gstack".into(),
            marketplace: None,
            action: MarketplaceAction::Install,
        };
        assert_eq!(
            serde_json::to_value(&action).unwrap(),
            json!({"provider": "claude", "pluginId": "gstack", "action": "install"}),
            "absent options stay off the wire"
        );
        assert_eq!(round_trip(&action), action);

        let scoped = MarketplaceActionParams {
            provider: MarketplaceProvider::Codex,
            plugin_id: "gstack".into(),
            marketplace: Some("community".into()),
            action: MarketplaceAction::Uninstall,
        };
        assert_eq!(
            serde_json::to_value(&scoped).unwrap(),
            json!({
                "provider": "codex",
                "pluginId": "gstack",
                "marketplace": "community",
                "action": "uninstall",
            })
        );
        assert_eq!(round_trip(&scoped), scoped);
    }

    #[test]
    fn marketplace_params_reject_incomplete_and_unknown_payloads() {
        assert!(serde_json::from_value::<MarketplaceActionParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<MarketplaceActionParams>(
                json!({"provider": "claude", "pluginId": "gstack"})
            )
            .is_err(),
            "action is required"
        );
        assert!(
            serde_json::from_value::<MarketplaceActionParams>(
                json!({"provider": "claude", "plugin_id": "gstack", "action": "install"})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<MarketplaceActionParams>(
                json!({"provider": "cursor", "pluginId": "gstack", "action": "install"})
            )
            .is_err(),
            "unknown providers must be rejected"
        );
        assert!(
            serde_json::from_value::<MarketplaceActionParams>(
                json!({"provider": "claude", "pluginId": "gstack", "action": "reinstall"})
            )
            .is_err(),
            "unknown actions must be rejected"
        );
    }
}
