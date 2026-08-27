//! One ACP agent's stream, in Bridge's normalized event vocabulary.
//!
//! **Why this is not in `agent.rs`.** The built-in normalizers there translate
//! `serde_json::Value` frames, because Codex, OpenCode, and the Claude sidecar
//! each speak a private JSON shape that no crate models. ACP is different: the
//! protocol crate already owns typed, versioned, exhaustively documented enums,
//! and re-deriving them from `Value` would throw away the one thing adopting
//! the crate bought. So the translation lives beside the client that produces
//! it, and works on the crate's types.
//!
//! **The protocol's enums are open, so every match here is written as if a
//! newer agent were already talking.** [`SessionUpdate`] is `#[non_exhaustive]`,
//! and [`ToolKind`] and [`ToolCallStatus`] carry `#[serde(other)]` fallbacks. A
//! wire value this build has no name for becomes `provider.unknown` carrying
//! the frame, never a drop and never a panic — a dropped frame is
//! indistinguishable from a frame that never arrived, which is the same rule
//! [`crate::agent_integration::IntegrationSession::drain`] already keeps.
//!
//! **Replay is reconciled, not re-appended.** `session/load` replays a whole
//! conversation as ordinary notifications, so a client that simply forwarded
//! them would duplicate history the session forest already owns. The forest has
//! no content identity of its own, so [`AcpReplayLedger`] supplies one: a
//! fingerprint over the parts of an event that make it *that* event, computed
//! the same way on both sides of a reload.

use crate::agent::NormalizedEvent;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, PermissionOptionKind, RequestPermissionRequest, SessionUpdate,
    StopReason, ToolCall, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// The `requestMethod` marker on an ACP permission normalized to
/// `approval.requested`.
///
/// This is the real wire method, and it deliberately does not end with
/// `requestApproval`, so `live_turn.rs`'s bypass-policy gate refuses to
/// auto-grant it. That refusal is the correct behaviour rather than an
/// oversight: an ACP permission is answered with one of the option ids the
/// agent offered, and a generic auto-grant has no way to know which of
/// `allow_once`, `allow_always`, `reject_once`, or `reject_always` an agent
/// actually put on the wire. The same reasoning already keeps
/// [`crate::agent::OPENCODE_QUESTION_REQUEST_METHOD`] off that path.
pub const ACP_PERMISSION_REQUEST_METHOD: &str = "session/request_permission";

/// How a prompt turn ended.
///
/// Kept distinct rather than collapsed into one "finished", because the five
/// endings mean different things to a caller: a refusal must not be retried, a
/// token limit invites a continuation, and a cancellation is the *expected*
/// answer to `session/cancel` rather than a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpTurnOutcome {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    /// A stop reason added after this build. Reported rather than guessed at:
    /// mapping an unknown ending onto `EndTurn` would tell a caller the turn
    /// finished normally when nothing here knows that.
    Unrecognized,
}

impl AcpTurnOutcome {
    /// `StopReason` is `#[non_exhaustive]`, so the fallback is named.
    pub fn from_stop_reason(reason: StopReason) -> Self {
        match reason {
            StopReason::EndTurn => Self::EndTurn,
            StopReason::MaxTokens => Self::MaxTokens,
            StopReason::MaxTurnRequests => Self::MaxTurnRequests,
            StopReason::Refusal => Self::Refusal,
            StopReason::Cancelled => Self::Cancelled,
            _ => Self::Unrecognized,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::MaxTurnRequests => "max_turn_requests",
            Self::Refusal => "refusal",
            Self::Cancelled => "cancelled",
            Self::Unrecognized => "unrecognized",
        }
    }

    /// Whether this ending is a completion rather than a fault. A cancelled
    /// turn is a normal response to a cancel the caller asked for; only an
    /// ending nothing here recognizes is treated as suspect.
    pub const fn is_normal_completion(self) -> bool {
        !matches!(self, Self::Unrecognized)
    }
}

