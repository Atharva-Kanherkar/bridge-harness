use crate::{binary, BridgeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, process::Command};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MarketplaceProvider {
    Codex,
    Claude,
}

impl MarketplaceProvider {
    fn binary(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginVariant {
    pub provider: MarketplaceProvider,
    pub plugin_id: String,
    pub name: String,
    pub description: Option<String>,
    pub marketplace: Option<String>,
    pub version: Option<String>,
    pub source: Option<String>,
    pub repository: Option<String>,
    pub publisher: Option<String>,
    pub capabilities: Vec<String>,
    pub mcp_endpoint: Option<String>,
    pub connector_type: Option<String>,
    pub installed: bool,
    pub enabled: bool,
    pub authentication_state: String,
    pub shared_auth_mechanism: Option<String>,
    pub portable_mcp: bool,
    pub compatibility_notes: Vec<String>,
    pub provider_metadata: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCatalog {
    pub provider: MarketplaceProvider,
    pub available: bool,
    pub variants: Vec<PluginVariant>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceCatalog {
    pub providers: Vec<ProviderCatalog>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MarketplaceAction {
    Install,
    Enable,
    Disable,
    Update,
    Uninstall,
    Authenticate,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceActionResult {
    pub provider: MarketplaceProvider,
    pub plugin_id: String,
    pub action: MarketplaceAction,
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
}

pub fn catalog() -> MarketplaceCatalog {
    MarketplaceCatalog {
        providers: [MarketplaceProvider::Codex, MarketplaceProvider::Claude]
            .into_iter()
            .map(provider_catalog)
            .collect(),
    }
}

fn provider_catalog(provider: MarketplaceProvider) -> ProviderCatalog {
    let Some(binary_path) = binary::resolve(provider.binary()) else {
        return ProviderCatalog {
            provider,
            available: false,
            variants: Vec::new(),
            error: Some(format!("{} CLI is not installed", provider.binary())),
        };
    };

    let commands: &[&[&str]] = match provider {
        MarketplaceProvider::Codex => &[
            &["plugin", "list", "--available", "--json"],
            &["plugin", "marketplace", "list", "--json"],
        ],
        MarketplaceProvider::Claude => &[
            &["plugin", "list", "--json"],
            &["plugin", "marketplace", "list", "--json"],
        ],
    };

    let mut variants = BTreeMap::<String, PluginVariant>::new();
    let mut failures = Vec::new();
    for args in commands {
        match Command::new(&binary_path).args(*args).output() {
            Ok(output) if output.status.success() => {
                match serde_json::from_slice::<Value>(&output.stdout) {
                    Ok(value) => {
                        for variant in parse_variants(provider, &value) {
                            variants
                                .entry(variant.plugin_id.clone())
                                .and_modify(|existing| merge_variant(existing, &variant))
                                .or_insert(variant);
                        }
                    }
                    Err(_) => failures.push(format!(
                        "{} returned non-JSON marketplace data",
                        provider.binary()
                    )),
                }
            }
            Ok(output) => {
                let detail = String::from_utf8_lossy(&output.stderr);
                failures.push(sanitize_error(if detail.trim().is_empty() {
                    "marketplace command failed"
                } else {
                    detail.trim()
                }));
            }
            Err(error) => failures.push(sanitize_error(&error.to_string())),
        }
    }

    ProviderCatalog {
        provider,
        available: true,
        variants: variants.into_values().collect(),
        error: (!failures.is_empty()).then(|| failures.join(" · ")),
    }
}

fn merge_variant(existing: &mut PluginVariant, incoming: &PluginVariant) {
    existing.installed |= incoming.installed;
    existing.enabled |= incoming.enabled;
    if existing.authentication_state == "unknown" {
        existing.authentication_state = incoming.authentication_state.clone();
    }
    for capability in &incoming.capabilities {
        if !existing.capabilities.contains(capability) {
            existing.capabilities.push(capability.clone());
        }
    }
}

fn parse_variants(provider: MarketplaceProvider, root: &Value) -> Vec<PluginVariant> {
    candidate_objects(root)
        .into_iter()
        .filter_map(|value| parse_variant(provider, value))
        .collect()
}

fn candidate_objects(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(items) => items.iter().filter(|item| item.is_object()).collect(),
        Value::Object(map) => {
            for key in [
                "plugins",
                "items",
                "entries",
                "available",
                "installed",
                "marketplaces",
            ] {
                if let Some(Value::Array(items)) = map.get(key) {
                    return items.iter().filter(|item| item.is_object()).collect();
                }
            }
            vec![value]
        }
        _ => Vec::new(),
    }
}

fn parse_variant(provider: MarketplaceProvider, value: &Value) -> Option<PluginVariant> {
    let object = value.as_object()?;
    let plugin_id = string_field(object, &["id", "pluginId", "plugin_id", "name"])?;
    let name = string_field(object, &["displayName", "display_name", "title", "name"])
        .unwrap_or_else(|| plugin_id.clone());
    let connector_type = string_field(
        object,
        &["connectorType", "connector_type", "transport", "type"],
    );
    let mcp_endpoint = string_field(
        object,
        &[
            "mcpEndpoint",
            "mcp_endpoint",
            "serverUrl",
            "server_url",
            "url",
        ],
    );
    let capabilities = object
        .get("capabilities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let authentication_state = string_field(
        object,
        &[
            "authenticationState",
            "authentication_state",
            "authStatus",
            "auth_status",
        ],
    )
    .unwrap_or_else(|| "unknown".into())
    .to_lowercase();
    let shared_auth_mechanism = string_field(
        object,
        &[
            "sharedAuthMechanism",
            "shared_auth_mechanism",
            "credentialSource",
            "credential_source",
        ],
    );
    let provider_specific = connector_type.as_deref().is_some_and(|kind| {
        matches!(
            kind.to_lowercase().as_str(),
            "connector" | "hosted_connector" | "hosted-connector"
        )
    });

    Some(PluginVariant {
        provider,
        plugin_id,
        name,
        description: string_field(object, &["description", "summary"]),
        marketplace: string_field(
            object,
            &["marketplace", "marketplaceName", "marketplace_name"],
        ),
        version: string_field(object, &["version"]),
        source: string_field(object, &["source", "sourceUrl", "source_url"]),
        repository: string_field(
            object,
            &["repository", "repositoryUrl", "repository_url", "repo"],
        ),
        publisher: string_field(object, &["publisher", "author", "owner"]),
        capabilities,
        mcp_endpoint: mcp_endpoint.clone(),
        connector_type,
        installed: bool_field(object, &["installed", "isInstalled", "is_installed"]),
        enabled: bool_field(object, &["enabled", "isEnabled", "is_enabled"]),
        authentication_state,
        shared_auth_mechanism,
        portable_mcp: mcp_endpoint.is_some() && !provider_specific,
        compatibility_notes: if provider_specific {
            vec!["Provider-specific connector; conversion is not supported".into()]
        } else {
            Vec::new()
        },
        provider_metadata: redact_sensitive_json(value),
    })
}

fn string_field(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn bool_field(map: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| map.get(*key).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn action_args(
    provider: MarketplaceProvider,
    action: MarketplaceAction,
    plugin_id: &str,
    marketplace: Option<&str>,
) -> Vec<String> {
    if action == MarketplaceAction::Authenticate {
        return match provider {
            MarketplaceProvider::Codex => vec!["mcp".into(), "login".into(), plugin_id.into()],
            MarketplaceProvider::Claude => vec!["/mcp".into(), plugin_id.into()],
        };
    }
    let mut args = vec![
        "plugin".into(),
        action_name(action).into(),
        plugin_id.into(),
    ];
    if action == MarketplaceAction::Install {
        if let Some(marketplace) = marketplace.filter(|value| !value.trim().is_empty()) {
            args.extend(["--marketplace".into(), marketplace.into()]);
        }
    }
    args
}

fn action_name(action: MarketplaceAction) -> &'static str {
    match action {
        MarketplaceAction::Install => "install",
        MarketplaceAction::Enable => "enable",
        MarketplaceAction::Disable => "disable",
        MarketplaceAction::Update => "update",
        MarketplaceAction::Uninstall => "uninstall",
        MarketplaceAction::Authenticate => "authenticate",
    }
}

pub fn execute_action(
    provider: MarketplaceProvider,
    plugin_id: &str,
    marketplace: Option<&str>,
    action: MarketplaceAction,
) -> Result<MarketplaceActionResult, BridgeError> {
    let plugin_id = plugin_id.trim();
    if plugin_id.is_empty()
        || plugin_id.starts_with('-')
        || plugin_id.chars().any(char::is_whitespace)
    {
        return Err(BridgeError::Invalid("Invalid provider plugin ID".into()));
    }
    let binary_path = binary::resolve(provider.binary()).ok_or_else(|| {
        BridgeError::Adapter(format!("{} CLI is not installed", provider.binary()))
    })?;
    let args = action_args(provider, action, plugin_id, marketplace);
    let output = Command::new(binary_path).args(&args).output()?;
    let stdout = sanitize_error(String::from_utf8_lossy(&output.stdout).trim());
    let stderr = sanitize_error(String::from_utf8_lossy(&output.stderr).trim());
    let success = output.status.success();
    Ok(MarketplaceActionResult {
        provider,
        plugin_id: plugin_id.into(),
        action,
        success,
        message: if stdout.is_empty() {
            if success {
                format!("{} completed", action_name(action))
            } else {
                format!("{} failed", action_name(action))
            }
        } else {
            stdout
        },
        error: (!success).then_some(if stderr.is_empty() {
            "Provider command failed".into()
        } else {
            stderr
        }),
    })
}

pub fn sanitize_error(value: &str) -> String {
    let mut redact_following = 0;
    value
        .split_whitespace()
        .map(|part| {
            let lower = part.to_lowercase();
            if redact_following > 0 {
                redact_following -= 1;
                return "[REDACTED]";
            }
            if [
                "token",
                "secret",
                "authorization",
                "password",
                "api_key",
                "apikey",
                "access_key",
            ]
            .iter()
            .any(|needle| lower.contains(needle))
            {
                redact_following = 2;
                "[REDACTED]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_sensitive_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter_map(|(key, value)| {
                    let lower = key.to_lowercase();
                    let sensitive = [
                        "token",
                        "secret",
                        "authorization",
                        "password",
                        "api_key",
                        "apikey",
                        "access_key",
                        "credential",
                    ]
                    .iter()
                    .any(|needle| lower.contains(needle));
                    (!sensitive).then(|| (key.clone(), redact_sensitive_json(value)))
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_sensitive_json).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_common_catalog_envelope_and_retains_metadata() {
        let source = json!({"plugins": [{
            "id": "vercel", "displayName": "Vercel", "repository": "https://github.com/vercel/mcp",
            "publisher": "Vercel", "installed": true, "enabled": false, "authStatus": "required",
            "mcpEndpoint": "https://mcp.vercel.com", "capabilities": ["deployments"]
        }]});
        let variants = parse_variants(MarketplaceProvider::Codex, &source);
        assert_eq!(variants.len(), 1);
        assert!(variants[0].installed);
        assert!(!variants[0].enabled);
        assert_eq!(variants[0].authentication_state, "required");
        assert_eq!(variants[0].provider_metadata["publisher"], "Vercel");
    }

    #[test]
    fn provider_specific_connector_is_not_portable() {
        let source = json!([{"id": "hosted", "connectorType": "hosted_connector", "url": "https://example.test"}]);
        let variant = parse_variants(MarketplaceProvider::Claude, &source).remove(0);
        assert!(!variant.portable_mcp);
        assert!(!variant.compatibility_notes.is_empty());
    }

    #[test]
    fn action_arguments_never_contain_credentials() {
        assert_eq!(
            action_args(
                MarketplaceProvider::Codex,
                MarketplaceAction::Install,
                "vercel",
                Some("official")
            ),
            vec!["plugin", "install", "vercel", "--marketplace", "official"]
        );
        assert_eq!(
            action_args(
                MarketplaceProvider::Claude,
                MarketplaceAction::Authenticate,
                "vercel",
                None
            ),
            vec!["/mcp", "vercel"]
        );
    }

    #[test]
    fn sanitizes_secret_shaped_output() {
        assert_eq!(
            sanitize_error("failed token=abc request"),
            "failed [REDACTED] [REDACTED]"
        );
        assert_eq!(
            sanitize_error("Authorization: Bearer abc"),
            "[REDACTED] [REDACTED] [REDACTED]"
        );
    }

    #[test]
    fn catalog_metadata_drops_sensitive_keys_recursively() {
        let value =
            json!({"id": "demo", "token": "abc", "nested": {"apiKey": "def", "safe": true}});
        let redacted = redact_sensitive_json(&value);
        assert!(redacted.get("token").is_none());
        assert!(redacted["nested"].get("apiKey").is_none());
        assert_eq!(redacted["nested"]["safe"], true);
    }
}
