//! In-app connector surfaces: what the harness can reach, what arrived, and the
//! approval-gated writes that answer it.
//!
//! Bridge owns no connector credential and speaks no MCP. Every payload here
//! describes something a harness turn produced, so nothing in this module is or
//! could be a token, a cookie, or a session key.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorListParams {
    /// Re-read the harness's MCP health instead of serving the cached verdict.
    #[serde(default)]
    pub refresh: bool,
}

/// Why a family is not on offer. Mirrors `connector_surface::UnavailableReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorUnavailableReason {
    /// Configured, but the harness needs the user to sign in. The one reason
    /// whose fix lives outside Bridge.
    AuthRequired,
    Unreachable,
    NotConfigured,
    /// Bridge cannot derive provenance for this family, so it will not surface it.
    NoResolver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDescriptor {
    pub family: String,
    pub display_name: String,
    /// The MCP server name the harness knows this family by, when one exists.
    pub server: Option<String>,
    pub available: bool,
    pub reason: Option<ConnectorUnavailableReason>,
    /// The sentence the pane shows when unavailable, written to say where the
    /// fix lives — which for most reasons is not inside Bridge.
    pub explanation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorListResult {
    pub connectors: Vec<ConnectorDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorInboxParams {
    /// Most items to return. Clamped host-side.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorItemKind {
    DirectMessage,
    Mention,
    ThreadReply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConnectorItemState {
    /// Announced, no card yet. The toast is already up.
    Pending,
    Rendered,
    Resolved,
}

/// One renderable block. A closed set: the harness picks from these, it never
/// authors markup, so no connector message can become script or layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ConnectorCardBlock {
    Message { author: String, text: String, timestamp: Option<String> },
    Context { text: String },
    Summary { text: String },
    Fact { label: String, value: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorCardPayload {
    pub item_key: String,
    pub headline: String,
    pub blocks: Vec<ConnectorCardBlock>,
    /// Drafts the user edits. Sending one still needs the same approval as any
    /// other reply.
    pub suggested_replies: Vec<String>,
    /// False when Bridge authored this card after refusing the harness's.
    pub harness_rendered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInboxItem {
    pub item_key: String,
    pub family: String,
    pub channel_id: String,
    pub channel_label: String,
    pub author: String,
    pub kind: ConnectorItemKind,
    /// The message body. Untrusted third-party content — display it as text.
    pub text: String,
    pub permalink: Option<String>,
    pub received_at: String,
    pub state: ConnectorItemState,
    pub card: Option<ConnectorCardPayload>,
    /// Why the harness's card was refused, when it was.
    pub render_rejection: Option<String>,
    pub resolution: Option<String>,
}

/// What the last ingress cycle did, per family. Present so the pane can say
/// "could not read" instead of showing an empty inbox that isn't.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorPollStatus {
    pub family: String,
    pub last_attempt_at: Option<String>,
    pub last_success_at: Option<String>,
    pub degraded: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInboxResult {
    pub items: Vec<ConnectorInboxItem>,
    pub unread_count: u32,
    pub poll: Vec<ConnectorPollStatus>,
}

/// The write the user is asking for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ConnectorActionRequest {
    Reply { text: String },
    React { emoji: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorActParams {
    pub item_key: String,
    pub action: ConnectorActionRequest,
    /// The user's decision on the exact effect this call already described.
    ///
    /// Absent means "I have not decided": the host refuses and returns the
    /// effect for a confirmation dialog. Approval is per call and never sticky —
    /// there is no "always allow" here, because the thing being approved is a
    /// specific string going to a specific place.
    #[serde(default)]
    pub approved: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ConnectorActResult {
    /// Nothing ran. `effect` is the literal sentence to show the user.
    ApprovalRequired { effect: String },
    // `rename_all` on the enum renames the *variants*, not their fields, so a
    // multi-word field inside one stays snake_case unless the variant says so.
    Sent {
        #[serde(rename = "itemKey")]
        item_key: String,
    },
    Refused { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorDismissParams {
    pub item_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDismissResult {
    pub dismissed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectorRefreshParams {
    pub family: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorRefreshResult {
    /// How many previously unseen items this cycle announced.
    pub announced: u32,
}
