//! What a chat does when its provider says the account is out of quota:
//! nothing, wait for the reset and continue on the same model, or switch to a
//! backup model and continue now. One record for the account; the chat banner
//! reads one row per chat.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Effort;

/// The user's choice for a chat stopped by a usage limit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitRecoveryMode {
    /// Today's behavior: the chat stops and waits for the person.
    #[default]
    Off,
    /// Wait for the provider's stated reset, then continue on the same
    /// harness, model and effort.
    Resume,
    /// Switch to the backup model now and continue. A backup that also runs
    /// out waits for its own reset.
    Backup,
}

/// The model a chat switches to under [`LimitRecoveryMode::Backup`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LimitRecoveryTarget {
    /// Harness id (`claude` | `codex` | `opencode` | ...).
    pub harness: String,
    /// `None` is the harness default model.
    pub model: Option<String>,
    /// `None` keeps the model's default effort.
    pub effort: Option<Effort>,
}

/// The account-wide setting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct LimitRecoverySettings {
    pub mode: LimitRecoveryMode,
    /// Required when `mode` is `backup`; kept when another mode is chosen so
    /// switching back does not lose the choice.
    pub backup: Option<LimitRecoveryTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveLimitRecoverySettingsParams {
    pub settings: LimitRecoverySettings,
}

/// Where one chat's recovery stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitRecoveryState {
    /// Waiting for `dueAt`.
    Pending,
    /// Bridge is switching or sending the continue turn right now.
    Acting,
    /// The continue turn was sent.
    Done,
    /// The person cancelled it, sent a message themselves, or changed the
    /// chat's model.
    Cancelled,
    /// The limit kept coming back and Bridge stopped trying.
    GaveUp,
    /// Bridge was closed past the due time by more than the catch-up window.
    Expired,
    /// The switch or the send failed; `outcome` says why.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LimitRecoveryView {
    pub session_id: String,
    pub mode: LimitRecoveryMode,
    pub state: LimitRecoveryState,
    /// The provider's phrase that started it ("usage limit").
    pub signal: String,
    /// The reset the provider stated, RFC 3339, when it stated one.
    pub reset_at: Option<String>,
    /// When Bridge will act, RFC 3339.
    pub due_at: String,
    /// What the chat continues on.
    pub target: LimitRecoveryTarget,
    /// Whether acting switches the chat's model first.
    pub switches_model: bool,
    /// Limits in a row this chat hit while recovering.
    pub attempts: u32,
    /// Why it ended, for a finished row.
    pub outcome: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LimitRecoveryStatusParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelLimitRecoveryParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResumeLimitRecoveryParams {
    pub session_id: String,
}

/// A chat's latest recovery row, or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LimitRecoveryStatus {
    pub recovery: Option<LimitRecoveryView>,
}
