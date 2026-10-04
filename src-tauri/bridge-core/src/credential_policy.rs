//! Deployment credential policy: which credentials a harness may run on.
//!
//! Two independent dimensions, so location never decides billing:
//! [`ExecutionTopology`] says where the runtime lives, [`CredentialPolicy`]
//! says which credentials it accepts. A local company deployment may demand
//! API keys; a remote runner may use an approved enterprise credential.
//!
//! The policy is enforced where a harness starts, not inside the
//! `CredentialBroker`: Codex is asked which auth mode it is in (`account/read`)
//! before its thread opens, and Claude's launch environment is checked before
//! the sidecar spawns. A refusal is the stable [`BridgeError::CredentialPolicy`]
//! (wire code `credential_policy_violation`) naming what the deployment
//! requires. Harnesses whose credential source Bridge cannot yet classify are
//! refused under any restrictive policy rather than waved through.
//!
//! The default, [`CredentialPolicy::UserManaged`], changes nothing: no extra
//! round trip, no refusal.

use crate::BridgeError;
pub use bridge_protocol::messages::{CredentialPolicy, DeploymentInfo, ExecutionTopology};
use serde_json::Value;
use std::sync::RwLock;

/// Overrides the environment when set by the host (`bridged --credential-policy`).
static CONFIGURED: RwLock<Option<DeploymentInfo>> = RwLock::new(None);

pub const POLICY_ENV: &str = "BRIDGE_CREDENTIAL_POLICY";
pub const TOPOLOGY_ENV: &str = "BRIDGE_EXECUTION_TOPOLOGY";

/// Where a harness's credential actually comes from, as far as Bridge can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSource {
    /// A metered API key.
    ApiKey,
    /// A cloud-provider account (Bedrock, Vertex, Foundry): metered, and
    /// provisioned by whoever runs the account.
    CloudProvider,
    /// A consumer-subscription login (ChatGPT, Claude Pro/Max).
    Subscription,
    /// The harness reports no credential.
    SignedOut,
    /// A shape Bridge does not recognise.
    Unknown,
}

impl AuthSource {
    fn describe(self) -> &'static str {
        match self {
            AuthSource::ApiKey => "an API key",
            AuthSource::CloudProvider => "a cloud-provider account",
            AuthSource::Subscription => "a subscription login",
            AuthSource::SignedOut => "no credential",
            AuthSource::Unknown => "a credential Bridge cannot identify",
        }
    }
}

/// Whether a harness running on `source` may start under `policy`.
pub fn admits(policy: CredentialPolicy, source: AuthSource) -> bool {
    match policy {
        CredentialPolicy::UserManaged => true,
        CredentialPolicy::ApiKeyOnly => {
            matches!(source, AuthSource::ApiKey | AuthSource::CloudProvider)
        }
        CredentialPolicy::EnterpriseManaged => matches!(source, AuthSource::CloudProvider),
    }
}

/// What the operator must configure, for the refusal message.
fn requirement(policy: CredentialPolicy) -> &'static str {
    match policy {
        CredentialPolicy::UserManaged => "any sign-in",
        CredentialPolicy::ApiKeyOnly => "an API key or a cloud-provider account",
        CredentialPolicy::EnterpriseManaged => {
            "a cloud-provider account provisioned by the operator"
        }
    }
}

/// Pin the deployment for this process. Called once by the host before any
/// harness starts.
pub fn configure(deployment: DeploymentInfo) {
    *CONFIGURED.write().unwrap() = Some(deployment);
}

/// The deployment in force: the host's explicit configuration, else the
/// environment, else the default (embedded, user-managed). An environment value
/// that does not parse is ignored for the topology but never loosens the
/// policy: an unparseable policy fails closed to `ApiKeyOnly`.
pub fn active() -> DeploymentInfo {
    if let Some(configured) = *CONFIGURED.read().unwrap() {
        return configured;
    }
    deployment_from_env(|key| std::env::var(key).ok())
}

fn deployment_from_env(read: impl Fn(&str) -> Option<String>) -> DeploymentInfo {
    let topology = read(TOPOLOGY_ENV)
        .and_then(|value| ExecutionTopology::parse(&value))
        .unwrap_or_default();
    let credential_policy = match read(POLICY_ENV).filter(|value| !value.trim().is_empty()) {
        None => CredentialPolicy::default(),
        Some(value) => CredentialPolicy::parse(&value).unwrap_or(CredentialPolicy::ApiKeyOnly),
    };
    DeploymentInfo { topology, credential_policy }
}

