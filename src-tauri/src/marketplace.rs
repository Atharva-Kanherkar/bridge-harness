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
    pub supported_actions: Vec<MarketplaceAction>,
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
        MarketplaceProvider::Codex => &[["plugin", "list", "--available", "--json"].as_slice()],
        MarketplaceProvider::Claude => &[["plugin", "list", "--available", "--json"].as_slice()],
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
            let mut candidates = Vec::new();
            for key in [
                "plugins",
                "items",
                "entries",
                "available",
                "installed",
                "marketplaces",
            ] {
                if let Some(Value::Array(items)) = map.get(key) {
                    candidates.extend(items.iter().filter(|item| item.is_object()));
                }
            }
            if candidates.is_empty() {
                vec![value]
            } else {
                candidates
            }
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
    let source = source_value(object);
    let repository = string_field(
        object,
        &["repository", "repositoryUrl", "repository_url", "repo"],
    )
    .or_else(|| source_url(object));

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
        source,
        repository,
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
        supported_actions: supported_actions(provider),
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

fn source_value(map: &serde_json::Map<String, Value>) -> Option<String> {
    string_field(map, &["source", "sourceUrl", "source_url"]).or_else(|| {
        map.get("source")
            .and_then(Value::as_object)
            .and_then(|source| string_field(source, &["url", "path", "source"]))
    })
}

fn source_url(map: &serde_json::Map<String, Value>) -> Option<String> {
    map.get("source")
        .and_then(Value::as_object)
        .and_then(|source| string_field(source, &["url"]))
}

fn supported_actions(provider: MarketplaceProvider) -> Vec<MarketplaceAction> {
    match provider {
        MarketplaceProvider::Codex => vec![
            MarketplaceAction::Install,
            MarketplaceAction::Update,
            MarketplaceAction::Uninstall,
            MarketplaceAction::Authenticate,
        ],
        MarketplaceProvider::Claude => vec![
            MarketplaceAction::Install,
            MarketplaceAction::Enable,
            MarketplaceAction::Disable,
            MarketplaceAction::Update,
            MarketplaceAction::Uninstall,
            MarketplaceAction::Authenticate,
        ],
    }
}

fn action_args(
    provider: MarketplaceProvider,
    action: MarketplaceAction,
    plugin_id: &str,
    marketplace: Option<&str>,
) -> Result<Vec<String>, String> {
    if action == MarketplaceAction::Authenticate {
        let auth_target = plugin_id.split('@').next().unwrap_or(plugin_id);
        return Ok(match provider {
            MarketplaceProvider::Codex => vec!["mcp".into(), "login".into(), auth_target.into()],
            MarketplaceProvider::Claude => vec!["/mcp".into(), auth_target.into()],
        });
    }
    let has_marketplace = plugin_id.contains('@');
    let selector = if has_marketplace {
        plugin_id.to_owned()
    } else if let Some(marketplace) = marketplace.filter(|value| !value.trim().is_empty()) {
        format!("{plugin_id}@{marketplace}")
    } else {
        plugin_id.to_owned()
    };
    match provider {
        MarketplaceProvider::Codex => match action {
            MarketplaceAction::Install | MarketplaceAction::Update => Ok(vec![
                "plugin".into(),
                "add".into(),
                selector,
                "--json".into(),
            ]),
            MarketplaceAction::Uninstall => Ok(vec![
                "plugin".into(),
                "remove".into(),
                selector,
                "--json".into(),
            ]),
            MarketplaceAction::Enable | MarketplaceAction::Disable => {
                Err("This Codex CLI does not expose per-plugin enable or disable commands".into())
            }
            MarketplaceAction::Authenticate => unreachable!(),
        },
        MarketplaceProvider::Claude => {
            Ok(vec!["plugin".into(), action_name(action).into(), selector])
        }
    }
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
    let args =
        action_args(provider, action, plugin_id, marketplace).map_err(BridgeError::Adapter)?;
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
            )
            .unwrap(),
            vec!["plugin", "add", "vercel@official", "--json"]
        );
        assert_eq!(
            action_args(
                MarketplaceProvider::Claude,
                MarketplaceAction::Authenticate,
                "vercel@official",
                None
            )
            .unwrap(),
            vec!["/mcp", "vercel"]
        );
        assert_eq!(
            action_args(
                MarketplaceProvider::Claude,
                MarketplaceAction::Install,
                "vercel@official",
                Some("official")
            )
            .unwrap(),
            vec!["plugin", "install", "vercel@official"]
        );
        assert!(action_args(
            MarketplaceProvider::Codex,
            MarketplaceAction::Disable,
            "vercel@official",
            None
        )
        .is_err());
    }

    #[test]
    fn parses_installed_and_available_envelope_entries() {
        let source = json!({
            "installed": [{"pluginId": "one@official", "name": "one", "installed": true}],
            "available": [{"pluginId": "two@official", "name": "two", "installed": false}]
        });
        let variants = parse_variants(MarketplaceProvider::Codex, &source);
        assert_eq!(variants.len(), 2);
        assert!(variants
            .iter()
            .any(|variant| variant.plugin_id == "one@official"));
        assert!(variants
            .iter()
            .any(|variant| variant.plugin_id == "two@official"));
    }

    #[test]
    fn extracts_repository_url_from_structured_source() {
        let source = json!([{"pluginId": "demo@official", "name": "demo", "source": {"source": "url", "url": "https://github.com/example/demo.git"}}]);
        let variant = parse_variants(MarketplaceProvider::Claude, &source).remove(0);
        assert_eq!(
            variant.repository.as_deref(),
            Some("https://github.com/example/demo.git")
        );
        assert_eq!(
            variant.source.as_deref(),
            Some("https://github.com/example/demo.git")
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
