//! The browser-bridge domain: the supervised tab lease, the actions an agent
//! may take in it, and the routing decision between local and remote browsers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the leased tab may be used for. The command rejects anything outside
/// this set, so the contract names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BrowserPermission {
    ReadOnly,
    Interact,
}

/// One requested browser action. Mirrors
/// `bridge_core::browser_bridge::BrowserActionRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserActionRequest {
    pub kind: String,
    pub element_id: Option<String>,
    pub text: Option<String>,
    pub url: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub tab_id: Option<i64>,
    /// Set when the action touches a sensitive field, so the supervisor can
    /// require an approval before it runs.
    pub sensitive_kind: Option<String>,
    pub expected_domain: Option<String>,
    pub actor: Option<String>,
}

/// The signals the browser router decides on. Mirrors
/// `bridge_core::browser_bridge::BrowserRouteRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRouteRequest {
    pub structured_api_available: bool,
    pub needs_user_auth: bool,
    pub needs_isolation: bool,
    pub needs_parallelism: bool,
    pub needs_geo_or_proxy: bool,
    pub unattended: bool,
    pub dom_control_available: bool,
    pub remote_provider_configured: bool,
    pub task_class: Option<String>,
}

/// A remote browser provider. Mirrors
/// `bridge_core::browser_bridge::RemoteBrowserConfig`; the bearer token is
/// named by environment variable, never carried inline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBrowserConfig {
    pub endpoint: String,
    pub bearer_token_env: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserActionParams {
    pub request: BrowserActionRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetBrowserPermissionParams {
    pub permission: BrowserPermission,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveBrowserApprovalParams {
    pub approval_id: String,
    pub allow: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteBrowserParams {
    pub request: BrowserRouteRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigureRemoteBrowserParams {
    /// Omitted or null clears the configured remote provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<RemoteBrowserConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartRemoteBrowserParams {
    pub initial_url: String,
}

/// `browser/route_browser`'s result. Mirrors
/// `bridge_core::browser_bridge::BrowserRouteDecision`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRouteDecision {
    pub route: String,
    pub reason: String,
    pub requires_user_grant: bool,
}

/// Mirrors `bridge_core::browser_bridge::BrowserSkill` — a bundled scripted
/// flow; `steps` is the skill's own script format, not contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BrowserSkill {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub description: String,
    pub steps: Vec<serde_json::Value>,
}

/// `browser/browser_skills`' result: a bare array on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct BrowserSkillsResult(pub Vec<BrowserSkill>);

/// `browser/browser_action`'s result: the queued command's id, a bare string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct BrowserActionResult(pub String);

/// `browser/install_browser_native_host`'s result: the installed manifest
/// path, a bare string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct InstallBrowserNativeHostResult(pub String);

/// `browser/detach_browser`'s result: the detach grant id, a bare string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct DetachBrowserResult(pub String);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn browser_action_requests_round_trip_with_camel_case_wire_names() {
        let action = BrowserActionParams {
            request: BrowserActionRequest {
                kind: "click".into(),
                element_id: Some("submit".into()),
                text: None,
                url: None,
                x: Some(12.5),
                y: Some(48.0),
                tab_id: Some(3),
                sensitive_kind: None,
                expected_domain: Some("example.test".into()),
                actor: Some("worker-1".into()),
            },
        };
        let wire = serde_json::to_value(&action).unwrap();
        assert_eq!(wire["request"]["elementId"], json!("submit"));
        assert_eq!(wire["request"]["tabId"], json!(3));
        assert_eq!(wire["request"]["expectedDomain"], json!("example.test"));
        assert_eq!(wire["request"]["sensitiveKind"], json!(null));
        assert_eq!(round_trip(&action), action);
    }

    #[test]
    fn route_requests_round_trip() {
        let route = RouteBrowserParams {
            request: BrowserRouteRequest {
                structured_api_available: false,
                needs_user_auth: true,
                needs_isolation: false,
                needs_parallelism: false,
                needs_geo_or_proxy: false,
                unattended: false,
                dom_control_available: true,
                remote_provider_configured: false,
                task_class: Some("checkout".into()),
            },
        };
        let wire = serde_json::to_value(&route).unwrap();
        assert_eq!(wire["request"]["structuredApiAvailable"], json!(false));
        assert_eq!(wire["request"]["needsGeoOrProxy"], json!(false));
        assert_eq!(wire["request"]["taskClass"], json!("checkout"));
        assert_eq!(round_trip(&route), route);
    }

    #[test]
    fn permissions_and_leases_round_trip() {
        let permission = SetBrowserPermissionParams { permission: BrowserPermission::ReadOnly };
        assert_eq!(
            serde_json::to_value(&permission).unwrap(),
            json!({"permission": "read_only"})
        );
        assert_eq!(round_trip(&permission), permission);

        let approval =
            ResolveBrowserApprovalParams { approval_id: "a-1".into(), allow: true };
        assert_eq!(
            serde_json::to_value(&approval).unwrap(),
            json!({"approvalId": "a-1", "allow": true})
        );
        assert_eq!(round_trip(&approval), approval);

        let start = StartRemoteBrowserParams { initial_url: "https://example.test".into() };
        assert_eq!(
            serde_json::to_value(&start).unwrap(),
            json!({"initialUrl": "https://example.test"})
        );
        assert_eq!(round_trip(&start), start);
    }

    #[test]
    fn clearing_the_remote_provider_is_an_absent_config() {
        let clear = ConfigureRemoteBrowserParams { config: None };
        assert_eq!(serde_json::to_value(&clear).unwrap(), json!({}));
        assert_eq!(round_trip(&clear), clear);
        assert_eq!(
            serde_json::from_value::<ConfigureRemoteBrowserParams>(json!({"config": null}))
                .unwrap(),
            clear,
            "an explicit null clears the provider too"
        );

        let configure = ConfigureRemoteBrowserParams {
            config: Some(RemoteBrowserConfig {
                endpoint: "wss://remote.test".into(),
                bearer_token_env: "REMOTE_BROWSER_TOKEN".into(),
                enabled: true,
            }),
        };
        assert_eq!(
            serde_json::to_value(&configure).unwrap()["config"]["bearerTokenEnv"],
            json!("REMOTE_BROWSER_TOKEN")
        );
        assert_eq!(round_trip(&configure), configure);
    }

    #[test]
    fn browser_params_reject_incomplete_and_unknown_payloads() {
        assert!(serde_json::from_value::<BrowserActionParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<BrowserActionRequest>(json!({})).is_err(),
            "kind is required"
        );
        assert!(serde_json::from_value::<RouteBrowserParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<BrowserRouteRequest>(json!({"structuredApiAvailable": true}))
                .is_err(),
            "every routing signal is required"
        );
        assert!(serde_json::from_value::<SetBrowserPermissionParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<SetBrowserPermissionParams>(json!({"permission": "write"}))
                .is_err(),
            "unknown permissions must be rejected"
        );
        assert!(
            serde_json::from_value::<SetBrowserPermissionParams>(
                json!({"permission": "readOnly"})
            )
            .is_err(),
            "permissions are snake_case, matching core"
        );
        assert!(serde_json::from_value::<ResolveBrowserApprovalParams>(
            json!({"approvalId": "a-1"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<ResolveBrowserApprovalParams>(
                json!({"approval_id": "a-1", "allow": true})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(serde_json::from_value::<StartRemoteBrowserParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<StartRemoteBrowserParams>(json!({"url": "https://x.test"}))
                .is_err(),
            "params reject arguments the contract does not name"
        );
    }
}