fn violation(harness: &str, policy: CredentialPolicy, source: AuthSource) -> BridgeError {
    BridgeError::CredentialPolicy(format!(
        "{harness} is using {}, but this deployment's credential policy ({}) requires {}. \
         Configure that credential for {harness} and start the session again.",
        source.describe(),
        policy.name(),
        requirement(policy)
    ))
}

/// Refuse `harness` when its credential source is not allowed by `policy`.
pub fn admit(harness: &str, policy: CredentialPolicy, source: AuthSource) -> Result<(), BridgeError> {
    if admits(policy, source) {
        Ok(())
    } else {
        Err(violation(harness, policy, source))
    }
}

/// Classify Codex's `account/read` result. `{"account": null}` is signed out;
/// anything with an unrecognised `type` is `Unknown`, which a restrictive
/// policy refuses.
pub fn codex_auth_source(account_read: &Value) -> AuthSource {
    let account = account_read.get("account").unwrap_or(&Value::Null);
    if account.is_null() {
        return AuthSource::SignedOut;
    }
    match account.get("type").and_then(Value::as_str) {
        Some("apiKey") => AuthSource::ApiKey,
        Some("chatgpt") => AuthSource::Subscription,
        Some("amazonBedrock") => AuthSource::CloudProvider,
        _ => AuthSource::Unknown,
    }
}

/// Classify the credential Claude's sidecar will inherit from its launch
/// environment. A cloud-provider switch wins; otherwise `ANTHROPIC_API_KEY`
/// (which Claude Code prefers over a login); otherwise a subscription login.
pub fn claude_auth_source(read: impl Fn(&str) -> Option<String>) -> AuthSource {
    let set = |key: &str| read(key).is_some_and(|value| !value.trim().is_empty());
    let truthy = |key: &str| {
        read(key).is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "True"))
    };
    if ["CLAUDE_CODE_USE_BEDROCK", "CLAUDE_CODE_USE_VERTEX", "CLAUDE_CODE_USE_FOUNDRY"]
        .into_iter()
        .any(truthy)
    {
        AuthSource::CloudProvider
    } else if set("ANTHROPIC_API_KEY") {
        AuthSource::ApiKey
    } else {
        AuthSource::Subscription
    }
}

/// Gate a Claude launch on the environment it will inherit.
pub fn admit_claude_launch() -> Result<(), BridgeError> {
    let policy = active().credential_policy;
    if !policy.restricts() {
        return Ok(());
    }
    admit("Claude", policy, claude_auth_source(|key| std::env::var(key).ok()))
}

/// Harnesses Bridge can verify itself (Codex and Claude check their own
/// credential at launch). Every other adapter is refused under a restrictive
/// policy: its credential source is not something Bridge can classify yet, and
/// a policy that quietly skipped it would be a promise nobody keeps.
pub fn gate_adapter(adapter_id: &str) -> Result<(), BridgeError> {
    gate_adapter_under(active().credential_policy, adapter_id)
}

