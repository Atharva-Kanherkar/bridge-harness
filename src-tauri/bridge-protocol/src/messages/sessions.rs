//! The sessions domain: the session forest, live turns, and durable replay.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::common::{HarnessId, JsSafeU64};
use super::state::BridgeState;

pub const DEFAULT_REPLAY_EVENT_LIMIT: u32 = 500;
pub const MAX_REPLAY_EVENT_LIMIT: u32 = 1_000;

/// Hard cap on the segments returned by `sessions/get_context_breakdown`.
/// Ordering is deterministic, so truncation is stable across recomputes.
pub const MAX_CONTEXT_BREAKDOWN_SEGMENTS: u32 = 64;

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetContextBreakdownParams {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GetContextBreakdownDigestParams {
    pub session_id: String,
}

/// `sessions/get_context_breakdown_digest`'s result: an opaque change token
/// for one session's context breakdown. Equal digests mean the breakdown
/// would be unchanged; the token covers prompt compilations, prompt-section
/// revisions, adapter context observations, and active-branch changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownDigestResult {
    pub digest: String,
}

/// Where a breakdown segment's numbers come from. Every segment names its
/// source so clients never have to guess attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ContextBreakdownOrigin {
    Conversation,
    PromptCompilation,
    AdapterInventory,
}

/// Availability of one segment, mirroring the adapter inventory provenance
/// vocabulary. `unavailable` segments carry a reason and contribute nothing
/// to totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ContextBreakdownState {
    Reported,
    Measured,
    Estimated,
    Unavailable,
}

/// One bounded, source-labelled slice of a session's context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownSegment {
    pub origin: ContextBreakdownOrigin,
    pub segment_class: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    pub state: ContextBreakdownState,
    /// Estimation method when `state` is `estimated`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Why nothing could be observed when `state` is `unavailable`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<i64>,
    /// True when a value was clamped to the observation bounds upstream.
    pub capped: bool,
}

/// Sums over available segments only. A unit is `None` when no available
/// segment observed it — absence is preserved instead of zero-filling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownTotals {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<i64>,
    /// How many returned segments are `unavailable` — segments, not distinct
    /// sources: one silent source contributes one entry per class it covers.
    pub unavailable_sources: u32,
}

/// The projected conversation state behind the breakdown, computed from
/// `SessionForest::active_branch` followed by `ContextProjector`. This is
/// compacted context, not the uncompacted active token estimate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownConversation {
    pub entry_count: u32,
    pub rendered_entry_count: u32,
    pub token_estimate: i64,
    pub context_pressure: i64,
    pub context_window_tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restoration_boundary_entry_id: Option<String>,
}

/// Change since the previous valid compaction snapshot on the active branch.
/// `null` on the result when no valid compaction exists yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownDelta {
    pub boundary_entry_id: String,
    pub first_retained_entry_id: String,
    pub source_agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub tokens_before: i64,
    pub current_token_estimate: i64,
    pub growth_tokens: i64,
}

/// `sessions/get_context_breakdown`'s result: a capped, stably ordered
/// breakdown merging conversation projection, Bridge prompt accounting, and
/// live adapter context observations. Every segment preserves its source and
/// availability state; nothing is fabricated when a source cannot report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ContextBreakdownResult {
    pub session_id: String,
    pub segments: Vec<ContextBreakdownSegment>,
    pub totals: ContextBreakdownTotals,
    pub conversation: ContextBreakdownConversation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction_delta: Option<ContextBreakdownDelta>,
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

