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
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// The exact wire method carried by an ACP permission interaction.
pub const ACP_PERMISSION_REQUEST_METHOD: &str = "session/request_permission";

/// How much of one thought run is kept. A thought is persisted and replayed
/// into a model's context like any other entry, so it is capped for the same
/// reason the assembled reply is.
const THOUGHT_BYTES: usize = 1024 * 1024;

/// What a capped thought ends with, so a truncated one says so.
const THOUGHT_TRUNCATED: &str = "\n[truncated]";

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
            // ACP's usage is a context gauge, not an input/output split, so the
            // payload carries the aliases the usage ledger actually reads:
            // context_percent from the gauge, and the cumulative cost only when
            // it is denominated in the currency the ledger assumes. The raw
            // fields travel beside them for anything that wants the gauge.
            let context_percent = (usage.size > 0)
                .then(|| (usage.used.saturating_mul(100) / usage.size) as i64);
            let total_cost_usd = usage
                .cost
                .as_ref()
                .filter(|cost| cost.currency.eq_ignore_ascii_case("USD"))
                .map(|cost| cost.amount);
            event.data = json!({
                "usage": {
                    "used_tokens": usage.used,
                    "context_window": usage.size,
                },
                "context_percent": context_percent,
                "total_cost_usd": total_cost_usd,
                "cost": usage.cost,
                "update": event.data,
            });
            event
        }
        _ => unknown_frame_event(frame),
    }
}

/// One run of `agent_thought_chunk` updates, held so that it can be closed.
///
/// **ACP has no "thought completed" frame.** A thought arrives as chunks and
/// simply stops when the agent moves on to something else. Every other
/// normalizer Bridge has emits a terminal reasoning event — Codex on
/// `item/completed`, OpenCode when the part carries `time.end`, the Claude
/// sidecar directly — and the transcript is built on that: a thought card
/// shimmers until its completion settles it, and the session forest refuses to
/// persist a kind ending in `.delta`. So an ACP thought used to shimmer until
/// the whole turn ended, and a reloaded Cursor conversation had no thoughts in
/// it at all, however many the live window had shown.
///
/// The run is closed here instead, at the boundary where the agent moves on:
/// the first non-thought update after it, or the end of the turn. What is
/// published is a `reasoning.completed` carrying the accumulated text under the
/// same item id the run's deltas carried, which is what makes the streamed card
/// and its persisted twin one item rather than two.
#[derive(Debug, Default)]
pub struct AcpThoughtRun {
    text: String,
    /// The id every delta in this run carries: the agent's `messageId` when it
    /// sent one, otherwise the one minted here.
    item_id: Option<String>,
    /// Whether `item_id` came from the agent. Only an agent-named run can be
    /// resumed — a minted id belongs to the run that opened it and to nothing
    /// else.
    named: bool,
    truncated: bool,
    /// The run this accumulator last closed, kept for the length of the turn.
    /// An agent is free to break off mid-thought, call a tool, and carry on
    /// under the same `messageId`; ACP says that is one message, so the second
    /// completion has to carry the whole thought rather than replace the first
    /// one's text with its tail.
    resumable: Option<(String, String)>,
}

/// Whether an event of this kind means the agent moved on from what it was
/// thinking, so an open thought run should close.
///
/// ACP interleaves non-conversational notifications into the middle of a
/// turn wherever the agent's runtime happens to flush them: a `usage_update`,
/// an `available_commands_update`, a mode or config change, a session
/// lifecycle frame. None of those are the agent moving on — they are
/// bookkeeping that arrives beside whatever it is actually doing. Treating
/// every one of them as "moved on" closed a thought run early: a `usage_update`
/// landing between two chunks of one thought split it in two, so an
/// agent-named run had to be stitched back together into one card with two
/// completions, and an unnamed run became two cards, the first a fragment.
/// Only a conversational move — prose, a tool call, a plan update, an
/// approval or permission exchange, an error, or the turn itself ending —
/// actually ends what the agent was thinking, so only those kinds close the
/// run here. Anything else passes through untouched and the run stays open.
fn is_conversational(kind: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "message.",
        "tool.",
        "command.",
        "file_change.",
        "diff.",
        "approval.",
        "permission.",
        "question.",
        "plan.",
        "turn.",
        "runtime.",
    ];
    kind == "error" || PREFIXES.iter().any(|prefix| kind.starts_with(prefix))
}