fn gate_adapter_under(policy: CredentialPolicy, adapter_id: &str) -> Result<(), BridgeError> {
    if !policy.restricts() || matches!(adapter_id, "codex" | "claude") {
        return Ok(());
    }
    Err(BridgeError::CredentialPolicy(format!(
        "{adapter_id} cannot be started: this deployment's credential policy ({}) requires {}, \
         and Bridge cannot yet verify which credential {adapter_id} uses. Use Codex or Claude \
         with an API key, or run with the user-managed policy.",
        policy.name(),
        requirement(policy)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> =
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |key| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn user_managed_admits_every_source() {
        for source in [
            AuthSource::ApiKey,
            AuthSource::CloudProvider,
            AuthSource::Subscription,
            AuthSource::SignedOut,
            AuthSource::Unknown,
        ] {
            assert!(admits(CredentialPolicy::UserManaged, source));
        }
    }

    #[test]
    fn api_key_only_refuses_subscriptions_signed_out_and_unrecognised_sources() {
        let policy = CredentialPolicy::ApiKeyOnly;
        assert!(admits(policy, AuthSource::ApiKey));
        assert!(admits(policy, AuthSource::CloudProvider));
        assert!(!admits(policy, AuthSource::Subscription));
        assert!(!admits(policy, AuthSource::SignedOut));
        assert!(!admits(policy, AuthSource::Unknown));
    }

    #[test]
    fn enterprise_managed_admits_only_the_operators_cloud_account() {
        let policy = CredentialPolicy::EnterpriseManaged;
        assert!(admits(policy, AuthSource::CloudProvider));
        for source in [AuthSource::ApiKey, AuthSource::Subscription, AuthSource::SignedOut, AuthSource::Unknown] {
            assert!(!admits(policy, source));
        }
    }

    #[test]
    fn codex_account_read_is_classified_fail_closed() {
        assert_eq!(codex_auth_source(&json!({"account": {"type": "apiKey"}})), AuthSource::ApiKey);
        assert_eq!(
            codex_auth_source(&json!({"account": {"type": "chatgpt", "email": "a@b.c", "planType": "plus"}})),
            AuthSource::Subscription
        );
        assert_eq!(
            codex_auth_source(&json!({"account": {"type": "amazonBedrock"}})),
            AuthSource::CloudProvider
        );
        assert_eq!(codex_auth_source(&json!({"account": null})), AuthSource::SignedOut);
        assert_eq!(codex_auth_source(&json!({})), AuthSource::SignedOut);
        assert_eq!(codex_auth_source(&json!({"account": {"type": "somethingNew"}})), AuthSource::Unknown);
        assert_eq!(codex_auth_source(&json!({"account": {}})), AuthSource::Unknown);
    }

    #[test]
    fn claude_source_prefers_cloud_then_api_key_then_subscription() {
        assert_eq!(claude_auth_source(env(&[])), AuthSource::Subscription);
        assert_eq!(claude_auth_source(env(&[("ANTHROPIC_API_KEY", "sk-ant-x")])), AuthSource::ApiKey);
        assert_eq!(claude_auth_source(env(&[("ANTHROPIC_API_KEY", "  ")])), AuthSource::Subscription);
        assert_eq!(
            claude_auth_source(env(&[("ANTHROPIC_API_KEY", "sk-ant-x"), ("CLAUDE_CODE_USE_BEDROCK", "1")])),
            AuthSource::CloudProvider
        );
        assert_eq!(claude_auth_source(env(&[("CLAUDE_CODE_USE_VERTEX", "0")])), AuthSource::Subscription);
    }

    #[test]
    fn a_refusal_is_the_stable_error_and_names_the_requirement() {
        let error = admit("Codex", CredentialPolicy::ApiKeyOnly, AuthSource::Subscription).unwrap_err();
        assert!(matches!(error, BridgeError::CredentialPolicy(_)));
        let message = error.to_string();
        assert!(message.contains("a subscription login"), "{message}");
        assert!(message.contains("api-key-only"), "{message}");
        assert!(message.contains("an API key or a cloud-provider account"), "{message}");
        assert_eq!(
            bridge_protocol::ErrorCode::from(&error),
            bridge_protocol::ErrorCode::CredentialPolicyViolation
        );
    }

    #[test]
    fn the_environment_selects_the_deployment_and_a_bad_policy_fails_closed() {
        assert_eq!(deployment_from_env(env(&[])), DeploymentInfo::default());
        let pinned = deployment_from_env(env(&[
            (POLICY_ENV, "api-key-only"),
            (TOPOLOGY_ENV, "remote-runner"),
        ]));
        assert_eq!(pinned.credential_policy, CredentialPolicy::ApiKeyOnly);
        assert_eq!(pinned.topology, ExecutionTopology::RemoteRunner);
        // A typo must not silently become the permissive default.
        let typo = deployment_from_env(env(&[(POLICY_ENV, "apikey")]));
        assert_eq!(typo.credential_policy, CredentialPolicy::ApiKeyOnly);
        assert_eq!(deployment_from_env(env(&[(POLICY_ENV, "")])).credential_policy, CredentialPolicy::UserManaged);
    }

    #[test]
    fn every_adapter_but_codex_and_claude_is_refused_under_a_restrictive_policy() {
        assert!(gate_adapter_under(CredentialPolicy::UserManaged, "opencode").is_ok());
        assert!(gate_adapter_under(CredentialPolicy::ApiKeyOnly, "codex").is_ok());
        assert!(gate_adapter_under(CredentialPolicy::ApiKeyOnly, "claude").is_ok());
        let refused = gate_adapter_under(CredentialPolicy::ApiKeyOnly, "opencode").unwrap_err();
        assert!(matches!(refused, BridgeError::CredentialPolicy(_)));
        assert!(refused.to_string().contains("api-key-only"));
        assert!(gate_adapter_under(CredentialPolicy::EnterpriseManaged, "cursor").is_err());
    }
}
