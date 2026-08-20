//! The sessions domain: the session forest, live turns, and durable replay.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::common::HarnessId;

pub const DEFAULT_REPLAY_EVENT_LIMIT: u32 = 500;
pub const MAX_REPLAY_EVENT_LIMIT: u32 = 1_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GetSessionForestParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetSessionForestDigestParams {
    pub session_id: String,
}

/// `sessions/get_session_forest_digest`'s result: an opaque change token for
/// one session's forest. Equal digests mean the snapshot would be unchanged;
/// clients compare tokens instead of fetching and stringifying complete
/// histories every poll. External state the store cannot see (repository
/// divergence) is not covered — poll a full snapshot at a low cadence for
/// that.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionForestDigestResult {
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ActivateSessionEntryParams {
    pub session_id: String,
    /// The forest entry to become the conversation head; files are not changed.
    pub entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateChatParams {
    pub harness: HarnessId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkspaceSessionParams {
    pub workspace_id: String,
    /// Create the session in an isolated Git worktree (requires a connected
    /// repository).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_worktree: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChatModelParams {
    pub session_id: String,
    pub harness: HarnessId,
    /// Explicit model id; omitted selects the harness's default for the
    /// chat's tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplaySessionEventsParams {
    pub session_id: String,
    /// The last durable sequence the client has seen; events strictly after
    /// this cursor are returned in order, with no gaps and no duplicates.
    #[schemars(range(min = 0))]
    pub after_sequence: i64,
    /// Maximum number of events to return. Omitted requests use 500; the
    /// server rejects values outside 1..=1000.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 1_000))]
    pub limit: Option<u32>,
}

/// Structured provider data accepted by normalized events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StructuredJson {
    Object(BTreeMap<String, Value>),
    Array(Vec<Value>),
}

/// The durable event wire shape returned by session replay. This mirrors the
/// core `AgentEvent` DTO without making the protocol crate depend on core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReplaySessionEvent {
    pub id: i64,
    pub session_id: String,
    pub sequence: i64,
    pub protocol_version: i64,
    pub kind: String,
    pub item_id: Option<String>,
    pub role: Option<String>,
    pub status: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub data: StructuredJson,
    pub provider_meta: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ReplaySessionEventsResult(pub Vec<ReplaySessionEvent>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionParams {
    pub workspace_id: String,
    /// Explicit harness; omitted resolves the configured orchestrator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<HarnessId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StartChatParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PrepareTurnParams {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendTurnParams {
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitInputParams {
    pub session_id: String,
    pub text: String,
}

/// What Bridge did with submitted user input. These three modes are the whole
/// contract: a client that receives anything else is talking to a server it
/// does not understand, so the enum is closed rather than tolerant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum InputDisposition {
    /// Nothing was running; the input started a normal turn.
    StartedNewTurn,
    /// A turn was running and the provider took the input natively.
    SteeredActiveTurn,
    /// A turn was running and the provider cannot take input mid-turn, so the
    /// input is durably queued for delivery at the next phase boundary.
    QueuedForPhaseBoundary,
}

impl InputDisposition {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StartedNewTurn => "startedNewTurn",
            Self::SteeredActiveTurn => "steeredActiveTurn",
            Self::QueuedForPhaseBoundary => "queuedForPhaseBoundary",
        }
    }
}

/// `sessions/submit_input`'s result. The disposition is what the client renders;
/// `queuedInputId` names the durable row so the optimistic message can be
/// reconciled with its delivery, and the interceptions mirror
/// `sessions/prepare_turn` so a steered or queued message reports replaced
/// secrets exactly like a new turn does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubmitInputResult {
    pub disposition: InputDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queued_input_id: Option<String>,
    pub interceptions: Vec<SecretInterception>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StopSessionParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InterruptTurnParams {
    pub session_id: String,
}

/// `sessions/retry_worker_task` — re-dispatch a finished worker's objective
/// because the user asked for it.
///
/// The counterpart to the automatic retry Bridge no longer takes on its own: the
/// worker's failure now reaches the user with its real cause, and this is the
/// action offered alongside it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryWorkerTaskParams {
    /// The worker whose objective should run again.
    pub child_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CompactSessionParams {
    pub session_id: String,
}

pub const DEFAULT_RECALL_HIT_LIMIT: u32 = 20;
pub const MAX_RECALL_HIT_LIMIT: u32 = 50;