impl AcpThoughtRun {
    /// Fold one normalized event into the run, returning the completion this
    /// event ended, if any.
    ///
    /// Takes the event by `&mut` for one reason: a chunk the agent did not name
    /// leaves with the run's minted id stamped on it, so the deltas and the
    /// completion agree on which thought they are.
    pub fn absorb(&mut self, event: &mut NormalizedEvent) -> Option<NormalizedEvent> {
        if event.kind != "reasoning.delta" {
            if !is_conversational(&event.kind) {
                // Usage, commands, mode, config, session lifecycle, and
                // unrecognized frames are not the agent moving on — leave the
                // run open and let the frame pass through untouched.
                return None;
            }
            // A conversational move is the agent moving on. A run that is
            // already closed stays closed: `close` is a no-op with nothing
            // open.
            return self.close();
        }
        let incoming = event.item_id.clone();
        let ended = (!self.continues(incoming.as_deref()))
            .then(|| self.close())
            .flatten();
        if self.item_id.is_none() {
            self.open(incoming.as_deref());
        }
        if event.item_id.is_none() {
            event.item_id.clone_from(&self.item_id);
        }
        if let Some(text) = event.text.as_deref() {
            self.push(text);
        }
        ended
    }

    /// The completion for whatever is open, and nothing when nothing is.
    pub fn close(&mut self) -> Option<NormalizedEvent> {
        let item_id = self.item_id.take()?;
        let text = std::mem::take(&mut self.text);
        let truncated = std::mem::replace(&mut self.truncated, false);
        let named = std::mem::replace(&mut self.named, false);
        if text.trim().is_empty() {
            return None;
        }
        // Only an agent-named run is worth keeping: a minted id can never be
        // matched by a later chunk, so holding its text would only strand it.
        self.resumable = named.then(|| (item_id.clone(), text.clone()));
        let mut event = NormalizedEvent::new("reasoning.completed");
        event.item_id = Some(item_id);
        event.status = Some("completed".into());
        event.text = Some(if truncated {
            format!("{text}{THOUGHT_TRUNCATED}")
        } else {
            text
        });
        event.data = json!({"assembledFrom": "reasoning.delta"});
        Some(event)
    }

    /// Arm the accumulator for a turn about to start. Deltas can arrive with no
    /// prompt outstanding — this module accepts what an agent flushes after a
    /// cancel — so a turn boundary clears the run rather than only draining it.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Whether a chunk belongs to the run in flight. ACP states that a change of
    /// `messageId` starts a new message, so a chunk carrying a different one
    /// ends this run instead of being concatenated onto it. An unnamed chunk
    /// continues whatever is open, because the only id it could carry is the one
    /// minted here.
    fn continues(&self, item_id: Option<&str>) -> bool {
        match item_id {
            None => true,
            Some(incoming) => self.item_id.as_deref() == Some(incoming),
        }
    }

    fn open(&mut self, item_id: Option<&str>) {
        if let Some(id) = item_id {
            if let Some((resumed, text)) = self
                .resumable
                .take()
                .filter(|(resumed, _)| resumed.as_str() == id)
            {
                self.item_id = Some(resumed);
                self.text = text;
                self.named = true;
                return;
            }
        }
        self.resumable = None;
        self.named = item_id.is_some();
        self.item_id = Some(
            item_id
                .map(str::to_owned)
                .unwrap_or_else(|| format!("acp-thought-{}", Uuid::new_v4())),
        );
    }

