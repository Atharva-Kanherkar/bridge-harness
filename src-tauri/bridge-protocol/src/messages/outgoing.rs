//! Outgoing: messages a chat drafts for a person or a place.
//!
//! An agent drafts one through the `outgoing_draft` tool on Bridge's own MCP
//! server; the person edits, fixes, copies or discards it from a card in the
//! transcript. Nothing here sends anything: a later slice adds Send behind an
//! exact-text approval.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The longest body a draft may carry, in characters.
pub const MAX_OUTGOING_BODY_CHARS: usize = 8000;

/// How a destination shows text: Slack mrkdwn, GitHub Markdown, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OutgoingFormat {
    Slack,
    Github,
    Plain,
}

/// Where a draft is addressed. Names are kept as the agent wrote them
/// (`#dev`, `@nina`); resolving them to ids belongs to the slice that sends.
///
/// `rename_all` renames the variant tags only, so multi-word fields carry an
/// explicit rename to stay camelCase on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OutgoingDestination {
    SlackChannel { channel: String },
    SlackThread { channel: String, thread: String },
    SlackDm { user: String },
    GithubComment { repo: String, number: u64 },
    GithubReviewReply {
        repo: String,
        number: u64,
        #[serde(rename = "commentId")]
        comment_id: u64,
    },
    Clipboard { format: OutgoingFormat },
}

/// Where a draft is in its life. `copied` returns to `draft` on an edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OutgoingState {
    Draft,
    Copied,
    Discarded,
}

/// One style-rule finding. `fixed` means Bridge already changed the text;
/// a fixable finding that is not fixed is what the card's Fix button applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingLintFinding {
    /// `no_em_dash`, `no_ai_attribution`, `slack_table`, `slack_heading`, `max_lines`.
    pub rule: String,
    pub message: String,
    /// 1-based line in the body, when the finding has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub fixable: bool,
    pub fixed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    pub id: String,
    pub session_id: String,
    pub destination: OutgoingDestination,
    /// The chip text: `Slack · #dev · thread`, `GitHub · owner/repo#123`.
    pub label: String,
    pub format: OutgoingFormat,
    /// The text as written, in Markdown.
    pub body: String,
    /// The body as the destination shows it.
    pub rendered: String,
    pub lint: Vec<OutgoingLintFinding>,
    pub state: OutgoingState,
    /// Bumped on every change; edits and copies name the revision they saw.
    pub revision: u32,
    /// Why the person wants this message, in the agent's words.
    pub reason: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListOutgoingParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListOutgoingResult {
    /// Oldest first, discarded ones included.
    pub items: Vec<OutgoingMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateOutgoingParams {
    pub id: String,
    /// The revision the editor saw. A stale one is refused.
    pub revision: u32,
    /// The new body; omitted keeps the current one (for a Fix alone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Apply every fixable style rule to the body.
    #[serde(default)]
    pub apply_fixes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutgoingResult {
    pub item: OutgoingMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CopyOutgoingParams {
    pub id: String,
    pub revision: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CopyOutgoingResult {
    /// The rendered text, for the client to put on the clipboard.
    pub text: String,
    pub item: OutgoingMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DiscardOutgoingParams {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DiscardOutgoingResult {
    pub item: OutgoingMessage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_are_camel_case_on_the_wire() {
        let reply = OutgoingDestination::GithubReviewReply { repo: "o/r".into(), number: 3, comment_id: 9 };
        assert_eq!(
            serde_json::to_value(&reply).unwrap(),
            serde_json::json!({"kind": "githubReviewReply", "repo": "o/r", "number": 3, "commentId": 9})
        );
        let clip = OutgoingDestination::Clipboard { format: OutgoingFormat::Slack };
        assert_eq!(serde_json::to_value(&clip).unwrap(), serde_json::json!({"kind": "clipboard", "format": "slack"}));
    }

    #[test]
    fn update_params_refuse_unknown_fields() {
        let error = serde_json::from_value::<UpdateOutgoingParams>(serde_json::json!({"id": "a", "revision": 1, "state": "sent"}));
        assert!(error.is_err());
        let ok: UpdateOutgoingParams = serde_json::from_value(serde_json::json!({"id": "a", "revision": 1})).unwrap();
        assert!(!ok.apply_fixes && ok.body.is_none());
    }
}