/// FTS5 recall over `session_entries`. `sessionId` is required; there is no
/// workspace-wide search on this method.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchSessionEntriesParams {
    pub session_id: String,
    pub query: String,
    /// Omitted requests use 20; the server rejects values outside 1..=50.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 50))]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecallHit {
    pub entry_id: String,
    pub kind: String,
    pub sequence: i64,
    pub snippet: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchSessionEntriesResult {
    pub session_id: String,
    pub query: String,
    pub hits: Vec<SessionRecallHit>,
}

/// Mirrors `bridge_core::secret_interception::SecretInterception` — one
/// secret replaced by a broker reference before the turn left the machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretInterception {
    /// The broker reference substituted into the text; the secret itself
    /// never crosses the wire.
    pub reference: String,
    pub detector: String,
}

/// `sessions/prepare_turn`'s result. Mirrors
/// `bridge_core::secret_interception::SanitizedTurn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedTurn {
    pub text: String,
    pub interceptions: Vec<SecretInterception>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::common::round_trip;
    use serde_json::json;

    #[test]
    fn replay_result_accepts_array_event_data() {
        let result = ReplaySessionEventsResult(vec![ReplaySessionEvent {
            id: 1,
            session_id: "s".into(),
            sequence: 1,
            protocol_version: 1,
            kind: "tool.completed".into(),
            item_id: Some("tool-1".into()),
            role: Some("tool".into()),
            status: Some("completed".into()),
            title: None,
            text: None,
            data: StructuredJson::Array(vec![json!({"line": 1})]),
            provider_meta: json!({"adapter": "codex"}),
            created_at: "now".into(),
        }]);
        assert_eq!(serde_json::to_value(&result).unwrap()[0]["data"][0]["line"], 1);
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn session_params_round_trip_and_omit_absent_options() {
        let create = CreateChatParams { harness: HarnessId::parse("codex").unwrap(), model: None, title: None };
        let wire = serde_json::to_value(&create).unwrap();
        assert_eq!(wire, json!({"harness": "codex"}), "absent options stay off the wire");
        assert_eq!(round_trip(&create), create);

        let update = UpdateChatModelParams {
            session_id: "s-1".into(),
            harness: HarnessId::parse("opencode").unwrap(),
            model: Some("kimi-k2.5".into()),
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(
            wire,
            json!({"sessionId": "s-1", "harness": "opencode", "model": "kimi-k2.5"})
        );
        assert_eq!(round_trip(&update), update);

        let session = CreateWorkspaceSessionParams {
            workspace_id: "w-1".into(),
            create_worktree: Some(true),
        };
        assert_eq!(
            serde_json::to_value(&session).unwrap(),
            json!({"workspaceId": "w-1", "createWorktree": true})
        );
        let activate =
            ActivateSessionEntryParams { session_id: "s-1".into(), entry_id: "e-9".into() };
        assert_eq!(round_trip(&activate), activate);
        let forest = GetSessionForestParams { session_id: "s-1".into() };
        assert_eq!(round_trip(&forest), forest);

        let start = StartSessionParams {
            workspace_id: "w-1".into(),
            harness: Some(HarnessId::parse("claude").unwrap()),
            model: None,
        };
        assert_eq!(
            serde_json::to_value(&start).unwrap(),
            json!({"workspaceId": "w-1", "harness": "claude"})
        );
        let submit = SubmitInputParams { session_id: "s-1".into(), text: "steer left".into() };
        assert_eq!(
            serde_json::to_value(&submit).unwrap(),
            json!({"sessionId": "s-1", "text": "steer left"})
        );
        assert_eq!(round_trip(&submit), submit);
        let turn = SendTurnParams { session_id: "s-1".into(), text: "ship it".into() };
        assert_eq!(
            serde_json::to_value(&turn).unwrap(),
            json!({"sessionId": "s-1", "text": "ship it"})
        );
        for params in [
            serde_json::to_value(PrepareTurnParams {
                session_id: "s".into(),
                text: "t".into(),
            })
            .unwrap(),
            serde_json::to_value(StartChatParams { session_id: "s".into() }).unwrap(),
            serde_json::to_value(StopSessionParams { session_id: "s".into() }).unwrap(),
        ] {
            assert_eq!(params["sessionId"], json!("s"));
        }
    }

    #[test]
    fn submit_input_dispositions_are_a_closed_set() {
        for (disposition, wire) in [
            (InputDisposition::StartedNewTurn, "startedNewTurn"),
            (InputDisposition::SteeredActiveTurn, "steeredActiveTurn"),
            (InputDisposition::QueuedForPhaseBoundary, "queuedForPhaseBoundary"),
        ] {
            assert_eq!(serde_json::to_value(disposition).unwrap(), json!(wire));
            assert_eq!(disposition.as_str(), wire);
        }
        // A client must not be able to invent a fourth mode, and the server
        // must not be able to ship one without regenerating the contract.
        assert!(serde_json::from_value::<InputDisposition>(json!("queued")).is_err());
        assert!(serde_json::from_value::<InputDisposition>(json!("started_new_turn")).is_err());

        let queued = SubmitInputResult {
            disposition: InputDisposition::QueuedForPhaseBoundary,
            queued_input_id: Some("q-1".into()),
            interceptions: vec![SecretInterception {
                reference: "bridge-secret://1".into(),
                detector: "openai_api_key".into(),
            }],
        };
        assert_eq!(
            serde_json::to_value(&queued).unwrap()["disposition"],
            json!("queuedForPhaseBoundary")
        );
        assert_eq!(round_trip(&queued), queued);

        let steered = SubmitInputResult {
            disposition: InputDisposition::SteeredActiveTurn,
            queued_input_id: None,
            interceptions: Vec::new(),
        };
        let wire = serde_json::to_value(&steered).unwrap();
        assert_eq!(
            wire,
            json!({"disposition": "steeredActiveTurn", "interceptions": []}),
            "an absent queue id stays off the wire"
        );
        assert_eq!(round_trip(&steered), steered);
    }

    #[test]
    fn params_reject_payloads_missing_their_required_fields() {
        // A validator (or the future compat adapter) must not accept an
        // empty object where the contract names required fields.
        assert!(serde_json::from_value::<GetSessionForestParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<ActivateSessionEntryParams>(json!({"sessionId": "s"}))
                .is_err()
        );
        assert!(serde_json::from_value::<CreateChatParams>(json!({})).is_err());
        // `cursor` is a well-formed agent id — a real ACP registry entry — so
        // it parses. Whether Bridge can *run* it is an adapter-registry
        // question answered later, with an error naming the harness. Only a
        // malformed id fails here.
        assert!(serde_json::from_value::<CreateChatParams>(json!({"harness": "cursor"})).is_ok());
        assert!(
            serde_json::from_value::<CreateChatParams>(json!({"harness": "Cursor"})).is_err(),
            "malformed harness ids must be rejected"
        );
        assert!(serde_json::from_value::<CreateWorkspaceSessionParams>(json!({})).is_err());
        assert!(serde_json::from_value::<UpdateChatModelParams>(json!({"sessionId": "s"})).is_err());
        assert!(serde_json::from_value::<InterruptTurnParams>(json!({})).is_err());
        assert!(serde_json::from_value::<RetryWorkerTaskParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<RetryWorkerTaskParams>(json!({"sessionId": "s"})).is_err(),
            "a retry names the worker, not the session asking"
        );
        assert_eq!(
            serde_json::to_value(RetryWorkerTaskParams { child_session_id: "w-1".into() }).unwrap(),
            json!({"childSessionId": "w-1"})
        );
        assert!(serde_json::from_value::<StartSessionParams>(json!({})).is_err());
        assert!(serde_json::from_value::<StartChatParams>(json!({})).is_err());
        assert!(serde_json::from_value::<StopSessionParams>(json!({})).is_err());
        assert!(
            serde_json::from_value::<PrepareTurnParams>(json!({"sessionId": "s"})).is_err(),
            "text is required"
        );
        assert!(
            serde_json::from_value::<SendTurnParams>(json!({"sessionId": "s"})).is_err(),
            "text is required"
        );
        assert!(
            serde_json::from_value::<SubmitInputParams>(json!({"sessionId": "s"})).is_err(),
            "text is required"
        );
        assert!(
            serde_json::from_value::<SubmitInputParams>(
                json!({"sessionId": "s", "text": "t", "mode": "steer"})
            )
            .is_err(),
            "the disposition is the server's decision, never a client hint"
        );
        assert!(
            serde_json::from_value::<ReplaySessionEventsParams>(json!({"sessionId": "s"}))
                .is_err(),
            "afterSequence is required"
        );
        assert!(serde_json::from_value::<CompactSessionParams>(json!({"session_id": "s"})).is_err());
        assert!(
            serde_json::from_value::<SearchSessionEntriesParams>(json!({"sessionId": "s"}))
                .is_err(),
            "query is required"
        );
        assert!(
            serde_json::from_value::<SearchSessionEntriesParams>(json!({
                "sessionId": "s",
                "query": "decide",
                "workspaceId": "w"
            }))
            .is_err(),
            "recall cannot take a workspace scope"
        );
        assert!(
            serde_json::from_value::<SearchSessionEntriesParams>(json!({
                "sessionId": "s",
                "query": "decide"
            }))
            .is_ok()
        );
    }
}