/// Create a direct chat with an exact identity result for concurrent clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateChatIdParams {
    pub harness: HarnessId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The identity committed by this creation, unaffected by concurrent clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateChatIdResult {
    pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateAsideChatParams {
    pub source_session_id: String,
    pub harness: HarnessId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateAsideChatResult {
    pub state: BridgeState,
    pub source_session_id: String,
    pub session_id: String,
    pub handoff_status: String,
    pub fidelity: String,
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
    /// Provider-specific thinking level, validated against the selected model.
    /// Omitted preserves the current value when the target model supports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CarrySessionHandoffParams {
    /// The chat receiving the projected context (usually a brand-new one).
    pub target_session_id: String,
    /// The chat whose stored history is projected into the brief.
    pub source_session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CarrySessionHandoffResult {
    /// Whether a handoff brief was appended; false means there was nothing to
    /// carry (empty or unknown sessions) and the target starts as usual.
    pub carried: bool,
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
    /// Return the newest `limit` durable events, still ordered oldest to
    /// newest. Intended for bounded activity surfaces, not cursor recovery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tail: Option<bool>,
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

/// One image a user attached to a submitted turn.
///
/// Images travel as base64 in-band rather than by path: the clipboard source
/// may never have touched the filesystem, and the wire stays provider-neutral
/// — each adapter decides what the pair becomes (Anthropic image content
/// blocks for Claude). Bytes are transport payload, so secret interception and
/// `@file` context never see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnImage {
    /// RFC 2046 media type, e.g. `image/png`.
    pub media_type: String,
    /// Raw base64 of the encoded image, without the data-URI prefix.
    pub base64_data: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitInputParams {
    pub session_id: String,
    pub text: String,
    /// Image attachments pasted or otherwise added in the composer. Absent
    /// (or `None`) means none: older clients omit it and stay on plain text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<TurnImage>>,
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

/// `sessions/dispatch_agent_shortcut` accepts only identity and user intent.
/// Role, model, effort, write scope, and policy are deliberately absent: the
/// host re-resolves all of them from persisted configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DispatchAgentShortcutParams {
    pub session_id: String,
    pub token: String,
    pub objective: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum AgentShortcutDisposition {
    Launched,
    Queued,
    AwaitingApproval,
}

/// The reservation outcome the composer can render without asking an
/// orchestrator to interpret a worker lifecycle event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DispatchAgentShortcutResult {
    pub disposition: AgentShortcutDisposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    pub agent_id: String,
    pub agent_name: String,
    pub role: String,
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

/// Which part of the forest `sessions/export_session_transcript` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptExportScope {
    /// Only the entries on the head's active branch — the conversation as it
    /// currently reads.
    ActiveBranch,
    /// Every entry of the session, abandoned branches included — what actually
    /// happened rather than what is currently shown.
    Forest,
}

impl Default for TranscriptExportScope {
    fn default() -> Self {
        Self::Forest
    }
}

/// Write one session's durable record out as newline-delimited JSON.
///
/// The export is a file, never an inline string: a long session is megabytes,
/// and a result that large belongs on disk rather than in a JSON-RPC frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportSessionTranscriptParams {
    pub session_id: String,
    /// Omitted requests export the whole forest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<TranscriptExportScope>,
    /// Whether to include the hidden control entries — turn boundaries, usage
    /// and plan updates. Omitted requests include them: they are the part of
    /// the record the rendered transcript cannot show.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_hidden: Option<bool>,
    /// An absolute path to write. Omitted requests land under the data
    /// directory's `exports/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination_path: Option<String>,
}

/// Mirrors `bridge_core::transcript_export::TranscriptExport` — where the file
/// landed and enough about it to verify the write without reopening it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionTranscriptResult {
    pub session_id: String,
    pub path: String,
    pub scope: TranscriptExportScope,
    pub schema_version: JsSafeU64,
    pub line_count: JsSafeU64,
    pub entry_count: JsSafeU64,
    pub bytes: JsSafeU64,
    /// `sha256:<hex>` over the entry lines only, so the digest is stable
    /// against the export timestamp in the header.
    pub digest: String,
    pub exported_at: String,
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
    fn context_breakdown_payloads_round_trip_and_reject_unknown_fields() {
        let params = GetContextBreakdownParams { session_id: "s-1".into() };
        assert_eq!(
            serde_json::to_value(&params).unwrap(),
            json!({"sessionId": "s-1"})
        );
        assert_eq!(round_trip(&params), params);
        assert!(serde_json::from_value::<GetContextBreakdownParams>(json!({
            "sessionId": "s-1",
            "session_id": "s-1"
        }))
        .is_err());

        let digest_params = GetContextBreakdownDigestParams { session_id: "s-1".into() };
        assert_eq!(round_trip(&digest_params), digest_params);

        let segment = ContextBreakdownSegment {
            origin: ContextBreakdownOrigin::AdapterInventory,
            segment_class: "toolSchemas".into(),
            names: vec!["shell".into()],
            state: ContextBreakdownState::Estimated,
            method: Some("catalog".into()),
            reason: None,
            item_count: Some(12),
            bytes: None,
            tokens: Some(340),
            capped: false,
        };
        let wire = serde_json::to_value(&segment).unwrap();
        assert_eq!(wire["state"], "estimated");
        assert_eq!(wire["origin"], "adapterInventory");
        assert!(wire.get("bytes").is_none(), "absent units stay off the wire");
        assert_eq!(round_trip(&segment), segment);

        let unavailable = ContextBreakdownSegment {
            names: Vec::new(),
            state: ContextBreakdownState::Unavailable,
            reason: Some("no prompt compilation recorded".into()),
            method: None,
            ..segment.clone()
        };
        assert_eq!(
            serde_json::to_value(&unavailable).unwrap()["reason"],
            "no prompt compilation recorded"
        );

        let result = ContextBreakdownResult {
            session_id: "s-1".into(),
            segments: vec![segment],
            totals: ContextBreakdownTotals {
                item_count: Some(12),
                bytes: None,
                tokens: Some(340),
                unavailable_sources: 1,
            },
            conversation: ContextBreakdownConversation {
                entry_count: 7,
                rendered_entry_count: 5,
                token_estimate: 900,
                context_pressure: 3,
                context_window_tokens: 128_000,
                model: Some("stub-standard".into()),
                effort: None,
                restoration_boundary_entry_id: None,
            },
            compaction_delta: Some(ContextBreakdownDelta {
                boundary_entry_id: "b-1".into(),
                first_retained_entry_id: "r-1".into(),
                source_agent: "orchestrator".into(),
                reason: Some("manual".into()),
                tokens_before: 500,
                current_token_estimate: 900,
                growth_tokens: 400,
            }),
            digest: "v1:test".into(),
        };
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["compactionDelta"]["tokensBefore"], 500);
        assert_eq!(wire["conversation"]["contextWindowTokens"], 128_000);
        assert_eq!(round_trip(&result), result);
    }

    #[test]
    fn context_breakdown_segment_cap_constant_is_stable() {
        assert_eq!(MAX_CONTEXT_BREAKDOWN_SEGMENTS, 64);
    }

    #[test]
    fn session_params_round_trip_and_omit_absent_options() {
        let create = CreateChatParams { harness: HarnessId::parse("codex").unwrap(), model: None, title: None };
        let wire = serde_json::to_value(&create).unwrap();
        assert_eq!(wire, json!({"harness": "codex"}), "absent options stay off the wire");
        assert_eq!(round_trip(&create), create);

        for level in ["max", "ultra"] {
            let value = json!({"sessionId":"s-1", "harness":"codex", "model":"live-model", "effort":level});
            let params: UpdateChatModelParams = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(params).unwrap(), value);
        }
        let update = UpdateChatModelParams {
            session_id: "s-1".into(),
            harness: HarnessId::parse("opencode").unwrap(),
            model: Some("kimi-k2.5".into()),
            effort: Some("high".into()),
        };
        let wire = serde_json::to_value(&update).unwrap();
        assert_eq!(
            wire,
            json!({"sessionId": "s-1", "harness": "opencode", "model": "kimi-k2.5", "effort": "high"})
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
        let submit = SubmitInputParams {
            session_id: "s-1".into(),
            text: "steer left".into(),
            attachments: None,
        };
        assert_eq!(
            serde_json::to_value(&submit).unwrap(),
            json!({"sessionId": "s-1", "text": "steer left"}),
            "empty attachments stay off the wire"
        );
        assert_eq!(round_trip(&submit), submit);
        let with_image = SubmitInputParams {
            session_id: "s-1".into(),
            text: "what is this?".into(),
            attachments: Some(vec![TurnImage { media_type: "image/png".into(), base64_data: "iVBORw0".into() }]),
        };
        assert_eq!(
            serde_json::to_value(&with_image).unwrap(),
            json!({
                "sessionId": "s-1",
                "text": "what is this?",
                "attachments": [{"mediaType": "image/png", "base64Data": "iVBORw0"}]
            })
        );
        assert_eq!(round_trip(&with_image), with_image);
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
        // Attachment-less submits stay valid without the new field, and an
        // explicit empty list is interchangeable with absence — both mean
        // plain text.
        let no_attachments =
            serde_json::from_value::<SubmitInputParams>(json!({"sessionId": "s", "text": "hi"}))
                .expect("older clients omit attachments");
        assert!(no_attachments.attachments.is_none());
        let empty_attachments = serde_json::from_value::<SubmitInputParams>(json!(
            {"sessionId": "s", "text": "hi", "attachments": []}
        ))
        .expect("an explicit empty list is accepted");
        assert!(empty_attachments.attachments.unwrap().is_empty());
        assert!(
            serde_json::from_value::<SubmitInputParams>(
                json!({"sessionId": "s", "text": "hi", "attachments": [{"mediaType": "image/png"}]})
            )
            .is_err(),
            "a half-formed attachment is a contract violation, not a silent drop"
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