    /// Append one chunk, stopping at the cap rather than growing past it. The
    /// cut lands on a character boundary, because the text is persisted and
    /// replayed to a model rather than only shown.
    fn push(&mut self, text: &str) {
        let room = THOUGHT_BYTES.saturating_sub(self.text.len());
        if text.len() <= room {
            self.text.push_str(text);
            return;
        }
        let mut cut = room;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        self.text.push_str(&text[..cut]);
        self.truncated = true;
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
    let actions: Vec<Value> = options
        .iter()
        .filter_map(|option| {
            let option_id = option.get("id")?.as_str()?;
            let kind = option.get("kind")?.as_str()?;
            let (decision, fallback_label) = match kind {
                "allow_always" => ("acceptForSession", "Allow for session"),
                "allow_once" => ("accept", "Allow once"),
                "reject_once" => ("decline", "Decline once"),
                "reject_always" => ("decline", "Always decline"),
                _ => return None,
            };
            Some(json!({
                "id": option_id,
                "optionId": option_id,
                "decision": decision,
                "label": option.get("name").and_then(Value::as_str).unwrap_or(fallback_label),
            }))
        })
        .collect();
    let mut event = NormalizedEvent::new("permission.requested");
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
        "interactionKind": "permission",
        "actions": actions,
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
///
/// Kind `error` with status `failed`, because that is the one shape the
/// session supervisor's failure arm recognizes — the spelling every built-in
/// adapter normalizes provider death to. A private kind here would be
/// persisted as an ordinary transcript entry while the session stayed
/// `working` forever.
pub fn runtime_failed_event(code: &str, reason: &str) -> NormalizedEvent {
    let mut event = NormalizedEvent::new("error");
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
    /// How many times each content fingerprint has occurred in the replay
    /// currently being reconciled. Cleared by [`Self::begin_replay`].
    occurrences: BTreeMap<String, usize>,
}

impl AcpReplayLedger {
    /// A ledger seeded with what the forest already holds.
    pub fn from_held(held: impl IntoIterator<Item = String>) -> Self {
        Self {
            held: held.into_iter().collect(),
            admitted: Vec::new(),
            suppressed: 0,
            occurrences: BTreeMap::new(),
        }
    }

    /// Start reconciling one replay. Occurrence counting is per replay: the
    /// second "yes" in a conversation is the second occurrence of that content
    /// in *this* replay, matched against the second occurrence the forest
    /// already holds.
    pub fn begin_replay(&mut self) {
        self.occurrences.clear();
    }

    /// Whether this replayed event is new.
    ///
    /// Identity is content plus occurrence: two genuinely distinct events with
    /// identical content — a user who typed "yes" twice, repeated content-only
    /// tool progress — are the first and second occurrence of one fingerprint,
    /// so a reload of history the forest holds still admits nothing while a
    /// conversation that really said the same thing twice keeps both.
    pub fn admit(&mut self, event: &NormalizedEvent) -> bool {
        let base = replay_fingerprint(event);
        let occurrence = self.occurrences.entry(base.clone()).or_insert(0);
        *occurrence += 1;
        let fingerprint = if *occurrence == 1 {
            base
        } else {
            format!("{base}#{occurrence}")
        };
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
        AvailableCommand, AvailableCommandsUpdate, CurrentModeUpdate, MessageId, PermissionOption,
        Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus, TextContent, ToolCallUpdateFields,
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

    /// The stream as [`crate::acp_session`]'s `publish_event` drives it: every
    /// normalized event goes through the run, and whatever the run closes is
    /// published just before it.
    fn drive(run: &mut AcpThoughtRun, updates: &[SessionUpdate]) -> Vec<NormalizedEvent> {
        let mut published = Vec::new();
        for update in updates {
            let mut event = session_update_event(update);
            if let Some(ended) = run.absorb(&mut event) {
                published.push(ended);
            }
            published.push(event);
        }
        published
    }

    fn thought(text: &str) -> SessionUpdate {
        SessionUpdate::AgentThoughtChunk(chunk(text))
    }

    fn named_thought(text: &str, message_id: &str) -> SessionUpdate {
        let mut content = chunk(text);
        content.message_id = Some(MessageId::new(message_id));
        SessionUpdate::AgentThoughtChunk(content)
    }

    fn tool(id: &'static str) -> SessionUpdate {
        SessionUpdate::ToolCall(ToolCall::new(id, "bun test"))
    }

    fn completions(published: &[NormalizedEvent]) -> Vec<&NormalizedEvent> {
        published
            .iter()
            .filter(|event| event.kind == "reasoning.completed")
            .collect()
    }

    #[test]
    fn a_thought_run_closes_before_the_tool_call_that_ended_it() {
        let mut run = AcpThoughtRun::default();
        let published = drive(
            &mut run,
            &[thought("Start with "), thought("the suite."), tool("call-1")],
        );
        let kinds: Vec<&str> = published.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "reasoning.delta",
                "reasoning.delta",
                "reasoning.completed",
                "tool.started"
            ],
            "the completion belongs before the call that ended the thought"
        );
        let completed = &published[2];
        assert_eq!(completed.text.as_deref(), Some("Start with the suite."));
        assert_eq!(completed.status.as_deref(), Some("completed"));
        // The id the deltas carried, so the streamed card and the persisted one
        // are the same item rather than two.
        assert_eq!(completed.item_id, published[0].item_id);
        assert_eq!(completed.item_id, published[1].item_id);
    }

    #[test]
    fn a_thought_run_open_at_the_end_of_the_turn_still_closes() {
        let mut run = AcpThoughtRun::default();
        let published = drive(&mut run, &[thought("Still weighing it.")]);
        assert_eq!(published.len(), 1, "nothing has ended the run yet");
        let completed = run.close().expect("the open run closes at turn end");
        assert_eq!(completed.kind, "reasoning.completed");
        assert_eq!(completed.text.as_deref(), Some("Still weighing it."));
        assert_eq!(completed.item_id, published[0].item_id);
        assert!(run.close().is_none(), "a closed run closes once");
    }

    #[test]
    fn two_thought_runs_get_two_ids() {
        let mut run = AcpThoughtRun::default();
        let published = drive(
            &mut run,
            &[
                thought("Start with the suite."),
                tool("call-1"),
                thought("Read the file it points at."),
                tool("call-2"),
            ],
        );
        let closed = completions(&published);
        assert_eq!(closed.len(), 2);
        assert_eq!(closed[0].text.as_deref(), Some("Start with the suite."));
        assert_eq!(
            closed[1].text.as_deref(),
            Some("Read the file it points at."),
            "the second run carries only its own text"
        );
        assert_ne!(
            closed[0].item_id, closed[1].item_id,
            "two thoughts are two cards"
        );
        assert!(closed.iter().all(|event| event.item_id.is_some()));
    }

    #[test]
    fn an_unnamed_chunk_leaves_with_the_runs_minted_id() {
        let mut run = AcpThoughtRun::default();
        let published = drive(&mut run, &[thought("one "), thought("two")]);
        let minted = published[0].item_id.clone().expect("a run is always named");
        assert!(minted.starts_with("acp-thought-"));
        assert_eq!(published[1].item_id.as_deref(), Some(minted.as_str()));
    }

    #[test]
    fn an_agent_named_thought_keeps_the_agents_id() {
        let mut run = AcpThoughtRun::default();
        let published = drive(&mut run, &[named_thought("hm", "thought-1"), tool("call-1")]);
        assert_eq!(published[0].item_id.as_deref(), Some("thought-1"));
        assert_eq!(published[1].item_id.as_deref(), Some("thought-1"));
    }

    #[test]
    fn a_thought_resumed_under_the_same_message_id_finishes_as_one_thought() {
        // ACP: chunks sharing a `messageId` are one message. An agent that
        // breaks off to call a tool and carries on is still on that message, so
        // the second completion has to carry the whole thought — otherwise the
        // durable card's text is replaced by its own tail.
        let mut run = AcpThoughtRun::default();
        let published = drive(
            &mut run,
            &[
                named_thought("Start with the suite. ", "thought-1"),
                tool("call-1"),
                named_thought("That failed, so read the file.", "thought-1"),
                tool("call-2"),
            ],
        );
        let closed = completions(&published);
        assert_eq!(closed.len(), 2);
        assert_eq!(
            closed[1].text.as_deref(),
            Some("Start with the suite. That failed, so read the file.")
        );
        assert_eq!(closed[1].item_id.as_deref(), Some("thought-1"));
    }

    #[test]
    fn a_new_message_id_starts_a_new_thought_without_a_gap() {
        let mut run = AcpThoughtRun::default();
        let published = drive(
            &mut run,
            &[named_thought("first", "a"), named_thought("second", "b")],
        );
        let kinds: Vec<&str> = published.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(
            kinds,
            ["reasoning.delta", "reasoning.completed", "reasoning.delta"],
            "the id change ends the first thought where it actually ended"
        );
        assert_eq!(published[1].item_id.as_deref(), Some("a"));
        assert_eq!(published[1].text.as_deref(), Some("first"));
    }

    #[test]
    fn a_run_that_streamed_nothing_publishes_nothing() {
        let mut run = AcpThoughtRun::default();
        let published = drive(&mut run, &[thought(""), tool("call-1")]);
        assert_eq!(completions(&published).len(), 0);
    }

    #[test]
    fn a_turn_boundary_clears_a_run_left_over_from_a_cancel() {
        let mut run = AcpThoughtRun::default();
        drive(&mut run, &[thought("abandoned")]);
        run.reset();
        assert!(run.close().is_none());
    }

    #[test]
    fn a_usage_update_between_chunks_does_not_split_the_thought() {
        // A `usage_update` is bookkeeping the agent's runtime flushes wherever
        // it happens to land in the stream, not the agent moving on from what
        // it was thinking. Closing the run on it used to split one thought
        // into two cards; the fix leaves the run open and lets the frame pass
        // through.
        let mut run = AcpThoughtRun::default();
        let published = drive(
            &mut run,
            &[
                thought("Start with "),
                SessionUpdate::UsageUpdate(UsageUpdate::new(120, 200_000)),
                thought("the suite."),
                tool("call-1"),
            ],
        );
        let closed = completions(&published);
        assert_eq!(
            closed.len(),
            1,
            "a usage update must not end the thought it interrupts"
        );
        assert_eq!(closed[0].text.as_deref(), Some("Start with the suite."));
        let kinds: Vec<&str> = published.iter().map(|event| event.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "reasoning.delta",
                "usage.updated",
                "reasoning.delta",
                "reasoning.completed",
                "tool.started",
            ],
            "the usage update passes through untouched, and the completion \
             still lands just before the tool call that actually ended it"
        );
    }

