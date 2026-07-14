use crate::{binary, BridgeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

const CODEX_APP_CONNECTOR: &str = "app";
const MCP_CONNECTOR: &str = "mcp";
const CODEX_APP_SERVER_TIMEOUT: Duration = Duration::from_secs(75);
const CLAUDE_MCP_STATUS_TIMEOUT: Duration = Duration::from_secs(20);
const CLAUDE_MCP_LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

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
    pub app_connector_ids: Vec<String>,
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

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceAppAuthState {
    pub provider: MarketplaceProvider,
    pub connector_id: String,
    pub display_name: Option<String>,
    pub native_connector: bool,
    pub authentication_state: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeSdkConfiguration {
    pub plugins: Vec<String>,
    pub mcp_servers: BTreeMap<String, Value>,
}

pub fn catalog() -> MarketplaceCatalog {
    MarketplaceCatalog {
        providers: [MarketplaceProvider::Codex, MarketplaceProvider::Claude]
            .into_iter()
            .map(provider_catalog)
            .collect(),
    }
}

pub fn claude_sdk_configuration() -> ClaudeSdkConfiguration {
    let Some(binary_path) = binary::resolve("claude") else {
        return ClaudeSdkConfiguration::default();
    };
    let plugins = bounded_output(
        &binary_path,
        &["plugin", "list", "--json"],
        CLAUDE_MCP_STATUS_TIMEOUT,
    )
    .ok()
    .filter(|output| output.status.success())
    .and_then(|output| serde_json::from_slice::<Value>(&output.stdout).ok())
    .map(|value| {
        candidate_objects(&value)
            .into_iter()
            .filter_map(Value::as_object)
            .filter(|plugin| bool_field(plugin, &["enabled", "isEnabled", "is_enabled"]))
            .filter_map(|plugin| string_field(plugin, &["installPath", "install_path"]))
            .filter(|path| Path::new(path).is_dir())
            .fold(Vec::new(), |mut paths, path| {
                if !paths.contains(&path) {
                    paths.push(path);
                }
                paths
            })
    })
    .unwrap_or_default();
    let mcp_servers = bounded_output(&binary_path, &["mcp", "list"], CLAUDE_MCP_STATUS_TIMEOUT)
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            parse_claude_native_connector_configs(&String::from_utf8_lossy(&output.stdout))
        })
        .unwrap_or_default();
    ClaudeSdkConfiguration {
        plugins,
        mcp_servers,
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
    for connector_id in &incoming.app_connector_ids {
        if !existing.app_connector_ids.contains(connector_id) {
            existing.app_connector_ids.push(connector_id.clone());
        }
    }
    if incoming.connector_type.as_deref() == Some(CODEX_APP_CONNECTOR) {
        existing.connector_type = incoming.connector_type.clone();
        existing.portable_mcp = false;
        existing.compatibility_notes = incoming.compatibility_notes.clone();
    } else if existing.connector_type.is_none() && incoming.connector_type.is_some() {
        existing.connector_type = incoming.connector_type.clone();
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
    let mut connector_type = string_field(
        object,
        &["connectorType", "connector_type", "transport", "type"],
    );
    let mut mcp_endpoint = string_field(
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
    let source = source_value(object);
    let app_connector_ids = match provider {
        MarketplaceProvider::Codex => plugin_app_connector_ids(source.as_deref()),
        MarketplaceProvider::Claude => {
            let (ids, endpoint) = claude_plugin_connectors(&plugin_id, object);
            if mcp_endpoint.is_none() {
                mcp_endpoint = endpoint;
            }
            ids
        }
    };
    if !app_connector_ids.is_empty() {
        connector_type = Some(if provider == MarketplaceProvider::Codex {
            CODEX_APP_CONNECTOR.into()
        } else {
            "connector".into()
        });
    } else if connector_type.is_none() && mcp_endpoint.is_some() {
        connector_type = Some(MCP_CONNECTOR.into());
    }
    let provider_specific = connector_type.as_deref().is_some_and(|kind| {
        matches!(
            kind.to_lowercase().as_str(),
            "app" | "connector" | "hosted_connector" | "hosted-connector"
        )
    });
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
        app_connector_ids,
        installed: bool_field(object, &["installed", "isInstalled", "is_installed"])
            || (provider == MarketplaceProvider::Claude
                && string_field(object, &["installPath", "install_path"]).is_some()),
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

fn plugin_app_connector_ids(source: Option<&str>) -> Vec<String> {
    let Some(source) = source.filter(|value| !value.contains("://")) else {
        return Vec::new();
    };
    let Ok(contents) = fs::read(Path::new(source).join(".app.json")) else {
        return Vec::new();
    };
    serde_json::from_slice::<Value>(&contents)
        .ok()
        .and_then(|value| value.get("apps").and_then(Value::as_object).cloned())
        .map(|apps| {
            apps.values()
                .filter_map(|app| app.get("id").and_then(Value::as_str))
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn claude_plugin_connectors(
    plugin_id: &str,
    object: &serde_json::Map<String, Value>,
) -> (Vec<String>, Option<String>) {
    let plugin_name = plugin_id.split('@').next().unwrap_or(plugin_id);
    let Some(servers) = object.get("mcpServers").and_then(Value::as_object) else {
        return (Vec::new(), None);
    };
    let ids = servers
        .keys()
        .map(|server_name| format!("plugin:{plugin_name}:{server_name}"))
        .collect();
    let endpoint = (servers.len() == 1)
        .then(|| servers.values().next())
        .flatten()
        .and_then(Value::as_object)
        .and_then(|server| string_field(server, &["url"]));
    (ids, endpoint)
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
    connector_type: Option<&str>,
) -> Result<Vec<String>, String> {
    if action == MarketplaceAction::Authenticate {
        let auth_target = plugin_id.split('@').next().unwrap_or(plugin_id);
        return match provider {
            MarketplaceProvider::Codex if connector_type == Some(MCP_CONNECTOR) => {
                Ok(vec!["mcp".into(), "login".into(), auth_target.into()])
            }
            MarketplaceProvider::Codex if connector_type == Some(CODEX_APP_CONNECTOR) => Err(
                "Codex app connectors must be authorized in the native Codex plugin surface".into(),
            ),
            MarketplaceProvider::Codex => {
                Err("No supported Codex authentication route was found for this plugin".into())
            }
            MarketplaceProvider::Claude
                if plugin_id.starts_with("plugin:") || plugin_id.starts_with("claude.ai ") =>
            {
                Ok(vec!["mcp".into(), "login".into(), plugin_id.into()])
            }
            MarketplaceProvider::Claude => {
                Err("No supported Claude connector authentication route was found".into())
            }
        };
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
            MarketplaceAction::Update => Ok(vec![
                "plugin".into(),
                "add".into(),
                selector,
                "--json".into(),
            ]),
            MarketplaceAction::Install | MarketplaceAction::Uninstall => {
                Err("Codex plugin installation state is managed through app-server".into())
            }
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
    let native_claude_connector = provider == MarketplaceProvider::Claude
        && action == MarketplaceAction::Authenticate
        && plugin_id.starts_with("claude.ai ");
    if plugin_id.is_empty()
        || plugin_id.starts_with('-')
        || (!native_claude_connector && plugin_id.chars().any(char::is_whitespace))
    {
        return Err(BridgeError::Invalid("Invalid provider plugin ID".into()));
    }
    let binary_path = binary::resolve(provider.binary()).ok_or_else(|| {
        BridgeError::Adapter(format!("{} CLI is not installed", provider.binary()))
    })?;
    let auth_variant = (action == MarketplaceAction::Authenticate)
        .then(|| {
            provider_catalog(provider)
                .variants
                .into_iter()
                .find(|variant| variant.plugin_id == plugin_id)
        })
        .flatten();
    let connector_type = auth_variant
        .as_ref()
        .and_then(|variant| variant.connector_type.clone());
    if provider == MarketplaceProvider::Codex {
        if action == MarketplaceAction::Install
            || (action == MarketplaceAction::Authenticate
                && connector_type.as_deref() == Some(CODEX_APP_CONNECTOR))
        {
            return codex_plugin_install(&binary_path, plugin_id, marketplace, action);
        }
        if action == MarketplaceAction::Uninstall {
            return codex_plugin_uninstall(&binary_path, plugin_id);
        }
    }
    if provider == MarketplaceProvider::Claude && action == MarketplaceAction::Authenticate {
        let connector_id = if plugin_id.starts_with("plugin:") || native_claude_connector {
            plugin_id.to_owned()
        } else {
            let variant = auth_variant.ok_or_else(|| {
                BridgeError::Adapter(
                    "Claude plugin is not installed or has no discoverable connectors".into(),
                )
            })?;
            preferred_claude_connector(&binary_path, &variant.app_connector_ids)?
        };
        return claude_connector_authenticate(&binary_path, plugin_id, &connector_id);
    }
    let args = action_args(
        provider,
        action,
        plugin_id,
        marketplace,
        connector_type.as_deref(),
    )
    .map_err(BridgeError::Adapter)?;
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

fn preferred_claude_connector(
    binary_path: &Path,
    connector_ids: &[String],
) -> Result<String, BridgeError> {
    if connector_ids.is_empty() {
        return Err(BridgeError::Adapter(
            "This Claude plugin does not declare an MCP connector".into(),
        ));
    }
    let states = bounded_output(binary_path, &["mcp", "list"], CLAUDE_MCP_STATUS_TIMEOUT)
        .ok()
        .filter(|output| output.status.success())
        .map(|output| parse_claude_mcp_auth_states(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default();
    Ok(connector_ids
        .iter()
        .find(|connector_id| {
            states.iter().any(|state| {
                state.connector_id == connector_id.as_str()
                    && state.authentication_state == "required"
            })
        })
        .unwrap_or(&connector_ids[0])
        .clone())
}

fn claude_connector_authenticate(
    binary_path: &Path,
    plugin_id: &str,
    connector_id: &str,
) -> Result<MarketplaceActionResult, BridgeError> {
    let output = bounded_output(
        binary_path,
        &["mcp", "login", connector_id],
        CLAUDE_MCP_LOGIN_TIMEOUT,
    )?;
    let success = output.status.success();
    let stderr = sanitize_error(String::from_utf8_lossy(&output.stderr).trim());
    Ok(MarketplaceActionResult {
        provider: MarketplaceProvider::Claude,
        plugin_id: plugin_id.into(),
        action: MarketplaceAction::Authenticate,
        success,
        message: if success {
            "Connected in Claude Code".into()
        } else {
            "Claude Code authentication did not complete".into()
        },
        error: (!success).then_some(if stderr.is_empty() {
            "Claude Code authentication did not complete".into()
        } else {
            stderr
        }),
    })
}

fn codex_plugin_install(
    binary_path: &Path,
    plugin_id: &str,
    marketplace: Option<&str>,
    action: MarketplaceAction,
) -> Result<MarketplaceActionResult, BridgeError> {
    let plugin_name = plugin_id.split('@').next().unwrap_or(plugin_id);
    let marketplace_name = marketplace
        .filter(|value| !value.trim().is_empty())
        .or_else(|| plugin_id.split_once('@').map(|(_, value)| value));
    let marketplace_path = marketplace_name
        .and_then(|name| codex_marketplace_path(binary_path, name))
        .map(|path| path.to_string_lossy().into_owned());
    let params = match marketplace_path {
        Some(path) => serde_json::json!({
            "pluginName": plugin_name,
            "marketplacePath": path,
            "remoteMarketplaceName": null,
        }),
        None => serde_json::json!({
            "pluginName": plugin_name,
            "marketplacePath": null,
            "remoteMarketplaceName": marketplace_name,
        }),
    };
    let result = codex_app_server_request(binary_path, "plugin/install", params)?;
    let authorization_urls = result
        .get("appsNeedingAuth")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|app| app.get("installUrl").and_then(Value::as_str))
        .filter(|url| url.starts_with("https://"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !authorization_urls.is_empty() {
        open_authorization_urls(&authorization_urls)?;
    }
    Ok(MarketplaceActionResult {
        provider: MarketplaceProvider::Codex,
        plugin_id: plugin_id.into(),
        action,
        success: true,
        message: if authorization_urls.is_empty() {
            "Installed in Codex; no additional authorization is required".into()
        } else {
            "Installed in Codex; finish authorization in the browser window that opened".into()
        },
        error: None,
    })
}

fn codex_plugin_uninstall(
    binary_path: &Path,
    plugin_id: &str,
) -> Result<MarketplaceActionResult, BridgeError> {
    codex_app_server_request(
        binary_path,
        "plugin/uninstall",
        serde_json::json!({"pluginId": plugin_id}),
    )?;
    Ok(MarketplaceActionResult {
        provider: MarketplaceProvider::Codex,
        plugin_id: plugin_id.into(),
        action: MarketplaceAction::Uninstall,
        success: true,
        message: "Uninstalled from Codex".into(),
        error: None,
    })
}

fn codex_marketplace_path(binary_path: &Path, marketplace: &str) -> Option<PathBuf> {
    let output = Command::new(binary_path)
        .args(["plugin", "marketplace", "list", "--json"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let value = serde_json::from_slice::<Value>(&output.stdout).ok()?;
    let root = value
        .get("marketplaces")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("name").and_then(Value::as_str) == Some(marketplace))?
        .get("root")?
        .as_str()?;
    let path = Path::new(root).join(".agents/plugins/marketplace.json");
    path.is_file().then_some(path)
}

pub fn app_auth_states() -> Result<Vec<MarketplaceAppAuthState>, BridgeError> {
    let mut states = Vec::new();
    let mut provider_available = false;
    if let Some(binary_path) = binary::resolve("codex") {
        provider_available = true;
        if let Ok(result) = codex_app_server_request(
            &binary_path,
            "app/list",
            serde_json::json!({"forceRefetch": true}),
        ) {
            states.extend(parse_app_auth_states(&result));
        }
    }
    if let Some(binary_path) = binary::resolve("claude") {
        provider_available = true;
        if let Ok(output) =
            bounded_output(&binary_path, &["mcp", "list"], CLAUDE_MCP_STATUS_TIMEOUT)
        {
            if output.status.success() {
                states.extend(parse_claude_mcp_auth_states(&String::from_utf8_lossy(
                    &output.stdout,
                )));
            }
        }
    }
    if provider_available {
        Ok(states)
    } else {
        Err(BridgeError::Adapter(
            "Neither Codex nor Claude CLI is installed".into(),
        ))
    }
}

fn parse_app_auth_states(result: &Value) -> Vec<MarketplaceAppAuthState> {
    result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|app| {
            let connector_id = app.get("id")?.as_str()?.trim();
            let state = match app.get("isAccessible").and_then(Value::as_bool) {
                Some(true) => "connected",
                Some(false) if app.get("installUrl").and_then(Value::as_str).is_some() => {
                    "required"
                }
                _ => return None,
            };
            Some(MarketplaceAppAuthState {
                provider: MarketplaceProvider::Codex,
                connector_id: connector_id.into(),
                display_name: None,
                native_connector: false,
                authentication_state: state.into(),
            })
        })
        .collect()
}

fn parse_claude_mcp_auth_states(output: &str) -> Vec<MarketplaceAppAuthState> {
    output
        .lines()
        .filter_map(|line| {
            let authentication_state = if line.contains("- ✔ Connected") {
                "connected"
            } else if line.contains("- ! Needs authentication") {
                "required"
            } else {
                return None;
            };
            let endpoint_index = line.find("https://").or_else(|| line.find("http://"))?;
            let connector_id = line[..endpoint_index].trim().trim_end_matches(':').trim();
            if connector_id.is_empty() {
                return None;
            }
            let native_connector = connector_id.starts_with("claude.ai ");
            Some(MarketplaceAppAuthState {
                provider: MarketplaceProvider::Claude,
                connector_id: connector_id.into(),
                display_name: native_connector
                    .then(|| connector_id.trim_start_matches("claude.ai ").to_owned()),
                native_connector,
                authentication_state: authentication_state.into(),
            })
        })
        .collect()
}

fn parse_claude_native_connector_configs(output: &str) -> BTreeMap<String, Value> {
    output
        .lines()
        .filter_map(|line| {
            let endpoint_index = line.find("https://")?;
            let connector_id = line[..endpoint_index].trim().trim_end_matches(':').trim();
            if !connector_id.starts_with("claude.ai ") {
                return None;
            }
            let endpoint = line[endpoint_index..].split_whitespace().next()?;
            Some((
                connector_id.to_owned(),
                serde_json::json!({"type": "http", "url": endpoint}),
            ))
        })
        .collect()
}

fn bounded_output(
    binary_path: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<std::process::Output, BridgeError> {
    let mut command = Command::new(binary_path);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| BridgeError::Adapter("Provider command stdout unavailable".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| BridgeError::Adapter("Provider command stderr unavailable".into()))?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            let stdout = stdout_reader
                .join()
                .map_err(|_| BridgeError::Adapter("Provider stdout reader failed".into()))??;
            let stderr = stderr_reader
                .join()
                .map_err(|_| BridgeError::Adapter("Provider stderr reader failed".into()))??;
            return Ok(std::process::Output {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= deadline {
            let _ = crate::adapters::terminate_process_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(BridgeError::Adapter(format!(
                "{} command timed out",
                binary_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Provider")
            )));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn codex_app_server_request(
    binary_path: &Path,
    method: &str,
    params: Value,
) -> Result<Value, BridgeError> {
    let mut command = Command::new(binary_path);
    command
        .args(["app-server", "--listen", "stdio://"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::adapters::configure_process_group(&mut command);
    let mut child = command.spawn()?;
    let result = (|| {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| BridgeError::Adapter("Codex app-server stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BridgeError::Adapter("Codex app-server stdout unavailable".into()))?;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let frame = serde_json::from_str::<Value>(line.trim())
                            .map_err(|_| "Codex app-server returned an invalid frame".to_owned());
                        if sender.send(frame).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(Err("Codex app-server output closed".into()));
                        break;
                    }
                }
            }
        });
        let deadline = Instant::now() + CODEX_APP_SERVER_TIMEOUT;
        write_app_server_frame(
            &mut stdin,
            &serde_json::json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": {"name": "bridge", "title": "Bridge", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"experimentalApi": true, "requestAttestation": false}
                }
            }),
        )?;
        checked_app_server_result(receive_app_server_response(&receiver, 1, deadline)?)?;
        write_app_server_frame(&mut stdin, &serde_json::json!({"method": "initialized"}))?;
        write_app_server_frame(
            &mut stdin,
            &serde_json::json!({"method": method, "id": 2, "params": params}),
        )?;
        let response = receive_app_server_response(&receiver, 2, deadline)?;
        checked_app_server_result(response)
    })();
    stop_app_server(&mut child);
    result
}

fn write_app_server_frame(stdin: &mut ChildStdin, value: &Value) -> Result<(), BridgeError> {
    serde_json::to_writer(&mut *stdin, value)
        .map_err(|_| BridgeError::Adapter("Could not encode Codex app-server request".into()))?;
    stdin.write_all(b"\n")?;
    stdin.flush()?;
    Ok(())
}

fn receive_app_server_response(
    receiver: &Receiver<Result<Value, String>>,
    id: i64,
    deadline: Instant,
) -> Result<Value, BridgeError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(BridgeError::Adapter(
                "Codex app-server timed out while syncing the plugin".into(),
            ));
        }
        match receiver.recv_timeout(remaining) {
            Ok(Ok(frame)) if frame.get("id").and_then(Value::as_i64) == Some(id) => {
                return Ok(frame)
            }
            Ok(Ok(_)) => {}
            Ok(Err(message)) => return Err(BridgeError::Adapter(message)),
            Err(_) => {
                return Err(BridgeError::Adapter(
                    "Codex app-server timed out while syncing the plugin".into(),
                ))
            }
        }
    }
}

fn checked_app_server_result(response: Value) -> Result<Value, BridgeError> {
    if let Some(error) = response.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(sanitize_error)
            .unwrap_or_else(|| "Codex app-server request failed".into());
        return Err(BridgeError::Adapter(message));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| BridgeError::Adapter("Codex app-server returned no result".into()))
}

fn stop_app_server(child: &mut Child) {
    let _ = crate::adapters::terminate_process_group(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

fn open_authorization_urls(urls: &[String]) -> Result<(), BridgeError> {
    #[cfg(target_os = "macos")]
    {
        for url in urls {
            if !Command::new("open").arg(url).status()?.success() {
                return Err(BridgeError::Adapter(
                    "Codex installed the plugin, but Bridge could not open its authorization page"
                        .into(),
                ));
            }
        }
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    Err(BridgeError::Adapter(
        "Codex installed the plugin, but automatic authorization-page opening is not supported on this platform".into(),
    ))
}

pub fn sanitize_error(value: &str) -> String {
    let mut redact_following = 0;
    value
        .split_whitespace()
        .map(|part| {
            let lower = part.to_lowercase();
            if lower.starts_with("https://") || lower.starts_with("http://") {
                return "[REDACTED_URL]";
            }
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
        assert!(action_args(
            MarketplaceProvider::Codex,
            MarketplaceAction::Install,
            "vercel",
            Some("official"),
            None,
        )
        .is_err());
        assert_eq!(
            action_args(
                MarketplaceProvider::Claude,
                MarketplaceAction::Authenticate,
                "plugin:vercel:vercel",
                None,
                None,
            )
            .unwrap(),
            vec!["mcp", "login", "plugin:vercel:vercel"]
        );
        assert!(action_args(
            MarketplaceProvider::Claude,
            MarketplaceAction::Authenticate,
            "vercel@official",
            None,
            None,
        )
        .is_err());
        assert_eq!(
            action_args(
                MarketplaceProvider::Claude,
                MarketplaceAction::Install,
                "vercel@official",
                Some("official"),
                None,
            )
            .unwrap(),
            vec!["plugin", "install", "vercel@official"]
        );
        assert!(action_args(
            MarketplaceProvider::Codex,
            MarketplaceAction::Disable,
            "vercel@official",
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn codex_authentication_routes_only_named_mcp_servers_to_mcp_login() {
        assert_eq!(
            action_args(
                MarketplaceProvider::Codex,
                MarketplaceAction::Authenticate,
                "notion@official",
                None,
                Some(MCP_CONNECTOR),
            )
            .unwrap(),
            vec!["mcp", "login", "notion"]
        );
        assert!(action_args(
            MarketplaceProvider::Codex,
            MarketplaceAction::Authenticate,
            "vercel@openai-curated",
            None,
            Some(CODEX_APP_CONNECTOR),
        )
        .is_err());
        assert!(action_args(
            MarketplaceProvider::Codex,
            MarketplaceAction::Authenticate,
            "unknown@official",
            None,
            None,
        )
        .is_err());
    }

    #[test]
    fn detects_codex_app_manifest_without_reading_credentials() {
        let directory = std::env::temp_dir().join(format!(
            "bridge-marketplace-app-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join(".app.json"),
            r#"{"apps":{"vercel":{"id":"connector_example"}}}"#,
        )
        .unwrap();
        let source = json!([{
            "pluginId": "vercel@openai-curated",
            "name": "vercel",
            "source": {"source": "local", "path": directory.to_string_lossy()},
            "installed": true
        }]);

        let variant = parse_variants(MarketplaceProvider::Codex, &source).remove(0);

        assert_eq!(variant.connector_type.as_deref(), Some(CODEX_APP_CONNECTOR));
        assert_eq!(variant.app_connector_ids, vec!["connector_example"]);
        assert!(!variant.portable_mcp);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_only_explicit_app_accessibility_states() {
        let states = parse_app_auth_states(&json!({"data": [
            {"id": "connected", "isAccessible": true},
            {"id": "required", "isAccessible": false, "installUrl": "https://example.test/login?secret=state"},
            {"id": "unknown", "isAccessible": false},
            {"id": "missing"}
        ]}));

        assert_eq!(
            states,
            vec![
                MarketplaceAppAuthState {
                    provider: MarketplaceProvider::Codex,
                    connector_id: "connected".into(),
                    display_name: None,
                    native_connector: false,
                    authentication_state: "connected".into()
                },
                MarketplaceAppAuthState {
                    provider: MarketplaceProvider::Codex,
                    connector_id: "required".into(),
                    display_name: None,
                    native_connector: false,
                    authentication_state: "required".into()
                },
            ]
        );
    }

    #[test]
    fn parses_claude_plugin_connectors_with_canonical_namespaced_ids() {
        let source = json!([{
            "id": "vercel@claude-plugins-official",
            "enabled": true,
            "installPath": "/tmp/vercel/0.44.0",
            "mcpServers": {"vercel": {"type": "http", "url": "https://mcp.vercel.com"}}
        }]);

        let variant = parse_variants(MarketplaceProvider::Claude, &source).remove(0);

        assert_eq!(variant.connector_type.as_deref(), Some("connector"));
        assert_eq!(variant.app_connector_ids, vec!["plugin:vercel:vercel"]);
        assert_eq!(
            variant.mcp_endpoint.as_deref(),
            Some("https://mcp.vercel.com")
        );
        assert!(variant.installed);
        assert!(!variant.portable_mcp);
    }

    #[test]
    fn parses_only_explicit_claude_remote_connector_health() {
        let states = parse_claude_mcp_auth_states(
            "Checking MCP server health…\n\
             claude.ai Notion: https://mcp.example/notion - ✔ Connected\n\
             plugin:vercel:vercel: https://mcp.example/vercel (HTTP) - ! Needs authentication\n\
             local: /tmp/local-server - ✔ Connected\n\
             malformed remote status\n",
        );

        assert_eq!(states.len(), 2);
        assert_eq!(states[0].connector_id, "claude.ai Notion");
        assert_eq!(states[0].display_name.as_deref(), Some("Notion"));
        assert!(states[0].native_connector);
        assert_eq!(states[0].authentication_state, "connected");
        assert_eq!(states[1].connector_id, "plugin:vercel:vercel");
        assert!(!states[1].native_connector);
        assert_eq!(states[1].authentication_state, "required");
    }

    #[test]
    fn extracts_only_native_claude_connector_endpoints_for_sdk_configuration() {
        let configs = parse_claude_native_connector_configs(
            "claude.ai Notion: https://mcp.example/notion - ✔ Connected\n\
             plugin:vercel:vercel: https://mcp.example/vercel - ! Needs authentication\n\
             claude.ai Missing endpoint - ! Needs authentication\n",
        );

        assert_eq!(configs.len(), 1);
        assert_eq!(configs["claude.ai Notion"]["type"], "http");
        assert_eq!(
            configs["claude.ai Notion"]["url"],
            "https://mcp.example/notion"
        );
    }

    #[test]
    #[ignore = "requires an installed Claude Code plugin with a remote MCP connector"]
    fn live_claude_plugin_and_connector_state_are_discoverable() {
        let binary = binary::resolve("claude").expect("Claude CLI must be installed");
        let catalog_output = bounded_output(
            &binary,
            &["plugin", "list", "--available", "--json"],
            CLAUDE_MCP_STATUS_TIMEOUT,
        )
        .expect("Claude plugin catalog must respond");
        assert!(catalog_output.status.success());
        let value: Value = serde_json::from_slice(&catalog_output.stdout).unwrap();
        let variants = parse_variants(MarketplaceProvider::Claude, &value);
        let vercel = variants
            .iter()
            .find(|variant| variant.plugin_id == "vercel@claude-plugins-official")
            .expect("Vercel plugin must be listed");
        assert!(vercel.installed);
        assert!(vercel
            .app_connector_ids
            .contains(&"plugin:vercel:vercel".into()));

        let status_output = bounded_output(&binary, &["mcp", "list"], CLAUDE_MCP_STATUS_TIMEOUT)
            .expect("Claude MCP status must respond");
        assert!(status_output.status.success());
        let states = parse_claude_mcp_auth_states(&String::from_utf8_lossy(&status_output.stdout));
        assert!(states
            .iter()
            .any(|state| state.connector_id == "plugin:vercel:vercel"));
        assert!(states.iter().any(|state| state.native_connector));

        let sdk_configuration = claude_sdk_configuration();
        assert!(sdk_configuration
            .plugins
            .iter()
            .any(|path| path.contains("/vercel/")));
        assert!(sdk_configuration
            .mcp_servers
            .keys()
            .any(|name| name.starts_with("claude.ai ")));
    }

    #[test]
    fn sanitization_never_surfaces_authorization_urls() {
        assert_eq!(
            sanitize_error("open https://chatgpt.com/apps/demo?state=sensitive now"),
            "open [REDACTED_URL] now"
        );
    }

    #[test]
    #[ignore = "requires signed-in Codex app services and opens the provider consent page"]
    fn live_codex_app_install_opens_provider_authorization() {
        let binary = binary::resolve("codex").expect("Codex CLI must be installed");
        let result = codex_plugin_install(
            &binary,
            "vercel@openai-curated",
            Some("openai-curated"),
            MarketplaceAction::Install,
        )
        .expect("Codex app-server install should succeed");

        assert!(result.success);
        assert!(result.message.contains("finish authorization"));
        assert!(!result.message.contains("https://"));
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