/// One streamed session update, normalized.
///
/// Total: a variant, tool kind, or tool status this build has no name for
/// becomes `provider.unknown` carrying the serialized update.
pub fn session_update_event(update: &SessionUpdate) -> NormalizedEvent {
    let frame = serde_json::to_value(update).unwrap_or(Value::Null);
    match update {
        SessionUpdate::UserMessageChunk(chunk) => message_chunk("user", chunk, frame),
        SessionUpdate::AgentMessageChunk(chunk) => message_chunk("assistant", chunk, frame),
        SessionUpdate::AgentThoughtChunk(chunk) => {
            let mut event = with_data("reasoning.delta", frame);
            event.item_id = chunk.message_id.as_ref().map(|id| id.0.to_string());
            event.text = content_text(&chunk.content);
            event
        }
        SessionUpdate::ToolCall(call) => tool_call_started(call, frame),
        SessionUpdate::ToolCallUpdate(update) => tool_call_progressed(update, frame),
        SessionUpdate::Plan(plan) => {
            let mut event = with_data("plan.updated", frame);
            event.title = Some(format!("{} steps", plan.entries.len()));
            event
        }
        SessionUpdate::AvailableCommandsUpdate(commands) => {
            let mut event = with_data("commands.updated", frame);
            event.title = Some(format!("{} commands", commands.available_commands.len()));
            event
        }
        SessionUpdate::CurrentModeUpdate(mode) => {
            let mut event = with_data("mode.updated", frame);
            event.text = Some(mode.current_mode_id.0.to_string());
            event
        }
        SessionUpdate::ConfigOptionUpdate(options) => {
            let mut event = with_data("config.updated", frame);
            event.title = Some(format!("{} options", options.config_options.len()));
            event
        }
        SessionUpdate::SessionInfoUpdate(_) => {
            let mut event = with_data("session.status", frame);
            event.status = Some("updated".into());
            event
        }
        SessionUpdate::UsageUpdate(usage) => {
            let mut event = with_data("usage.updated", frame);
            event.data = json!({
                "usage": {
                    "used_tokens": usage.used,
                    "context_window": usage.size,
                },
                "cost": usage.cost,
                "update": event.data,
            });
            event
        }
        _ => unknown_frame_event(frame),
    }
}

/// A permission request, normalized into Bridge's approval event.
///
/// `request_id` is Bridge's own handle on the parked responder, not anything
/// from the wire — ACP request ids belong to the protocol crate. The offered
/// options travel with the event because the answer must echo one of their ids
/// back verbatim, and a caller that never saw them could only guess.
pub fn permission_request_event(
    request_id: u64,
    request: &RequestPermissionRequest,
) -> NormalizedEvent {
    let options: Vec<Value> = request
        .options
        .iter()
        .map(|option| {
            json!({
                "id": option.option_id.0.to_string(),
                "name": option.name,
                "kind": permission_option_kind(option.kind),
            })
        })
        .collect();
    let mut event = NormalizedEvent::new("approval.requested");
    event.item_id = Some(request.tool_call.tool_call_id.0.to_string());
    event.status = Some("pending".into());
    event.title = request
        .tool_call
        .fields
        .title
        .clone()
        .or_else(|| Some("Approve tool call".into()));
    event.data = json!({
        "requestId": request_id,
        "requestMethod": ACP_PERMISSION_REQUEST_METHOD,
        "options": options,
        "toolCall": serde_json::to_value(&request.tool_call).unwrap_or(Value::Null),
    });
    event
}

/// A permission Bridge answered, so a caller can retire the pending row it
/// raised — including the ones a cancel answered on its behalf.
pub fn approval_settled_event(request_id: u64, outcome: &str) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("approval.settled");
    event.status = Some(outcome.into());
    event.data = json!({"requestId": request_id, "outcome": outcome});
    event
}

/// The end of a prompt turn.
pub fn turn_completed_event(outcome: AcpTurnOutcome) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("turn.completed");
    event.status = Some(outcome.as_str().into());
    event.data = json!({"stopReason": outcome.as_str()});
    event
}

/// The runtime stopped being able to serve the session. Carries the bounded
/// failure context, never a transcript, matching what the built-in adapters
/// already report.
pub fn runtime_failed_event(code: &str, reason: &str) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("runtime.failed");
    event.status = Some("failed".into());
    event.text = Some(reason.to_owned());
    event.data = json!({"code": code});
    event
}

/// A frame nothing recognized, reported rather than dropped. Mirrors the
/// payload shape `agent_integration.rs` already uses for the same condition.
pub fn unknown_frame_event(frame: Value) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("provider.unknown");
    event.data = json!({"frame": frame});
    event
}

