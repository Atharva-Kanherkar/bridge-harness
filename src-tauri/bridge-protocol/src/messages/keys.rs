//! The API keys domain: keys an agent can use but never read. A saved key is
//! kept for every chat (its value in the macOS Keychain); a chat key was
//! pasted into one chat and lasts until Bridge quits. No result ever carries a
//! key's value.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A key saved for every chat. The agent writes `[secret:<name>]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedApiKey {
    pub name: String,
    /// The detector that recognized the value (`openai`, `github`, ...), or
    /// `custom`.
    pub kind: String,
    /// Sites the key may be sent to. Empty means none yet.
    pub hosts: Vec<String>,
    /// Bridge cannot read the value back. The Keychain ties a saved key to
    /// the build that wrote it unless the build is signed with a team id, so
    /// a local rebuild needs the key pasted again.
    #[serde(default)]
    pub needs_value: bool,
}

/// A key pasted into one chat. The agent writes `[secret:<reference>]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChatApiKey {
    pub reference: String,
    pub kind: String,
    pub hosts: Vec<String>,
    pub session_id: String,
    /// The chat's title, when it still exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_label: Option<String>,
    /// The saved key's name, once the person saved this one for every chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_as: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeysSnapshot {
    /// False off macOS: keys can still be pasted into a chat, not saved.
    pub can_save: bool,
    /// False when Bridge could not bind the loopback port agents call.
    pub proxy_running: bool,
    pub saved: Vec<SavedApiKey>,
    pub chat: Vec<ChatApiKey>,
}

/// Save a key for every chat, or change the sites of one already saved.
/// Exactly one source: `value` (a new key), `fromReference` (a key pasted into
/// a chat), or neither (keep the saved value, change only the sites).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveApiKeyParams {
    pub name: String,
    pub hosts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_reference: Option<String>,
}

/// Forget a saved key by name, or a chat key by its `sec_…` reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForgetApiKeyParams {
    pub reference: String,
}
