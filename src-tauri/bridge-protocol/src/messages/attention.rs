//! The attention ledger: what a person should know about, decided in core.
//! Clients read it with `attention/list_attention` and learn that it moved
//! from the `attention/changed` refetch hint, which carries no rows.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AttentionKind {
    /// A top-level chat finished a turn.
    Finished,
    /// A top-level chat's turn ended in an error.
    Failed,
    /// A visible chat, workers included, is waiting on the person.
    NeedsYou,
    /// A kind added after this client was built. The ledger gains kinds slice
    /// by slice; decoding them as `other` keeps an older client's read working
    /// instead of failing it, and a client shows nothing for them.
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AttentionSeverity {
    Informational,
    Actionable,
    /// A severity added after this client was built.
    #[serde(other)]
    Other,
}

/// One ledger row. `title` and `body` are written by core so every client
/// says the same thing; a client that knows the chat may render its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    /// The ledger cursor. Strictly increasing, never reused.
    pub sequence: i64,
    pub id: String,
    pub kind: AttentionKind,
    pub severity: AttentionSeverity,
    pub session_id: Option<String>,
    /// For `needsYou`: the session-forest sequence of the request, which is
    /// what `approvals/resolve_approval` takes as its event id.
    pub approval_event_id: Option<i64>,
    pub title: String,
    pub body: Option<String>,
    pub created_at: String,
    /// Set when the request was answered, by the person or by policy.
    pub resolved_at: Option<String>,
    pub seen_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct ListAttentionParams {
    /// Exclusive cursor; absent reads from the start.
    pub after_sequence: Option<i64>,
    /// Default 100, at most 500.
    pub limit: Option<u32>,
    /// Leave out rows the person has already seen.
    pub unseen_only: bool,
}

/// Rows oldest first, without superseded ones. `latestSequence` is the
/// ledger's newest sequence (0 when empty), so a fresh client can start its
/// cursor without paging.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListAttentionResult {
    pub items: Vec<AttentionItem>,
    pub latest_sequence: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarkAttentionSeenParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MarkAttentionSeenResult {
    /// How many rows this call stamped.
    pub updated: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_attention_kinds_decode_as_other() {
        // A row from a newer daemon, with a kind and severity this build has
        // never heard of, must not fail the whole read.
        let page: ListAttentionResult = serde_json::from_value(serde_json::json!({
            "items": [{
                "sequence": 7, "id": "a", "kind": "somethingNew", "severity": "urgent",
                "title": "t", "createdAt": "2026-11-08T00:00:00Z"
            }],
            "latestSequence": 7
        }))
        .unwrap();
        assert_eq!(page.items[0].kind, AttentionKind::Other);
        assert_eq!(page.items[0].severity, AttentionSeverity::Other);
        assert_eq!(serde_json::to_value(AttentionKind::NeedsYou).unwrap(), "needsYou");
    }
}