/// A stable identity for one replayed event.
///
/// Covers exactly the fields that make an event *that* event: its kind, the
/// item it belongs to, who said it, its status, and its text. Deliberately not
/// the `data` blob — an agent is free to enrich a replayed payload with
/// timestamps or ids it did not have live, and a fingerprint that moved when it
/// did would re-append the whole conversation on every reload.
pub fn replay_fingerprint(event: &NormalizedEvent) -> String {
    let mut digest = Sha256::new();
    for part in [
        Some(event.kind.as_str()),
        event.item_id.as_deref(),
        event.role.as_deref(),
        event.status.as_deref(),
        event.title.as_deref(),
        event.text.as_deref(),
    ] {
        digest.update(part.unwrap_or("\u{0}").as_bytes());
        digest.update([0x1f]);
    }
    format!("{:x}", digest.finalize())
}

/// What the session forest already holds, so a reload adds only what it does
/// not.
///
/// The ledger is handed the fingerprints of the entries the forest has and
/// answers one question per replayed event: is this new? A reload of a
/// conversation the forest already holds admits nothing, which is the whole
/// point — history has one owner, and `session/load` is a re-read of it rather
/// than a second source of truth.
#[derive(Debug, Clone, Default)]
pub struct AcpReplayLedger {
    held: BTreeSet<String>,
    admitted: Vec<String>,
    suppressed: usize,
}

impl AcpReplayLedger {
    /// A ledger seeded with what the forest already holds.
    pub fn from_held(held: impl IntoIterator<Item = String>) -> Self {
        Self {
            held: held.into_iter().collect(),
            admitted: Vec::new(),
            suppressed: 0,
        }
    }

    /// Whether this replayed event is new. Repeats within one replay are
    /// suppressed too: an agent that replays the same chunk twice is still only
    /// one entry.
    pub fn admit(&mut self, event: &NormalizedEvent) -> bool {
        let fingerprint = replay_fingerprint(event);
        if !self.held.insert(fingerprint.clone()) {
            self.suppressed += 1;
            return false;
        }
        self.admitted.push(fingerprint);
        true
    }

    /// How many replayed events the forest already had.
    pub const fn suppressed(&self) -> usize {
        self.suppressed
    }

    /// Every fingerprint the ledger now holds, so a caller can persist it
    /// beside the forest and seed the next reload with it.
    pub fn held(&self) -> impl Iterator<Item = &str> {
        self.held.iter().map(String::as_str)
    }

    /// The fingerprints admitted by this ledger, in replay order.
    pub fn admitted(&self) -> impl Iterator<Item = &str> {
        self.admitted.iter().map(String::as_str)
    }
}

fn message_chunk(role: &str, chunk: &ContentChunk, frame: Value) -> NormalizedEvent {
    let mut event = with_data("message.delta", frame);
    event.role = Some(role.into());
    event.item_id = chunk.message_id.as_ref().map(|id| id.0.to_string());
    event.text = content_text(&chunk.content);
    event
}

fn tool_call_started(call: &ToolCall, frame: Value) -> NormalizedEvent {
    let (Some(kind), Some(status)) = (tool_kind_label(call.kind), tool_status_label(call.status))
    else {
        return unknown_frame_event(frame);
    };
    let mut event = with_data("tool.started", frame);
    event.item_id = Some(call.tool_call_id.0.to_string());
    event.title = Some(call.title.clone());
    event.status = Some(status.into());
    event.data = json!({"kind": kind, "update": event.data});
    event
}

fn tool_call_progressed(update: &ToolCallUpdate, frame: Value) -> NormalizedEvent {
    let status = match update.fields.status {
        Some(status) => match tool_status_label(status) {
            Some(status) => Some(status),
            None => return unknown_frame_event(frame),
        },
        None => None,
    };
    if let Some(kind) = update.fields.kind {
        if tool_kind_label(kind).is_none() {
            return unknown_frame_event(frame);
        }
    }
    let terminal = matches!(status, Some("completed") | Some("failed"));
    let mut event = with_data(
        if terminal {
            "tool.completed"
        } else {
            "tool.progress"
        },
        frame,
    );
    event.item_id = Some(update.tool_call_id.0.to_string());
    event.title = update.fields.title.clone();
    event.status = status.map(str::to_owned);
    event
}

