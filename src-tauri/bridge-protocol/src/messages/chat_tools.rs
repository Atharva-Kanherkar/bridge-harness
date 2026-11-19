//! The tools of one chat: the connectors and skills that a chat leaves out.
//! A choice only removes items. It never grants a permission.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a person keeps for one chat. Servers and Codex skills use an off
/// list, so a new item is on. Claude skills use an allowlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatToolSelection {
    /// The harness of the choice.
    pub harness: String,
    /// The extra account of the choice. Absent means the own login of the
    /// user.
    #[serde(default)]
    pub account: Option<String>,
    /// MCP servers to leave out, by the exact name that the harness uses.
    #[serde(default)]
    pub servers_off: Vec<String>,
    /// Codex only: leave out `codex_apps`, the bundle of every app.
    #[serde(default)]
    pub codex_apps_off: bool,
    /// Claude only. Absent or null keeps each skill. A list keeps only the
    /// named skills, and an empty list keeps none.
    #[serde(default)]
    pub claude_skills: Option<ClaudeSkillChoice>,
    /// Codex only: skills to leave out, by their `skills/list` name.
    #[serde(default)]
    pub codex_skills_off: Vec<String>,
}

/// The Claude skills that a chat keeps. Names are as the init message gives
/// them: a bare name, or `plugin:skill` for a plugin skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClaudeSkillChoice {
    pub only: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChatToolKind {
    /// One MCP server.
    Server,
    /// The Codex `codex_apps` switch, which turns every app on or off.
    Apps,
    /// One app inside `codex_apps`.
    App,
    /// One skill.
    Skill,
    /// Every skill of a harness, as one item.
    AllSkills,
    /// One plugin.
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChatToolState {
    On,
    Off,
    /// No verified lever exists, or Bridge owns the item. `fixedReason` says
    /// which.
    Fixed,
}

/// A token figure and the source that measured or estimated it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolTokens {
    pub value: u64,
    pub source: String,
    pub at: Option<String>,
}

/// One row of the tools of a chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolItem {
    pub kind: ChatToolKind,
    /// The exact identity that the choice stores.
    pub id: String,
    pub label: String,
    pub group: String,
    pub state: ChatToolState,
    pub fixed_reason: Option<String>,
    /// False for a saved name that the current source does not list.
    pub seen: bool,
    pub tokens: Option<ChatToolTokens>,
}

/// The stored choice of a chat, and whether its next launch applies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolSelectionStatus {
    pub revision: u32,
    pub harness: String,
    pub account: Option<String>,
    pub applies: bool,
    pub not_applied_reason: Option<String>,
}

/// A chat that does not exist yet: the composer of a new chat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChatToolsDraft {
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub harness: String,
    #[serde(default)]
    pub account: Option<String>,
}

/// Exactly one of `sessionId` and `draft`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetChatToolsParams {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub draft: Option<ChatToolsDraft>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ChatToolSourceKind {
    /// The connector catalog that Bridge already cached.
    Catalog,
    /// The init message of a Claude chat.
    ClaudeInit,
    /// A context reading that measured the tokens of each item.
    ContextReading,
    /// The warm runtime of the chat.
    Runtime,
}

/// One source that the list read, and when. A source that Bridge could not
/// read has `available` false and a `note` that says why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatToolSource {
    pub kind: ChatToolSourceKind,
    pub label: String,
    pub at: Option<String>,
    pub available: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetChatToolsResult {
    pub harness: String,
    pub account: Option<String>,
    pub items: Vec<ChatToolItem>,
    /// The saved choice. Absent when the chat has none or the row does not
    /// parse.
    pub choice: Option<ChatToolSelection>,
    /// Revision 0 means that the chat has no saved choice yet.
    pub selection: ChatToolSelectionStatus,
    pub sources: Vec<ChatToolSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateChatToolsParams {
    pub session_id: String,
    /// 0 for the first choice of a chat.
    pub expected_revision: u32,
    pub selection: ChatToolSelection,
}

/// Exactly one field is present. A refusal leaves the saved choice as it
/// was. A stale revision or a choice that is not valid is an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatToolsResult {
    pub revision: Option<u32>,
    pub refusal: Option<String>,
}
