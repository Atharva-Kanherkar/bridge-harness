//! The approvals domain: the human decision on a paused agent turn.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What the operator decided about a pending approval. The command rejects
/// anything outside this set, so the contract names it: a host validating
/// params from the registry turns a typo into `invalid_params` instead of an
/// application error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum QuestionAction {
    Answer,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum InteractionResolutionDisposition {
    Resolved,
    AlreadyResolved,
}

/// Durable result shared by permission decisions and question replies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InteractionResolutionResult {
    pub disposition: InteractionResolutionDisposition,
    pub interaction_kind: String,
    pub status: String,
    pub resolved_by: String,
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveApprovalParams {
    pub session_id: String,
    /// The durable sequence of the `approval.requested` event being answered.
    pub event_id: i64,
    pub decision: ApprovalDecision,
    /// Exact provider option id when the permission protocol advertises one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveQuestionParams {
    pub session_id: String,
    /// The durable sequence of the `question.requested` event.
    pub event_id: i64,
    pub action: QuestionAction,
    /// Question id to one or more exact selected/free-form values. Form
    /// elicitations use property names as ids.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub answers: std::collections::BTreeMap<String, Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn resolve_approval_round_trips_with_camel_case_decisions() {
        let resolve = ResolveApprovalParams {
            session_id: "s-1".into(),
            event_id: 42,
            decision: ApprovalDecision::AcceptForSession,
            option_id: Some("allow-always".into()),
        };
        assert_eq!(
            serde_json::to_value(&resolve).unwrap(),
            json!({"sessionId": "s-1", "eventId": 42, "decision": "acceptForSession", "optionId": "allow-always"})
        );
        assert_eq!(round_trip(&resolve), resolve);
    }

    #[test]
    fn question_resolution_keeps_answers_typed_and_separate() {
        let resolve = ResolveQuestionParams {
            session_id: "s-1".into(),
            event_id: 7,
            action: QuestionAction::Answer,
            answers: std::collections::BTreeMap::from([(
                "target".into(),
                vec!["Core".into()],
            )]),
        };
        assert_eq!(
            serde_json::to_value(&resolve).unwrap(),
            json!({"sessionId":"s-1","eventId":7,"action":"answer","answers":{"target":["Core"]}})
        );
        assert_eq!(round_trip(&resolve), resolve);
    }

    #[test]
    fn resolve_approval_rejects_incomplete_and_unknown_decisions() {
        assert!(serde_json::from_value::<ResolveApprovalParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<ResolveApprovalParams>(
                json!({"sessionId": "s", "eventId": 1})
            )
            .is_err(),
            "decision is required"
        );
        assert!(
            serde_json::from_value::<ResolveApprovalParams>(
                json!({"sessionId": "s", "event_id": 1, "decision": "accept"})
            )
            .is_err(),
            "wire names are camelCase"
        );
        assert!(
            serde_json::from_value::<ResolveApprovalParams>(
                json!({"sessionId": "s", "eventId": 1, "decision": "accept_for_session"})
            )
            .is_err(),
            "decisions are camelCase, not snake_case"
        );
        assert!(
            serde_json::from_value::<ResolveApprovalParams>(
                json!({"sessionId": "s", "eventId": 1, "decision": "escalate"})
            )
            .is_err(),
            "unknown decisions must be rejected"
        );
    }
}
