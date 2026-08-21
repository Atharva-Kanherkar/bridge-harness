//! The configuration domain: harness defaults, agent definitions, and the
//! OpenCode provider catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::common::Effort;

/// Per-harness defaults. Mirrors `bridge_core::agent_config::HarnessConfig`,
/// including its refusal of unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessConfig {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default)]
    pub system_prompt: String,
    /// Harness-specific settings passed through untouched.
    #[serde(default = "empty_object")]
    pub advanced: Value,
    /// Set when this config overrides a built-in default.
    #[serde(default)]
    pub is_override: bool,
}

/// A configured agent. Mirrors `bridge_core::agent_config::AgentDefinition`,
/// including its refusal of unknown fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentDefinition {
    /// Empty when creating; the server assigns the id.
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub role: String,
    pub harness: String,
    #[serde(default)]
    pub model: Option<String>,
    pub effort: Effort,
    #[serde(default)]
    pub system_prompt: String,
    pub enabled: bool,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub is_built_in: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveHarnessConfigParams {
    pub config: HarnessConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetHarnessConfigParams {
    /// The harness id to restore to its built-in defaults.
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RefreshOpencodeCatalogParams {
    /// Working directory whose OpenCode configuration to read; omitted uses
    /// the user-level configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetOpencodeProviderApiKeyParams {
    pub provider_id: String,
    /// Stored by OpenCode's own auth file, never persisted by Bridge.
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveOpencodeProviderAuthParams {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveAgentConfigParams {
    pub agent: AgentDefinition,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteAgentConfigParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetDefaultAgentParams {
    pub id: String,
}

/// How much Bridge asks before an agent acts. Mirrors
/// `bridge_core::agent_config::PermissionPolicy`.
///
/// `default` on the container, not just the fields: a policy payload written by
/// an older build must still read once slice 3 adds the graduated modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct PermissionPolicy {
    /// Auto-accept every provider approval, for every agent. Worker write scope
    /// and browser outward effects are unaffected — those are authorization.
    pub bypass_all: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SavePermissionPolicyParams {
    pub policy: PermissionPolicy,
}

/// The configuration snapshot every config mutation returns. Mirrors
/// `bridge_core::agent_config::ConfigState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfigState {
    pub harnesses: Vec<HarnessConfig>,
    pub agents: Vec<AgentDefinition>,
    pub default_agent_id: String,
    pub permission_policy: PermissionPolicy,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn harness_config_round_trips_and_defaults_its_optional_fields() {
        let save = SaveHarnessConfigParams {
            config: HarnessConfig {
                id: "codex".into(),
                label: "Codex".into(),
                enabled: true,
                default_model: Some("gpt-5".into()),
                effort: Some(Effort::Xhigh),
                system_prompt: String::new(),
                advanced: json!({"sandbox": "workspace-write"}),
                is_override: true,
            },
        };
        let wire = serde_json::to_value(&save).unwrap();
        assert_eq!(wire["config"]["defaultModel"], json!("gpt-5"));
        assert_eq!(wire["config"]["effort"], json!("xhigh"));
        assert_eq!(wire["config"]["isOverride"], json!(true));
        assert_eq!(wire["config"]["advanced"]["sandbox"], json!("workspace-write"));
        assert_eq!(round_trip(&save), save);

        let minimal: HarnessConfig =
            serde_json::from_value(json!({"id": "shell", "label": "Shell", "enabled": false}))
                .unwrap();
        assert_eq!(minimal.default_model, None);
        assert_eq!(minimal.effort, None);
        assert_eq!(minimal.advanced, json!({}), "advanced defaults to an empty object");
        assert!(!minimal.is_override);
    }

    #[test]
    fn agent_definitions_round_trip() {
        let save = SaveAgentConfigParams {
            agent: AgentDefinition {
                id: String::new(),
                name: "Reviewer".into(),
                description: "Reviews diffs".into(),
                role: "review".into(),
                harness: "claude".into(),
                model: None,
                effort: Effort::Medium,
                system_prompt: "Be exacting.".into(),
                enabled: true,
                is_default: false,
                is_built_in: false,
                created_at: String::new(),
                updated_at: String::new(),
            },
        };
        let wire = serde_json::to_value(&save).unwrap();
        assert_eq!(wire["agent"]["systemPrompt"], json!("Be exacting."));
        assert_eq!(wire["agent"]["isBuiltIn"], json!(false));
        assert_eq!(round_trip(&save), save);

        let created: AgentDefinition = serde_json::from_value(json!({
            "name": "Planner", "role": "plan", "harness": "codex", "effort": "low",
            "enabled": true,
        }))
        .unwrap();
        assert!(created.id.is_empty(), "the server assigns ids");
        assert!(created.system_prompt.is_empty());
    }

    #[test]
    fn opencode_and_id_params_round_trip() {
        let key = SetOpencodeProviderApiKeyParams {
            provider_id: "openrouter".into(),
            api_key: "sk-test".into(),
            directory: None,
        };
        assert_eq!(
            serde_json::to_value(&key).unwrap(),
            json!({"providerId": "openrouter", "apiKey": "sk-test"}),
            "absent options stay off the wire"
        );
        assert_eq!(round_trip(&key), key);

        let remove = RemoveOpencodeProviderAuthParams {
            provider_id: "openrouter".into(),
            directory: Some("/repos/demo".into()),
        };
        assert_eq!(
            serde_json::to_value(&remove).unwrap(),
            json!({"providerId": "openrouter", "directory": "/repos/demo"})
        );
        assert_eq!(round_trip(&remove), remove);

        let refresh = RefreshOpencodeCatalogParams { directory: None };
        assert_eq!(serde_json::to_value(&refresh).unwrap(), json!({}));
        assert_eq!(round_trip(&refresh), refresh);

        for wire in [
            serde_json::to_value(ResetHarnessConfigParams { id: "codex".into() }).unwrap(),
            serde_json::to_value(DeleteAgentConfigParams { id: "codex".into() }).unwrap(),
            serde_json::to_value(SetDefaultAgentParams { id: "codex".into() }).unwrap(),
        ] {
            assert_eq!(wire, json!({"id": "codex"}));
        }
    }

    #[test]
    fn config_params_reject_incomplete_and_misspelled_payloads() {
        assert!(serde_json::from_value::<SaveHarnessConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SaveAgentConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<ResetHarnessConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<DeleteAgentConfigParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SetDefaultAgentParams>(json!({})).is_err());
        assert!(serde_json::from_value::<SetOpencodeProviderApiKeyParams>(
            json!({"providerId": "openrouter"})
        )
        .is_err());
        assert!(
            serde_json::from_value::<SetOpencodeProviderApiKeyParams>(
                json!({"provider_id": "openrouter", "api_key": "sk"})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<HarnessConfig>(json!({
                "id": "codex", "label": "Codex", "enabled": true, "defaultEffort": "high",
            }))
            .is_err(),
            "a harness config refuses fields it does not define"
        );
        assert!(
            serde_json::from_value::<AgentDefinition>(json!({
                "name": "Planner", "role": "plan", "harness": "codex", "effort": "extreme",
                "enabled": true,
            }))
            .is_err(),
            "unknown effort levels must be rejected"
        );
        assert!(
            serde_json::from_value::<RefreshOpencodeCatalogParams>(json!({"cwd": "/x"})).is_err(),
            "params reject arguments the contract does not name"
        );
    }
}