/// The tool categories Bridge has a name for. `None` marks a category added to
/// the protocol after this build — `ToolKind::Other` is *not* that case, it is
/// the protocol's own documented catch-all and maps to a real label.
fn tool_kind_label(kind: ToolKind) -> Option<&'static str> {
    match kind {
        ToolKind::Read => Some("read"),
        ToolKind::Edit => Some("edit"),
        ToolKind::Delete => Some("delete"),
        ToolKind::Move => Some("move"),
        ToolKind::Search => Some("search"),
        ToolKind::Execute => Some("execute"),
        ToolKind::Think => Some("think"),
        ToolKind::Fetch => Some("fetch"),
        ToolKind::SwitchMode => Some("switch_mode"),
        ToolKind::Other => Some("other"),
        _ => None,
    }
}

/// Tool statuses, spelled the way the built-in adapters already spell them so
/// one consumer reads all of them.
fn tool_status_label(status: ToolCallStatus) -> Option<&'static str> {
    match status {
        ToolCallStatus::Pending => Some("pending"),
        ToolCallStatus::InProgress => Some("inProgress"),
        ToolCallStatus::Completed => Some("completed"),
        ToolCallStatus::Failed => Some("failed"),
        _ => None,
    }
}

fn permission_option_kind(kind: PermissionOptionKind) -> &'static str {
    match kind {
        PermissionOptionKind::AllowOnce => "allow_once",
        PermissionOptionKind::AllowAlways => "allow_always",
        PermissionOptionKind::RejectOnce => "reject_once",
        PermissionOptionKind::RejectAlways => "reject_always",
        _ => "unrecognized",
    }
}

fn content_text(content: &ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text(text) => Some(text.text.clone()),
        _ => None,
    }
}

