//! Personal defaults are preferences, never imported execution authority.
use super::{ActiveTurnInput, BridgeState, ChatToolSelection};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChatLaunchTarget {
    Direct,
    Orchestrator,
    Agent {
        #[serde(rename = "agentId")]
        agent_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatAccess {
    Ask,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatModelChoice {
    pub harness: String,
    pub model: Option<String>,
    /// The extra Claude or Codex account the chat signs in with, by its id in
    /// the harness's `advanced.accounts`. Absent is the user's own login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

/// Absent inherits; an explicit object containing null uses provider defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatEffortChoice {
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ChatDefaults {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ChatLaunchTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ChatModelChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ChatEffortChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<ChatAccess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_worktree: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_turn_input: Option<ActiveTurnInput>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatDefaultsRecord {
    pub revision: u32,
    pub defaults: ChatDefaults,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatDefaultsView {
    pub application: ChatDefaultsRecord,
    pub project: Option<ChatDefaultsRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatDefaultsRevision {
    pub application: u32,
    pub project: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedChatSettings {
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
    pub revision: ChatDefaultsRevision,
    pub target: ChatLaunchTarget,
    pub model: ChatModelChoice,
    pub effort: Option<String>,
    pub access: ChatAccess,
    pub create_worktree: bool,
    pub active_turn_input: ActiveTurnInput,
    pub sources: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetChatDefaultsParams {
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveChatDefaultsParams {
    pub project_id: Option<String>,
    pub expected_revision: u32,
    pub defaults: ChatDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveChatSettingsParams {
    pub workspace_id: Option<String>,
    pub overrides: ChatDefaults,
    pub expected_revision: Option<ChatDefaultsRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateConfiguredChatParams {
    pub workspace_id: Option<String>,
    pub overrides: ChatDefaults,
    pub expected_revision: ChatDefaultsRevision,
    pub title: Option<String>,
    /// The first tool choice of the new chat. Absent keeps every tool.
    #[serde(default)]
    pub tool_selection: Option<ChatToolSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatSessionSettings {
    pub revision: u32,
    pub settings: ResolvedChatSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredChatResult {
    pub session_id: String,
    pub state: BridgeState,
    pub configuration: ChatSessionSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetChatSettingsParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateChatOptionsParams {
    pub session_id: String,
    pub expected_revision: u32,
    pub access: ChatAccess,
    pub active_turn_input: ActiveTurnInput,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn create_configured_chat_params_accept_an_optional_tool_selection() {
        let base = json!({"workspaceId": null, "overrides": {}, "expectedRevision": {"application": 0, "project": 0}, "title": null});
        let without: CreateConfiguredChatParams = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(without.tool_selection, None, "a 1.45 client never sends the field");
        let mut with = base;
        with["toolSelection"] = json!({"harness": "codex", "serversOff": ["github"], "codexAppsOff": true});
        let parsed: CreateConfiguredChatParams = serde_json::from_value(with.clone()).unwrap();
        let choice = parsed.tool_selection.unwrap();
        assert_eq!(choice.harness, "codex");
        assert_eq!(choice.servers_off, vec!["github".to_string()]);
        assert!(choice.codex_apps_off);
        with["toolSelection"]["grant"] = json!(true);
        assert!(serde_json::from_value::<CreateConfiguredChatParams>(with).is_err(), "an unknown key in the choice is refused");
    }
}