    #[test]
    fn an_available_commands_update_between_chunks_does_not_split_an_unnamed_thought() {
        let mut run = AcpThoughtRun::default();
        let commands = SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(
            vec![AvailableCommand::new("plan", "make one")],
        ));
        let published = drive(
            &mut run,
            &[
                thought("Weighing "),
                commands,
                thought("the options."),
                tool("call-1"),
            ],
        );
        let closed = completions(&published);
        assert_eq!(
            closed.len(),
            1,
            "an available-commands update must not end the thought it interrupts"
        );
        assert_eq!(closed[0].text.as_deref(), Some("Weighing the options."));
        assert_eq!(
            closed[0].item_id, published[0].item_id,
            "one minted id for the one thought, not two"
        );
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
        assert_eq!(event.kind, "permission.requested");
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
        let actions = event
            .data
            .pointer("/actions")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(actions[0]["optionId"], "yes-once");
        assert_eq!(actions[0]["decision"], "accept");
        assert_eq!(actions[1]["optionId"], "no");
        assert_eq!(actions[1]["decision"], "decline");
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
    fn identical_content_twice_in_one_conversation_is_two_events_and_reloads_as_two() {
        // A user who says "echo" twice said it twice: dropping the second copy
        // silently rewrites history. But a *reload* of that same conversation
        // must still admit nothing — occurrence counting keeps both promises.
        let event = session_update_event(&SessionUpdate::AgentMessageChunk(chunk("echo")));
        let mut ledger = AcpReplayLedger::default();
        ledger.begin_replay();
        assert!(ledger.admit(&event));
        assert!(ledger.admit(&event), "a second occurrence is a second event");
        assert_eq!(ledger.suppressed(), 0);

        let held: Vec<String> = ledger.held().map(str::to_owned).collect();
        let mut reload = AcpReplayLedger::from_held(held);
        reload.begin_replay();
        assert!(!reload.admit(&event));
        assert!(!reload.admit(&event), "the reloaded second occurrence is already held");
        assert_eq!(reload.suppressed(), 2);
        assert!(
            reload.admit(&event),
            "a third occurrence the forest does not hold is genuinely new"
        );
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