fn with_data(kind: &str, data: Value) -> NormalizedEvent {
    let mut event = NormalizedEvent::new(kind);
    event.data = data;
    event
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate, PermissionOption, Plan,
        PlanEntry, PlanEntryPriority, PlanEntryStatus, TextContent, ToolCallUpdateFields,
        UsageUpdate,
    };

    fn chunk(text: &str) -> ContentChunk {
        ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
    }

    #[test]
    fn assistant_and_user_chunks_keep_their_roles() {
        let assistant = session_update_event(&SessionUpdate::AgentMessageChunk(chunk("hi")));
        assert_eq!(assistant.kind, "message.delta");
        assert_eq!(assistant.role.as_deref(), Some("assistant"));
        assert_eq!(assistant.text.as_deref(), Some("hi"));

        let user = session_update_event(&SessionUpdate::UserMessageChunk(chunk("hello")));
        assert_eq!(user.role.as_deref(), Some("user"));
    }

    #[test]
    fn a_thought_chunk_is_reasoning_not_a_message() {
        let event = session_update_event(&SessionUpdate::AgentThoughtChunk(chunk("pondering")));
        assert_eq!(event.kind, "reasoning.delta");
        assert_eq!(event.text.as_deref(), Some("pondering"));
    }

    #[test]
    fn every_tool_kind_the_protocol_names_has_a_bridge_label() {
        for kind in [
            ToolKind::Read,
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Search,
            ToolKind::Execute,
            ToolKind::Think,
            ToolKind::Fetch,
            ToolKind::SwitchMode,
            ToolKind::Other,
        ] {
            let mut call = ToolCall::new("t1", "run");
            call.kind = kind;
            let event = session_update_event(&SessionUpdate::ToolCall(call));
            assert_eq!(event.kind, "tool.started", "{kind:?}");
            assert!(
                event
                    .data
                    .pointer("/kind")
                    .and_then(Value::as_str)
                    .is_some(),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_tool_call_carries_its_id_title_and_status() {
        let mut call = ToolCall::new("call-7", "Read src/main.rs");
        call.kind = ToolKind::Read;
        call.status = ToolCallStatus::InProgress;
        let event = session_update_event(&SessionUpdate::ToolCall(call));
        assert_eq!(event.item_id.as_deref(), Some("call-7"));
        assert_eq!(event.title.as_deref(), Some("Read src/main.rs"));
        assert_eq!(event.status.as_deref(), Some("inProgress"));
        assert_eq!(
            event.data.pointer("/kind").and_then(Value::as_str),
            Some("read")
        );
    }

    #[test]
    fn tool_updates_separate_progress_from_completion() {
        let cases = [
            (ToolCallStatus::Pending, "tool.progress", "pending"),
            (ToolCallStatus::InProgress, "tool.progress", "inProgress"),
            (ToolCallStatus::Completed, "tool.completed", "completed"),
            (ToolCallStatus::Failed, "tool.completed", "failed"),
        ];
        for (status, kind, label) in cases {
            let update =
                ToolCallUpdate::new("call-1", ToolCallUpdateFields::new().status(Some(status)));
            let event = session_update_event(&SessionUpdate::ToolCallUpdate(update));
            assert_eq!(event.kind, kind, "{status:?}");
            assert_eq!(event.status.as_deref(), Some(label), "{status:?}");
            assert_eq!(event.item_id.as_deref(), Some("call-1"));
        }
    }

    #[test]
    fn a_tool_update_without_a_status_is_progress_without_one() {
        let update =
            ToolCallUpdate::new("call-1", ToolCallUpdateFields::new().title("still going"));
        let event = session_update_event(&SessionUpdate::ToolCallUpdate(update));
        assert_eq!(event.kind, "tool.progress");
        assert_eq!(event.status, None);
        assert_eq!(event.title.as_deref(), Some("still going"));
    }

    #[test]
    fn plans_modes_commands_and_usage_each_get_their_own_kind() {
        let plan = Plan::new(vec![PlanEntry::new(
            "step",
            PlanEntryPriority::Medium,
            PlanEntryStatus::Pending,
        )]);
        assert_eq!(
            session_update_event(&SessionUpdate::Plan(plan)).kind,
            "plan.updated"
        );

        let mode = CurrentModeUpdate::new("architect");
        let event = session_update_event(&SessionUpdate::CurrentModeUpdate(mode));
        assert_eq!(event.kind, "mode.updated");
        assert_eq!(event.text.as_deref(), Some("architect"));

        let commands =
            AvailableCommandsUpdate::new(vec![AvailableCommand::new("plan", "make one")]);
        assert_eq!(
            session_update_event(&SessionUpdate::AvailableCommandsUpdate(commands)).kind,
            "commands.updated"
        );

        let usage = UsageUpdate::new(120, 200_000);
        let event = session_update_event(&SessionUpdate::UsageUpdate(usage));
        assert_eq!(event.kind, "usage.updated");
        assert_eq!(
            event
                .data
                .pointer("/usage/used_tokens")
                .and_then(Value::as_u64),
            Some(120)
        );
        assert_eq!(
            event
                .data
                .pointer("/usage/context_window")
                .and_then(Value::as_u64),
            Some(200_000)
        );
    }

    #[test]
    fn a_permission_request_carries_the_offered_option_ids_verbatim() {
        let request = RequestPermissionRequest::new(
            "sess-1",
            ToolCallUpdate::new("call-3", ToolCallUpdateFields::new().title("rm -rf build")),
            vec![
                PermissionOption::new("yes-once", "Allow once", PermissionOptionKind::AllowOnce),
                PermissionOption::new("no", "Reject", PermissionOptionKind::RejectOnce),
            ],
        );
        let event = permission_request_event(42, &request);
        assert_eq!(event.kind, "approval.requested");
        assert_eq!(event.item_id.as_deref(), Some("call-3"));
        assert_eq!(event.title.as_deref(), Some("rm -rf build"));
        assert_eq!(event.status.as_deref(), Some("pending"));
        assert_eq!(
            event.data.pointer("/requestId").and_then(Value::as_u64),
            Some(42)
        );
        assert_eq!(
            event.data.pointer("/requestMethod").and_then(Value::as_str),
            Some(ACP_PERMISSION_REQUEST_METHOD)
        );
        let options = event
            .data
            .pointer("/options")
            .and_then(Value::as_array)
            .unwrap();
        let ids: Vec<&str> = options
            .iter()
            .filter_map(|option| option.get("id").and_then(Value::as_str))
            .collect();
        assert_eq!(ids, ["yes-once", "no"]);
        let names: Vec<&str> = options
            .iter()
            .filter_map(|option| option.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, ["Allow once", "Reject"]);
    }

    #[test]
    fn the_permission_marker_never_reaches_the_bypass_auto_grant_path() {
        assert!(!ACP_PERMISSION_REQUEST_METHOD.ends_with("requestApproval"));
    }

    #[test]
    fn every_stop_reason_stays_distinct() {
        let cases = [
            (StopReason::EndTurn, "end_turn"),
            (StopReason::MaxTokens, "max_tokens"),
            (StopReason::MaxTurnRequests, "max_turn_requests"),
            (StopReason::Refusal, "refusal"),
            (StopReason::Cancelled, "cancelled"),
        ];
        let mut seen = BTreeSet::new();
        for (reason, label) in cases {
            let outcome = AcpTurnOutcome::from_stop_reason(reason);
            assert_eq!(outcome.as_str(), label);
            assert!(outcome.is_normal_completion(), "{label}");
            assert!(seen.insert(outcome.as_str()), "{label} collided");
        }
        assert_eq!(seen.len(), 5);
        assert_eq!(
            turn_completed_event(AcpTurnOutcome::Cancelled)
                .status
                .as_deref(),
            Some("cancelled")
        );
    }

    #[test]
    fn an_unrecognized_ending_is_not_reported_as_a_normal_finish() {
        assert!(!AcpTurnOutcome::Unrecognized.is_normal_completion());
        assert_eq!(AcpTurnOutcome::Unrecognized.as_str(), "unrecognized");
    }

    #[test]
    fn a_reload_of_history_the_forest_holds_admits_nothing() {
        let replayed = vec![
            session_update_event(&SessionUpdate::UserMessageChunk(chunk("do the thing"))),
            session_update_event(&SessionUpdate::AgentMessageChunk(chunk("done"))),
        ];
        let mut first = AcpReplayLedger::default();
        assert!(replayed.iter().all(|event| first.admit(event)));
        assert_eq!(first.suppressed(), 0);

        let held: Vec<String> = first.held().map(str::to_owned).collect();
        let mut second = AcpReplayLedger::from_held(held);
        assert!(replayed.iter().all(|event| !second.admit(event)));
        assert_eq!(second.suppressed(), replayed.len());
        assert_eq!(second.admitted().count(), 0);
    }

    #[test]
    fn a_reload_that_grew_admits_only_the_new_tail() {
        let first_pass = vec![session_update_event(&SessionUpdate::UserMessageChunk(
            chunk("one"),
        ))];
        let mut ledger = AcpReplayLedger::default();
        assert!(ledger.admit(&first_pass[0]));

        let held: Vec<String> = ledger.held().map(str::to_owned).collect();
        let mut second = AcpReplayLedger::from_held(held);
        assert!(!second.admit(&first_pass[0]));
        let grown = session_update_event(&SessionUpdate::AgentMessageChunk(chunk("two")));
        assert!(second.admit(&grown));
        assert_eq!(second.admitted().count(), 1);
    }

    #[test]
    fn a_repeated_chunk_inside_one_replay_is_admitted_once() {
        let event = session_update_event(&SessionUpdate::AgentMessageChunk(chunk("echo")));
        let mut ledger = AcpReplayLedger::default();
        assert!(ledger.admit(&event));
        assert!(!ledger.admit(&event));
        assert_eq!(ledger.suppressed(), 1);
    }

    #[test]
    fn fingerprints_separate_events_that_differ_only_in_one_field() {
        let base = session_update_event(&SessionUpdate::AgentMessageChunk(chunk("same")));
        let mut other_role = base.clone();
        other_role.role = Some("user".into());
        let mut other_text = base.clone();
        other_text.text = Some("different".into());
        let mut other_item = base.clone();
        other_item.item_id = Some("m9".into());

        let fingerprints: BTreeSet<String> = [&base, &other_role, &other_text, &other_item]
            .into_iter()
            .map(replay_fingerprint)
            .collect();
        assert_eq!(fingerprints.len(), 4);
    }

    #[test]
    fn a_fingerprint_ignores_data_an_agent_may_enrich_on_replay() {
        let mut live = session_update_event(&SessionUpdate::AgentMessageChunk(chunk("stable")));
        let replayed = live.clone();
        live.data = json!({"timestamp": "2026-08-28T00:00:00Z"});
        assert_eq!(replay_fingerprint(&live), replay_fingerprint(&replayed));
    }

    #[test]
    fn an_unknown_frame_keeps_the_payload_shape_the_rest_of_bridge_reads() {
        let event = unknown_frame_event(json!({"sessionUpdate": "from_the_future"}));
        assert_eq!(event.kind, "provider.unknown");
        assert_eq!(
            event
                .data
                .pointer("/frame/sessionUpdate")
                .and_then(Value::as_str),
            Some("from_the_future")
        );
    }
}
